//! Claude's local catalog policy, separate from filesystem discovery and usage.
//! These facts describe the inspected roots, not an active session or permissions.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::harness::AgentKind;
use crate::scan::ScanResult;

const MAX_SETTINGS_BYTES: u64 = 256 * 1024;
const VERSION: &str = "claude-local-catalog-v1";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelInvocation {
    #[default]
    Automatic,
    ExplicitOnly,
    Unknown,
}

impl ModelInvocation {
    pub(crate) fn from_disable_scalar(value: &str) -> Self {
        match value
            .split(" #")
            .next()
            .unwrap_or(value)
            .trim()
            .trim_matches(['\'', '"'])
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "true" | "yes" | "on" | "1" => Self::ExplicitOnly,
            "false" | "no" | "off" | "0" => Self::Automatic,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogVisibility {
    On,
    NameOnly,
    UserInvocableOnly,
    Off,
    NotDiscovered,
    Unknown,
}

impl CatalogVisibility {
    pub(crate) fn listed(self) -> bool {
        matches!(self, Self::On | Self::NameOnly)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsObservation {
    Missing,
    Digest(String),
    Unavailable,
}

impl SettingsObservation {
    fn from_read(read: &io::Result<Option<Vec<u8>>>) -> Self {
        match read {
            Ok(Some(bytes)) => Self::Digest(hex::encode(Sha256::digest(bytes))),
            Ok(None) => Self::Missing,
            Err(_) => Self::Unavailable,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisibilitySnapshot {
    pub version: String,
    pub basis: String,
    /// Digests of inspected settings, including explicit absence. No settings
    /// contents, credentials, or unrelated configuration values are retained.
    pub settings_files: BTreeMap<PathBuf, SettingsObservation>,
    /// None means the root's policy could not be read or parsed. Inventory is
    /// still useful, but this policy cannot authorize an exposure decision.
    pub root_overrides: BTreeMap<PathBuf, Option<BTreeMap<String, CatalogVisibility>>>,
    pub placements: BTreeMap<String, CatalogVisibility>,
}

impl VisibilitySnapshot {
    pub(crate) fn skill_visibility(
        &self,
        root: &Path,
        name: &str,
        invocation: ModelInvocation,
    ) -> CatalogVisibility {
        let Some(Some(overrides)) = self.root_overrides.get(root) else {
            return CatalogVisibility::Unknown;
        };
        let configured = overrides.get(name).copied();
        match (configured, invocation) {
            (Some(CatalogVisibility::Off), _) => CatalogVisibility::Off,
            (Some(CatalogVisibility::UserInvocableOnly), _) => CatalogVisibility::UserInvocableOnly,
            (_, ModelInvocation::Unknown) => CatalogVisibility::Unknown,
            // The documented listing states do not specify how an explicit
            // enabling override interacts with a package's invocation block.
            // Do not invent precedence or claim an observed runtime catalog.
            (Some(mode), ModelInvocation::ExplicitOnly) if mode.listed() => {
                CatalogVisibility::Unknown
            }
            (_, ModelInvocation::ExplicitOnly) => CatalogVisibility::UserInvocableOnly,
            (mode, ModelInvocation::Automatic) => mode.unwrap_or(CatalogVisibility::On),
        }
    }

    pub(crate) fn summary(&self) -> Value {
        let mut states = BTreeMap::<String, usize>::new();
        for mode in self.placements.values() {
            let name = serde_json::to_value(mode)
                .expect("visibility enum serializes")
                .as_str()
                .expect("visibility enum is a string")
                .to_owned();
            *states.entry(name).or_default() += 1;
        }
        serde_json::json!({
            "agent": "claude-code",
            "basis": self.basis,
            "placement_count": self.placements.len(),
            "state_counts": states,
            "unknown_count": self.placements.values().filter(|mode| **mode == CatalogVisibility::Unknown).count(),
            "session_catalog_observed": false,
            "settings_file_count": self.settings_files.values().filter(|observation| matches!(observation, SettingsObservation::Digest(_))).count(),
            "unknown_configuration_root_count": self.root_overrides.values().filter(|overrides| overrides.is_none()).count(),
        })
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.version != VERSION {
            return Err("legacy_native_visibility_requires_rescan");
        }
        for (path, expected) in &self.settings_files {
            let current = SettingsObservation::from_read(&read_settings(path));
            if current != *expected {
                return Err("native_visibility_settings_changed");
            }
        }
        Ok(())
    }
}

pub(crate) fn observe(home: &Path, scan: &mut ScanResult) {
    let mut snapshot = VisibilitySnapshot {
        version: VERSION.into(),
        basis: "local_root_settings_and_frontmatter_not_session_catalog".into(),
        settings_files: BTreeMap::new(),
        root_overrides: BTreeMap::new(),
        placements: BTreeMap::new(),
    };
    let roots = scan
        .roots
        .iter()
        .filter(|root| {
            root.agent == Some(AgentKind::ClaudeCode) && root.kind == crate::scan::RootKind::Skills
        })
        .map(|root| root.path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let user_config_dir = home.join(".claude");
    let mut settings_by_root = BTreeMap::new();
    let mut parsed_files = BTreeMap::new();
    for root in roots {
        let mut paths = vec![user_config_dir.join("settings.json")];
        // An explicitly supplied conventional project root also identifies
        // its local configuration scope. Arbitrary roots never imply a project.
        if root.file_name().is_some_and(|name| name == "skills") {
            if let Some(parent) = root.parent().filter(|parent| {
                parent.file_name().is_some_and(|name| name == ".claude")
                    && *parent != user_config_dir
            }) {
                paths.push(parent.join("settings.json"));
                paths.push(parent.join("settings.local.json"));
            }
        }
        let mut overrides = BTreeMap::new();
        let mut policy_known = true;
        for path in paths {
            if !parsed_files.contains_key(&path) {
                let read = read_settings(&path);
                snapshot
                    .settings_files
                    .insert(path.clone(), SettingsObservation::from_read(&read));
                let values = read.ok().and_then(|bytes| match bytes {
                    Some(bytes) => parse_overrides(&bytes).ok(),
                    None => Some(BTreeMap::new()),
                });
                parsed_files.insert(path.clone(), values);
            }
            match &parsed_files[&path] {
                Some(values) => overrides.extend(values.clone()),
                None => policy_known = false,
            }
        }
        settings_by_root.insert(root, policy_known.then_some(overrides));
    }
    snapshot.root_overrides = settings_by_root;
    let skills = scan
        .skills
        .iter()
        .map(|skill| (skill.id.as_str(), skill))
        .collect::<BTreeMap<_, _>>();
    for placement in scan
        .placements
        .iter_mut()
        .filter(|placement| placement.agent == Some(AgentKind::ClaudeCode))
    {
        let skill = skills[placement.skill_id.as_str()];
        // Personal/project command names come from the placement directory;
        // frontmatter `name` is only a display label in these locations.
        let command_name = placement
            .directory
            .file_name()
            .and_then(|name| name.to_str());
        let configured = snapshot.skill_visibility(
            &placement.root,
            command_name.unwrap_or(&skill.name),
            skill.metadata.model_invocation,
        );
        let visibility = if !placement.default_exposed {
            CatalogVisibility::NotDiscovered
        } else if configured == CatalogVisibility::Off {
            CatalogVisibility::Off
        } else if placement.entrypoint_digest.is_none() || command_name.is_none() {
            CatalogVisibility::Unknown
        } else {
            configured
        };
        // Unknown retains a conservative possible-exposure count. The typed
        // fact prevents that Snapshot from authorizing downstream decisions.
        placement.default_exposed = visibility.listed() || visibility == CatalogVisibility::Unknown;
        snapshot.placements.insert(placement.id.clone(), visibility);
    }
    scan.native_visibility = Some(snapshot);
}

fn parse_overrides(bytes: &[u8]) -> io::Result<BTreeMap<String, CatalogVisibility>> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid settings JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "settings must be a JSON object")
    })?;
    let Some(overrides) = object.get("skillOverrides") else {
        return Ok(BTreeMap::new());
    };
    let overrides = overrides.as_object().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "skillOverrides must be an object",
        )
    })?;
    overrides
        .iter()
        .map(|(name, value)| {
            let mode = match value.as_str() {
                Some("on") => CatalogVisibility::On,
                Some("name-only") => CatalogVisibility::NameOnly,
                Some("user-invocable-only") => CatalogVisibility::UserInvocableOnly,
                Some("off") => CatalogVisibility::Off,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unsupported skillOverrides state",
                    ));
                }
            };
            Ok((name.clone(), mode))
        })
        .collect()
}

