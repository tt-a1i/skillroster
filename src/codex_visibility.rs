//! Codex user-Skill local-file configuration, not an effective session catalog.
//! Resolving configured paths observes metadata identity only; it grants no
//! permission to read Skill contents or mutate their locations.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::claude_visibility::{CatalogVisibility, SettingsObservation, read_settings};
use crate::harness::AgentKind;
use crate::scan::ScanResult;

const VERSION: &str = "codex-user-skill-config-v3";
const BASIS: &str = "codex_user_skill_local_file_configuration_not_effective_session_catalog";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PathObservation {
    Resolved(PathBuf),
    Missing(PathBuf),
    Unavailable,
}

impl PathObservation {
    fn observe(path: &Path) -> Self {
        match fs::canonicalize(path) {
            Ok(path) => Self::Resolved(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Freeze the nearest existing ancestor as well, so retargeting
                // an alias with a still-missing leaf cannot preserve freshness.
                let mut ancestor = path;
                while let Some(parent) = ancestor.parent() {
                    ancestor = parent;
                    match fs::canonicalize(ancestor) {
                        Ok(resolved) => {
                            return Self::Missing(
                                resolved.join(path.strip_prefix(ancestor).expect("path ancestor")),
                            );
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(_) => return Self::Unavailable,
                    }
                }
                Self::Unavailable
            }
            Err(_) => Self::Unavailable,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Rule {
    Name {
        name: String,
        enabled: bool,
    },
    Path {
        identity: PathObservation,
        enabled: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisibilitySnapshot {
    pub version: String,
    pub basis: String,
    pub config_path: PathBuf,
    pub settings_observation: SettingsObservation,
    pub placements: BTreeMap<String, CatalogVisibility>,
    user_roots: Vec<PathBuf>,
    #[serde(default)]
    verified_names: BTreeMap<PathBuf, Option<String>>,
    /// None preserves unreadable or unsupported configuration as unknown.
    #[serde(default)]
    rules: Option<Vec<Rule>>,
    configured_paths: BTreeMap<PathBuf, PathObservation>,
    #[serde(default)]
    root_identities: Option<BTreeMap<PathBuf, PathObservation>>,
}

impl VisibilitySnapshot {
    #[cfg(test)]
    pub(crate) fn visibility_for_entrypoint(
        &self,
        entrypoint: &Path,
        name: &str,
    ) -> CatalogVisibility {
        self.scoped_visibility(entrypoint, Some(name))
    }

    pub(crate) fn verified_name_for_entrypoint(&self, entrypoint: &Path) -> Option<&str> {
        let identity = fs::canonicalize(entrypoint).ok()?;
        self.verified_names.get(&identity)?.as_deref()
    }

    pub(crate) fn visibility_for_entrypoint_with_name(
        &self,
        entrypoint: &Path,
        name: Option<&str>,
    ) -> CatalogVisibility {
        self.scoped_visibility(entrypoint, name)
    }

    pub(crate) fn visibility_for_core_target_entrypoint(
        &self,
        target: &Path,
        source: &Path,
    ) -> CatalogVisibility {
        self.scoped_visibility(target, self.verified_name_for_entrypoint(source))
    }

    pub(crate) fn visibility_for_updated_entrypoint(
        &self,
        entrypoint: &Path,
        upstream_content: &str,
    ) -> CatalogVisibility {
        let name = parse_native_name(upstream_content);
        self.visibility_for_entrypoint_with_name(entrypoint, name.as_deref())
    }

    fn scoped_visibility(&self, entrypoint: &Path, name: Option<&str>) -> CatalogVisibility {
        if !self
            .user_roots
            .iter()
            .any(|root| entrypoint.starts_with(root))
        {
            return CatalogVisibility::Unknown;
        }
        self.evaluate(entrypoint, name)
    }

    /// The caller establishes source trust; this grants no read authority.
    pub(crate) fn visibility_for_core_source_entrypoint(
        &self,
        entrypoint: &Path,
    ) -> CatalogVisibility {
        self.evaluate(entrypoint, self.verified_name_for_entrypoint(entrypoint))
    }

    pub(crate) fn visibility_for_retargeted_entrypoint(
        &self,
        old: &Path,
        new: &Path,
    ) -> CatalogVisibility {
        let Some(rules) = &self.rules else {
            return CatalogVisibility::Unknown;
        };
        let old_identity = PathObservation::observe(old);
        if matches!(old_identity, PathObservation::Resolved(_))
            && rules.iter().any(
                |rule| matches!(rule, Rule::Path { identity, .. } if *identity == old_identity),
            )
        {
            // Moving the old canonical source can change which ordered alias
            // rules match. Refuse this unproven projection conservatively.
            return CatalogVisibility::Unknown;
        }
        self.evaluate(new, self.verified_name_for_entrypoint(old))
    }

    fn evaluate(&self, entrypoint: &Path, name: Option<&str>) -> CatalogVisibility {
        if !entrypoint.is_absolute()
            || entrypoint
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return CatalogVisibility::Unknown;
        }
        let Some(rules) = &self.rules else {
            return CatalogVisibility::Unknown;
        };
        let observed = PathObservation::observe(entrypoint);
        if observed == PathObservation::Unavailable {
            return CatalogVisibility::Unknown;
        }
        let mut mode = CatalogVisibility::On;
        for rule in rules {
            let enabled = match rule {
                Rule::Name {
                    name: selector,
                    enabled,
                } => match name {
                    Some(name) if name == selector => Some(*enabled),
                    Some(_) => None,
                    None => {
                        let matching = if *enabled {
                            CatalogVisibility::On
                        } else {
                            CatalogVisibility::Off
                        };
                        if mode != matching {
                            mode = CatalogVisibility::Unknown;
                        }
                        None
                    }
                },
                Rule::Path { identity, enabled } if *identity == observed => {
                    if matches!(observed, PathObservation::Missing(_)) && *enabled {
                        mode = CatalogVisibility::Unknown;
                        None
                    } else {
                        Some(*enabled)
                    }
                }
                Rule::Path { .. } => None,
            };
            if let Some(enabled) = enabled {
                mode = if enabled {
                    CatalogVisibility::On
                } else {
                    CatalogVisibility::Off
                };
            }
        }
        mode
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.version != VERSION {
            return Err("legacy_codex_visibility_requires_rescan");
        }
        let roots = self
            .root_identities
            .as_ref()
            .ok_or("legacy_codex_visibility_requires_rescan")?;
        if roots
            .iter()
            .any(|(path, expected)| PathObservation::observe(path) != *expected)
        {
            return Err("codex_visibility_root_identity_changed");
        }
        if SettingsObservation::from_read(&read_settings(&self.config_path))
            != self.settings_observation
        {
            return Err("codex_visibility_settings_changed");
        }
        if self
            .configured_paths
            .iter()
            .any(|(path, expected)| PathObservation::observe(path) != *expected)
        {
            return Err("codex_visibility_path_identity_changed");
        }
        Ok(())
    }

    pub(crate) fn summary(&self) -> Value {
        let mut states = BTreeMap::<String, usize>::new();
        for mode in self.placements.values() {
            let name = serde_json::to_value(mode)
                .expect("visibility serializes")
                .as_str()
                .expect("visibility is a string")
                .to_owned();
            *states.entry(name).or_default() += 1;
        }
        json!({
            "agent": "codex",
            "basis": self.basis,
            "config_path": self.config_path,
            "codex_home": self.config_path.parent(),
            "placement_count": self.placements.len(),
            "state_counts": states,
            "unknown_count": self.placements.values().filter(|mode| **mode == CatalogVisibility::Unknown).count(),
            "session_catalog_observed": false,
            "settings_file_count": usize::from(matches!(self.settings_observation, SettingsObservation::Digest(_))),
            "skill_configuration_known": self.rules.is_some(),
            "host_config_validity": "unverified",
        })
    }
}

pub(crate) fn observe(home: &Path, scan: &mut ScanResult) {
    let config_path = home.join(".codex/config.toml");
    let read = read_settings(&config_path);
    let mut snapshot = VisibilitySnapshot {
        version: VERSION.into(),
        basis: BASIS.into(),
        config_path: config_path.clone(),
        settings_observation: SettingsObservation::from_read(&read),
        placements: BTreeMap::new(),
        user_roots: vec![home.join(".codex/skills"), home.join(".agents/skills")],
        verified_names: BTreeMap::new(),
        rules: None,
        configured_paths: BTreeMap::new(),
        root_identities: Some(
            scan.roots
                .iter()
                .filter(|root| root.kind == crate::scan::RootKind::Skills)
                .map(|root| root.path.clone())
                .chain([home.join(".agents/skills"), home.join(".codex/skills")])
                .map(|path| {
                    let identity = PathObservation::observe(&path);
                    (path, identity)
                })
                .collect(),
        ),
    };
    snapshot.rules = match read {
        Ok(None) => Some(Vec::new()),
        Ok(Some(bytes)) => parse_config(&bytes, home, &config_path, &mut snapshot.configured_paths),
        Err(_) => None,
    };
    for placement in &scan.placements {
        if placement.entrypoint_digest.is_some() {
            if let Ok(identity) = fs::canonicalize(&placement.entrypoint) {
                let name = scan
                    .verified_entrypoint_names
                    .get(&placement.id)
                    .cloned()
                    .flatten();
                snapshot
                    .verified_names
                    .entry(identity)
                    .and_modify(|existing| {
                        if *existing != name {
                            *existing = None;
                        }
                    })
                    .or_insert(name);
            }
        }
    }
    for placement in scan
        .placements
        .iter_mut()
        .filter(|placement| placement.agent == Some(AgentKind::Codex))
    {
        let name = scan
            .verified_entrypoint_names
            .get(&placement.id)
            .and_then(Option::as_deref);
        let configured = snapshot.scoped_visibility(&placement.entrypoint, name);
        let visibility = if !placement.default_exposed {
            CatalogVisibility::NotDiscovered
        } else {
            configured
        };
        placement.default_exposed = matches!(
            visibility,
            CatalogVisibility::On | CatalogVisibility::Unknown
        );
        snapshot.placements.insert(placement.id.clone(), visibility);
    }
    scan.codex_visibility = Some(snapshot);
}

fn parse_config(
    bytes: &[u8],
    home: &Path,
    config_path: &Path,
    paths: &mut BTreeMap<PathBuf, PathObservation>,
) -> Option<Vec<Rule>> {
    let value = std::str::from_utf8(bytes)
        .ok()?
        .parse::<toml::Value>()
        .ok()?;
    let table = value.as_table()?;
    let Some(skills) = table.get("skills") else {
        return Some(Vec::new());
    };
    let Some(config) = skills.as_table()?.get("config") else {
        return Some(Vec::new());
    };
    let mut rules = Vec::new();
    for entry in config.as_array()? {
        let entry = entry.as_table()?;
        let enabled = entry.get("enabled")?.as_bool()?;
        let name = match entry.get("name") {
            Some(value) => Some(value.as_str()?),
            None => None,
        };
        let raw_path = match entry.get("path") {
            Some(value) => Some(value.as_str()?),
            None => None,
        };
        match (name, raw_path) {
            (Some(name), None) => {
                let name = name.trim();
                if !name.is_empty() {
                    rules.push(Rule::Name {
                        name: name.into(),
                        enabled,
                    });
                }
            }
            (None, Some(raw)) => {
                if raw.is_empty()
                    || raw.contains('\0')
                    || (raw.starts_with('~') && !raw.starts_with("~/"))
                {
                    return None;
                }
                let path = if let Some(relative) = raw.strip_prefix("~/") {
                    absolute_path(&home.join(relative), config_path.parent()?)?
                } else {
                    absolute_path(Path::new(raw), config_path.parent()?)?
                };
                let identity = PathObservation::observe(&path);
                paths.insert(path, identity.clone());
                if identity == PathObservation::Unavailable {
                    return None;
                }
                rules.push(Rule::Path { identity, enabled });
            }
            // Both selectors and neither selector are native inert entries.
            _ => {}
        }
    }
    Some(rules)
}

/// Bounded native-name parser. Unsupported YAML stays unknown; names never
/// fall back to directory labels or metadata from a deduplicated Skill.
pub(crate) fn parse_native_name(markdown: &str) -> Option<String> {
    let mut lines = markdown.lines();
    if lines.next()? != "---" {
        return None;
    }
    let mut name = None;
    let mut name_active = false;
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        if line.starts_with(char::is_whitespace) {
            if name_active && !line.trim().is_empty() && !line.trim().starts_with('#') {
                // YAML may fold an indented continuation into the preceding
                // plain name. That syntax is outside this bounded parser.
                return None;
            }
            continue;
        }
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        name_active = false;
        let Some(value) = line.strip_prefix("name:") else {
            continue;
        };
        name_active = true;
        if name.is_some() {
            return None;
        }
        let value = value.trim();
        let parsed = if let Some(quote) = value.chars().next().filter(|c| *c == '\'' || *c == '"') {
            let tail = &value[quote.len_utf8()..];
            let end = tail.find(quote)?;
            let contents = &tail[..end];
            let rest = tail[end + quote.len_utf8()..].trim();
            if (!rest.is_empty() && !rest.starts_with('#')) || contents.contains(['\\', '\'', '"'])
            {
                return None;
            }
            contents
        } else {
            let contents = value
                .split_once(" #")
                .map_or(value, |(value, _)| value)
                .trim();
            // Restrict the unquoted subset instead of emulating YAML scalars.
            if contents.is_empty()
                || !contents
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
            {
                return None;
            }
            if matches!(
                contents.to_ascii_lowercase().as_str(),
                "true" | "false" | "null" | "yes" | "no" | "on" | "off"
            ) || contents.parse::<f64>().is_ok()
            {
                return None;
            }
            contents
        };
        let parsed = parsed.trim();
        if parsed.is_empty() {
            return None;
        }
        name = Some(parsed.to_owned());
    }
    closed.then_some(name).flatten()
}

fn absolute_path(path: &Path, base: &Path) -> Option<PathBuf> {
    // Preserve `..` until canonicalization: a preceding component may be a
    // symlink, so lexical normalization would change native path identity.
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    };
    joined.is_absolute().then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn snapshot(home: &Path) -> VisibilitySnapshot {
        let mut scan = ScanResult::default();
        observe(home, &mut scan);
        let mut observed = scan.codex_visibility.unwrap();
        let example = home.join(".agents/skills/example/SKILL.md");
        if let Ok(identity) = fs::canonicalize(example) {
            observed
                .verified_names
                .insert(identity, Some("example".into()));
        }
        observed
    }

    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let temp = TempDir::new().unwrap();
        let skill = temp.path().join(".agents/skills/example/SKILL.md");
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(&skill, "fixture").unwrap();
        let config = temp.path().join(".codex/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        (temp, skill, config)
    }

    #[test]
    fn typed_entries_match_entrypoints_with_relative_tilde_and_last_wins() {
        let (temp, skill, config) = fixture();
        fs::write(&config, "[[skills.config]]\nenabled=false\n[[skills.config]]\npath='../.agents/skills/example/SKILL.md'\nenabled=false\n[[skills.config]]\npath='~/.agents/skills/example/SKILL.md'\nenabled=true\n").unwrap();
        assert_eq!(
            snapshot(temp.path()).visibility_for_entrypoint(&skill, "example"),
            CatalogVisibility::On
        );
        fs::write(
            &config,
            "[[skills.config]]\npath='../.agents/skills/example'\nenabled=false\n",
        )
        .unwrap();
        assert_eq!(
            snapshot(temp.path()).visibility_for_entrypoint(&skill, "example"),
            CatalogVisibility::On
        );
        fs::write(
            &config,
            "[[skills.config]]\npath='../.agents/skills/example/SKILL.md'\nenabled=false\n",
        )
        .unwrap();
        assert_eq!(
            snapshot(temp.path()).visibility_for_entrypoint(&skill, "example"),
            CatalogVisibility::Off
        );
    }

    #[test]
    fn malformed_or_unavailable_configuration_is_unknown() {
        let (temp, skill, config) = fixture();
        for contents in [
            "invalid toml",
            "[[skills.config]]\npath='x'",
            "[[skills.config]]\nenabled='false'",
            "[[skills.config]]\npath=5\nenabled=false",
        ] {
            fs::write(&config, contents).unwrap();
            assert_eq!(
                snapshot(temp.path()).visibility_for_entrypoint(&skill, "example"),
                CatalogVisibility::Unknown
            );
        }
        fs::remove_file(&config).unwrap();
        fs::create_dir(&config).unwrap();
        assert_eq!(
            snapshot(temp.path()).visibility_for_entrypoint(&skill, "example"),
            CatalogVisibility::Unknown
        );
    }

    #[test]
    fn configuration_appearance_and_changes_require_rescan() {
        let (temp, _, config) = fixture();
        let absent = snapshot(temp.path());
        assert!(absent.validate().is_ok());
        fs::write(&config, "").unwrap();
        assert_eq!(absent.validate(), Err("codex_visibility_settings_changed"));
        let present = snapshot(temp.path());
        fs::write(&config, "# changed").unwrap();
        assert_eq!(present.validate(), Err("codex_visibility_settings_changed"));
    }

    #[test]
    fn configured_missing_target_appearance_requires_rescan() {
        let (temp, _, config) = fixture();
        fs::write(
            &config,
            "[[skills.config]]\npath='../.agents/skills/future/SKILL.md'\nenabled=false\n",
        )
        .unwrap();
        let before = snapshot(temp.path());
        let future = temp.path().join(".agents/skills/future/SKILL.md");
        assert_eq!(
            before.visibility_for_entrypoint(&future, "example"),
            CatalogVisibility::Off
        );
        fs::create_dir_all(future.parent().unwrap()).unwrap();
        fs::write(future, "fixture").unwrap();
        assert_eq!(
            before.validate(),
            Err("codex_visibility_path_identity_changed")
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonical_aliases_match_and_retarget_requires_rescan() {
        let (temp, skill, config) = fixture();
        let alias = temp.path().join(".agents/skills/alias");
        std::os::unix::fs::symlink(skill.parent().unwrap(), &alias).unwrap();
        fs::write(
            &config,
            "[[skills.config]]\npath='../.agents/skills/alias/SKILL.md'\nenabled=false\n",
        )
        .unwrap();
        let before = snapshot(temp.path());
        assert_eq!(
            before.visibility_for_entrypoint(&skill, "example"),
            CatalogVisibility::Off
        );
        fs::remove_file(&alias).unwrap();
        let other = temp.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("SKILL.md"), "other fixture").unwrap();
        std::os::unix::fs::symlink(other, alias).unwrap();
        assert_eq!(
            before.validate(),
            Err("codex_visibility_path_identity_changed")
        );
    }
    #[cfg(unix)]
    #[test]
    fn unavailable_configured_alias_is_unknown_and_its_recovery_requires_rescan() {
        let (temp, skill, config) = fixture();
        let alias = temp.path().join("loop");
        std::os::unix::fs::symlink(&alias, &alias).unwrap();
        fs::write(
            &config,
            "[[skills.config]]\npath='../loop/SKILL.md'\nenabled=false\n",
        )
        .unwrap();
        let before = snapshot(temp.path());
        assert_eq!(
            before.visibility_for_entrypoint(&skill, "example"),
            CatalogVisibility::Unknown
        );
        assert!(before.validate().is_ok());
        fs::remove_file(alias).unwrap();
        assert_eq!(
            before.validate(),
            Err("codex_visibility_path_identity_changed")
        );
    }

    #[cfg(unix)]
    #[test]
    fn missing_leaf_still_freezes_its_symlinked_ancestor() {
        let (temp, _, config) = fixture();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(first, &alias).unwrap();
        fs::write(
            &config,
            "[[skills.config]]\npath='../alias/missing/SKILL.md'\nenabled=false\n",
        )
        .unwrap();
        let before = snapshot(temp.path());
        assert!(before.validate().is_ok());
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(second, alias).unwrap();
        assert_eq!(
            before.validate(),
            Err("codex_visibility_path_identity_changed")
        );
    }
    #[test]
    fn user_projection_does_not_claim_arbitrary_root_visibility() {
        let (temp, _, _) = fixture();
        let outside = temp.path().join("project/.agents/skills/example/SKILL.md");
        assert_eq!(
            snapshot(temp.path()).visibility_for_entrypoint(&outside, "example"),
            CatalogVisibility::Unknown
        );
    }
    #[test]
    fn verified_core_source_checks_canonical_disable_outside_user_root() {
        let (temp, _, config) = fixture();
        let source = temp.path().join("library/example/SKILL.md");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "fixture").unwrap();
        fs::write(
            &config,
            "[[skills.config]]\npath='../library/example/SKILL.md'\nenabled=false\n",
        )
        .unwrap();
        let observed = snapshot(temp.path());
        assert_eq!(
            observed.visibility_for_entrypoint(&source, "example"),
            CatalogVisibility::Unknown
        );
        assert_eq!(
            observed.visibility_for_core_source_entrypoint(&source),
            CatalogVisibility::Off
        );
        let target = temp.path().join(".codex/skills/example/SKILL.md");
        assert_eq!(
            observed.visibility_for_entrypoint(&target, "example"),
            CatalogVisibility::On
        );
    }
    #[test]
    fn missing_user_root_appearance_requires_rescan() {
        let temp = TempDir::new().unwrap();
        let before = snapshot(temp.path());
        fs::create_dir_all(temp.path().join(".agents/skills")).unwrap();
        assert_eq!(
            before.validate(),
            Err("codex_visibility_root_identity_changed")
        );
    }

    #[test]
    fn legacy_root_observations_require_rescan() {
        let temp = TempDir::new().unwrap();
        let mut before = snapshot(temp.path());
        before.root_identities = None;
        assert_eq!(
            before.validate(),
            Err("legacy_codex_visibility_requires_rescan")
        );
    }

    #[cfg(unix)]
    #[test]
    fn empty_user_root_alias_retarget_requires_rescan() {
        let temp = TempDir::new().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        fs::create_dir(temp.path().join(".agents")).unwrap();
        let root = temp.path().join(".agents/skills");
        std::os::unix::fs::symlink(first, &root).unwrap();
        let before = snapshot(temp.path());
        assert!(before.validate().is_ok());
        fs::remove_file(&root).unwrap();
        std::os::unix::fs::symlink(second, root).unwrap();
        assert_eq!(
            before.validate(),
            Err("codex_visibility_root_identity_changed")
        );
    }
    #[test]
    fn name_rules_trim_case_and_preserve_cross_selector_order() {
        let (temp, skill, config) = fixture();
        for (contents, name, expected) in [
            (
                "[[skills.config]]\nname='  example  '\nenabled=false",
                "example",
                CatalogVisibility::Off,
            ),
            (
                "[[skills.config]]\nname='EXAMPLE'\nenabled=false",
                "example",
                CatalogVisibility::On,
            ),
            (
                "[[skills.config]]\nname='*'\nenabled=false",
                "example",
                CatalogVisibility::On,
            ),
            (
                "[[skills.config]]\nname='example'\nenabled=false\n[[skills.config]]\npath='../.agents/skills/example/SKILL.md'\nenabled=true",
                "example",
                CatalogVisibility::On,
            ),
            (
                "[[skills.config]]\npath='../.agents/skills/example/SKILL.md'\nenabled=true\n[[skills.config]]\nname='example'\nenabled=false",
                "example",
                CatalogVisibility::Off,
            ),
            (
                "[[skills.config]]\nname='example'\nenabled=false\n[[skills.config]]\nname='example'\npath=''\nenabled=true",
                "example",
                CatalogVisibility::Off,
            ),
            (
                "[[skills.config]]\nname='example'\npath='../.agents/skills/example/SKILL.md'\nenabled=false",
                "example",
                CatalogVisibility::On,
            ),
        ] {
            fs::write(&config, contents).unwrap();
            assert_eq!(
                snapshot(temp.path()).visibility_for_entrypoint(&skill, name),
                expected,
                "{contents}"
            );
        }
    }

    #[test]
    fn malformed_name_fields_remain_unknown_even_in_inert_entries() {
        let (temp, skill, config) = fixture();
        for contents in [
            "[[skills.config]]\nname=4\nenabled=false",
            "[[skills.config]]\nname=[]\npath='unused'\nenabled=false",
            "[[skills.config]]\nname='example'",
            "[[skills.config]]\nname='example'\npath=4\nenabled=false",
        ] {
            fs::write(&config, contents).unwrap();
            assert_eq!(
                snapshot(temp.path()).visibility_for_entrypoint(&skill, "example"),
                CatalogVisibility::Unknown
            );
        }
    }

    #[test]
    fn unknown_name_only_matters_until_a_later_matching_path_decides() {
        let (temp, skill, config) = fixture();
        fs::write(&config, "[[skills.config]]\nname='example'\nenabled=false").unwrap();
        assert_eq!(
            snapshot(temp.path()).scoped_visibility(&skill, None),
            CatalogVisibility::Unknown
        );
        fs::write(&config, "[[skills.config]]\nname='example'\nenabled=false\n[[skills.config]]\npath='../.agents/skills/example/SKILL.md'\nenabled=true").unwrap();
        assert_eq!(
            snapshot(temp.path()).scoped_visibility(&skill, None),
            CatalogVisibility::On
        );
    }

    #[test]
    fn skill_rules_do_not_claim_whole_host_configuration_validity() {
        let (temp, skill, config) = fixture();
        fs::write(
            &config,
            "model=4\n[[skills.config]]\nname='example'\nenabled=false",
        )
        .unwrap();
        let observed = snapshot(temp.path());
        assert_eq!(
            observed.visibility_for_entrypoint(&skill, "example"),
            CatalogVisibility::Off
        );
        assert_eq!(observed.summary()["skill_configuration_known"], true);
        assert_eq!(observed.summary()["host_config_validity"], "unverified");
    }

    #[cfg(unix)]
    #[test]
    fn alias_path_and_name_order_and_retarget_are_conservative() {
        let (temp, skill, config) = fixture();
        let alias = temp.path().join(".agents/skills/alias");
        std::os::unix::fs::symlink(skill.parent().unwrap(), &alias).unwrap();
        fs::write(&config, "[[skills.config]]\npath='../.agents/skills/alias/SKILL.md'\nenabled=false\n[[skills.config]]\nname='example'\nenabled=true").unwrap();
        let observed = snapshot(temp.path());
        assert_eq!(
            observed.visibility_for_entrypoint(&skill, "example"),
            CatalogVisibility::On
        );
        let other = temp.path().join("other/SKILL.md");
        assert_eq!(
            observed.visibility_for_retargeted_entrypoint(&skill, &other),
            CatalogVisibility::Unknown
        );
        fs::write(&config, "[[skills.config]]\nname='example'\nenabled=false").unwrap();
        assert_eq!(
            snapshot(temp.path()).visibility_for_retargeted_entrypoint(&skill, &other),
            CatalogVisibility::Off
        );
    }
    #[test]
    fn unknown_name_does_not_erase_a_decision_it_cannot_change() {
        let (temp, skill, config) = fixture();
        fs::write(&config, "[[skills.config]]\nname='example'\nenabled=true").unwrap();
        assert_eq!(
            snapshot(temp.path()).scoped_visibility(&skill, None),
            CatalogVisibility::On
        );
        fs::write(&config, "[[skills.config]]\npath='../.agents/skills/example/SKILL.md'\nenabled=false\n[[skills.config]]\nname='example'\nenabled=false").unwrap();
        assert_eq!(
            snapshot(temp.path()).scoped_visibility(&skill, None),
            CatalogVisibility::Off
        );
    }
    #[test]
    fn source_identity_dedup_does_not_merge_native_placement_names() {
        let temp = TempDir::new().unwrap();
        for (root, name) in [
            (".codex/skills", "legacy-user"),
            (".agents/skills", "native-user"),
        ] {
            let path = temp.path().join(root).join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("SKILL.md"), format!("---\nname: {name}\ndescription: fixture\nsource: fixture/shared\nversion: '1.0'\n---\nfixture")).unwrap();
        }
        let config = temp.path().join(".codex/config.toml");
        for disabled in ["native-user", "legacy-user"] {
            fs::write(
                &config,
                format!("[[skills.config]]\nname='{disabled}'\nenabled=false"),
            )
            .unwrap();
            let mut options = crate::scan::ScanOptions::for_home(temp.path());
            options.include_session_evidence = false;
            let result = crate::scan::scan(&options).unwrap();
            assert_eq!(result.skills.len(), 1);
            for placement in &result.placements {
                let name = result.verified_entrypoint_names[&placement.id]
                    .as_deref()
                    .unwrap();
                let expected = if name == disabled {
                    CatalogVisibility::Off
                } else {
                    CatalogVisibility::On
                };
                assert_eq!(
                    result.codex_visibility.as_ref().unwrap().placements[&placement.id],
                    expected
                );
                let visibility = result.codex_visibility.as_ref().unwrap();
                assert_eq!(
                    visibility.visibility_for_core_source_entrypoint(&placement.entrypoint),
                    expected
                );
                assert_eq!(
                    visibility.visibility_for_core_target_entrypoint(
                        &temp.path().join(".codex/skills/proposed/SKILL.md"),
                        &placement.entrypoint
                    ),
                    expected
                );
            }
        }
    }

    #[test]
    fn bounded_names_handle_comments_and_reject_unsupported_yaml() {
        for line in [
            "name: native-user",
            "name: native-user # fixture",
            "name: \"native-user\" # fixture",
            "name: 'native-user' # fixture",
            "name: \" native-user \"",
            "name: ' native-user ' # fixture",
        ] {
            assert_eq!(
                parse_native_name(&format!("---\n{line}\n---\nbody")),
                Some("native-user".into())
            );
        }
        for line in [
            "name: [native-user]",
            "name: |\n  native-user",
            "name: native-user\nname: other",
            "name: \"escaped\\name\"",
            "name: true",
        ] {
            assert_eq!(parse_native_name(&format!("---\n{line}\n---\nbody")), None);
        }
    }

    #[test]
    fn updated_content_uses_its_own_native_name() {
        let (temp, skill, config) = fixture();
        fs::write(
            &config,
            "[[skills.config]]\nname='replacement'\nenabled=false",
        )
        .unwrap();
        let observed = snapshot(temp.path());
        assert_eq!(
            observed.visibility_for_updated_entrypoint(
                &skill,
                "---\nname: replacement # native name\n---\nbody"
            ),
            CatalogVisibility::Off
        );
        assert_eq!(
            observed
                .visibility_for_updated_entrypoint(&skill, "---\nname: [unsupported]\n---\nbody"),
            CatalogVisibility::Unknown
        );
    }
    #[test]
    fn name_continuations_are_unknown_without_affecting_other_field_blocks() {
        assert_eq!(
            parse_native_name("---\nname: native-user\n  suffix\n---\nbody"),
            None
        );
        assert_eq!(
            parse_native_name("---\nname: native-user\n\n  suffix\n---\nbody"),
            None
        );
        assert_eq!(
            parse_native_name(
                "---\nname: native-user\ndescription: |\n  detailed fixture\n---\nbody"
            ),
            Some("native-user".into())
        );
        assert_eq!(
            parse_native_name(
                "---\ndescription: |\n  detailed fixture\nname: native-user\n---\nbody"
            ),
            Some("native-user".into())
        );
        let (temp, skill, config) = fixture();
        fs::write(
            &config,
            "[[skills.config]]\nname='native-user suffix'\nenabled=false",
        )
        .unwrap();
        assert_eq!(
            snapshot(temp.path()).visibility_for_updated_entrypoint(
                &skill,
                "---\nname: native-user\n  suffix\n---\nbody"
            ),
            CatalogVisibility::Unknown
        );
    }
}