fn read_settings(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent")
    })?;
    let dir = match Dir::open_ambient_dir(parent, ambient_authority()) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings path has no name"))?;
    match dir.symlink_metadata(name) {
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "settings must be a regular file without a symlink",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        ._cap_fs_ext_follow(cap_primitives::fs::FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = dir.open_with(name, &options)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.is_symlink() || metadata.len() > MAX_SETTINGS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settings must be a regular file of at most 256 KiB",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settings grew beyond 256 KiB",
        ));
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{ExplicitSkillRoot, ScanOptions, parse_skill_markdown, scan};
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(root: &Path, directory: &str, metadata: &str) {
        let path = root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\n{metadata}\n---\nLocal fixture\n"),
        )
        .unwrap();
    }

    #[test]
    fn native_visibility_distinguishes_listing_from_inventory_and_other_agents() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join(".claude/skills");
        for (name, metadata) in [
            ("automatic", "name: automatic"),
            (
                "allowed",
                "name: allowed\ndisable-model-invocation: false\nuser-invocable: false",
            ),
            ("manual", "name: manual\ndisable-model-invocation: true"),
            ("compact", "name: compact"),
            ("explicit", "name: explicit"),
            ("disabled-directory", "name: different-display-name"),
        ] {
            write_skill(&root, name, metadata);
        }
        fs::write(temp.path().join(".claude/settings.json"), r#"{"skillOverrides":{"compact":"name-only","explicit":"user-invocable-only","disabled-directory":"off"}}"#).unwrap();
        write_skill(
            &temp.path().join(".codex/skills"),
            "manual",
            "name: manual\ndisable-model-invocation: true",
        );
        let result = scan(&ScanOptions::for_home(temp.path())).unwrap();
        assert_eq!(result.placements.len(), 7);
        assert_eq!(
            result
                .placements
                .iter()
                .filter(|p| p.default_exposed)
                .count(),
            4
        );
        let snapshot = result.native_visibility.as_ref().unwrap();
        for placement in &result.placements {
            if placement.agent == Some(AgentKind::Codex) {
                assert!(placement.default_exposed);
                continue;
            }
            let expected = match placement.directory.file_name().unwrap().to_str().unwrap() {
                "automatic" | "allowed" => CatalogVisibility::On,
                "manual" | "explicit" => CatalogVisibility::UserInvocableOnly,
                "compact" => CatalogVisibility::NameOnly,
                "disabled-directory" => CatalogVisibility::Off,
                name => panic!("unexpected placement {name}"),
            };
            assert_eq!(snapshot.placements[&placement.id], expected);
            assert!(placement.entrypoint_digest.is_some());
        }
        assert_eq!(snapshot.summary()["session_catalog_observed"], false);
        assert_eq!(snapshot.summary()["state_counts"]["user-invocable-only"], 2);
    }

    #[test]
    fn native_visibility_uses_only_explicit_project_scope_and_tracks_config_drift() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let user_root = home.join(".claude/skills");
        let project_root = temp.path().join("project/.claude/skills");
        for root in [&user_root, &project_root] {
            write_skill(root, "shared", "name: shared");
        }
        let settings = home.join(".claude/settings.json");
        fs::write(
            &settings,
            r#"{"skillOverrides":{"shared":"off"},"env":{"SECRET":"do-not-persist"}}"#,
        )
        .unwrap();
        fs::write(
            project_root.parent().unwrap().join("settings.json"),
            r#"{"skillOverrides":{"shared":"on"}}"#,
        )
        .unwrap();
        fs::write(
            project_root.parent().unwrap().join("settings.local.json"),
            r#"{"skillOverrides":{"shared":"name-only"}}"#,
        )
        .unwrap();
        let mut options = ScanOptions::for_home(&home);
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::ClaudeCode,
            path: project_root.clone(),
        });
        let result = scan(&options).unwrap();
        let snapshot = result.native_visibility.as_ref().unwrap();
        assert_eq!(
            snapshot.skill_visibility(&user_root, "shared", ModelInvocation::Automatic),
            CatalogVisibility::Off
        );
        assert_eq!(
            snapshot.skill_visibility(&project_root, "shared", ModelInvocation::Automatic),
            CatalogVisibility::NameOnly
        );
        assert!(snapshot.validate().is_ok());
        assert!(
            !serde_json::to_string(snapshot)
                .unwrap()
                .contains("do-not-persist")
        );
        fs::write(&settings, "{}").unwrap();
        assert_eq!(
            snapshot.validate(),
            Err("native_visibility_settings_changed")
        );
        let fresh = scan(&options).unwrap().native_visibility.unwrap();
        fs::remove_file(&settings).unwrap();
        assert_eq!(fresh.validate(), Err("native_visibility_settings_changed"));
        let absent = scan(&options).unwrap().native_visibility.unwrap();
        fs::write(&settings, "{}").unwrap();
        assert_eq!(absent.validate(), Err("native_visibility_settings_changed"));
    }

    #[test]
    fn native_visibility_preserves_unknown_without_pretending_it_is_hidden() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join(".claude/skills");
        write_skill(
            &root,
            "conflict",
            "name: conflict\ndisable-model-invocation: true",
        );
        write_skill(
            &root,
            "invalid",
            "name: invalid\ndisable-model-invocation: perhaps",
        );
        fs::write(
            temp.path().join(".claude/settings.json"),
            r#"{"skillOverrides":{"conflict":"on"}}"#,
        )
        .unwrap();
        let result = scan(&ScanOptions::for_home(temp.path())).unwrap();
        assert_eq!(
            result.native_visibility.unwrap().summary()["unknown_count"],
            2
        );
        assert!(
            result
                .placements
                .iter()
                .all(|placement| placement.default_exposed)
        );
    }

    #[test]
    fn native_visibility_frontmatter_respects_boolean_forms_and_scope() {
        for value in [
            "true",
            "YES",
            "On",
            "1",
            "true # only explicit",
            "\"true\" # explicit",
        ] {
            assert_eq!(
                parse_skill_markdown(&format!("---\ndisable-model-invocation: {value}\n---"))
                    .model_invocation,
                ModelInvocation::ExplicitOnly
            );
        }
        for value in ["false", "NO", "Off", "0"] {
            assert_eq!(
                parse_skill_markdown(&format!("---\ndisable-model-invocation: {value}\n---"))
                    .model_invocation,
                ModelInvocation::Automatic
            );
        }
        for markdown in [
            "\n---\ndisable-model-invocation: true\n---",
            "---\nmetadata:\n  disable-model-invocation: true\n---",
            "---\ndescription: |\n  disable-model-invocation: true\n---",
            "---\nname: example\n---\ndisable-model-invocation: true",
        ] {
            assert_eq!(
                parse_skill_markdown(markdown).model_invocation,
                ModelInvocation::Automatic
            );
        }
        assert_eq!(
            parse_skill_markdown("---\ndisable-model-invocation:\n---").model_invocation,
            ModelInvocation::Unknown
        );
    }

    #[test]
    fn native_visibility_keeps_inventory_available_when_settings_are_unknown() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join(".claude/skills");
        write_skill(&root, "ordinary", "name: ordinary");
        let path = temp.path().join(".claude/settings.json");
        for bytes in [
            "{invalid",
            r#"{"skillOverrides":{"ordinary":"unknown-state"}}"#,
        ] {
            fs::write(&path, bytes).unwrap();
            let result = scan(&ScanOptions::for_home(temp.path())).unwrap();
            assert_eq!(result.skills.len(), 1);
            let snapshot = result.native_visibility.unwrap();
            assert_eq!(snapshot.summary()["unknown_count"], 1);
            assert_eq!(snapshot.summary()["unknown_configuration_root_count"], 1);
            assert!(snapshot.validate().is_ok());
            fs::write(&path, "{}").unwrap();
            assert_eq!(
                snapshot.validate(),
                Err("native_visibility_settings_changed")
            );
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let result = scan(&ScanOptions::for_home(temp.path())).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(
            result.native_visibility.unwrap().summary()["unknown_count"],
            1
        );
    }

    #[test]
    fn native_visibility_rejects_unreadable_or_unrecognized_settings() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.json");
        for bytes in [
            b"{invalid".as_slice(),
            b"[]",
            br#"{"skillOverrides":[]}"#,
            br#"{"skillOverrides":{"x":"sometimes"}}"#,
        ] {
            assert!(parse_overrides(bytes).is_err());
        }
        fs::write(&path, vec![b' '; MAX_SETTINGS_BYTES as usize + 1]).unwrap();
        assert!(read_settings(&path).is_err());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(read_settings(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn native_visibility_never_follows_settings_links_or_opens_special_files() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target.json");
        let path = temp.path().join("settings.json");
        fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(read_settings(&path).is_err());
        fs::remove_file(&path).unwrap();
        let fifo_path = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
        assert!(read_settings(&path).is_err());
    }
}
