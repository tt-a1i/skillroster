//! Supplied-Home Hermes user-Skill local rules, never a session catalog.
//! Only verified entrypoint attributes and typed Skill controls are retained.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use yaml_serde::{Mapping, Value as Yaml};

use crate::claude_visibility::{CatalogVisibility, SettingsObservation, read_settings};
use crate::harness::AgentKind;
use crate::scan::ScanResult;

const VERSION: &str = "hermes-user-skill-local-rules-v3";
const BASIS: &str = "supplied_home_user_skill_local_file_rules_not_session_catalog";
const CONDITION_KEYS: &[&str] = &[
    "fallback_for_toolsets",
    "requires_toolsets",
    "fallback_for_tools",
    "requires_tools",
    "session_platforms",
];

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct EntrypointAttributes {
    // Outer None is unknown; inner None is a verified absent declared name.
    name: Option<Option<String>>,
    // Empty means no OS restriction, None means unsupported syntax.
    platforms: Option<Vec<String>>,
    runtime_conditions: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Rules {
    disabled: Option<Vec<String>>,
    channel_disabled: Option<Vec<String>>,
    additional_roots: bool,
}

impl Rules {
    fn empty() -> Self {
        Self {
            disabled: Some(Vec::new()),
            channel_disabled: Some(Vec::new()),
            additional_roots: false,
        }
    }
}

/// Object identity excludes directory ctime/mtime: adding a child changes those
/// timestamps without replacing the root. Birth time guards inode reuse where
/// the filesystem exposes it; Windows file identity already includes creation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum DirectoryIdentity {
    Posix {
        dev: u64,
        ino: u64,
        #[serde(default)]
        birth_time: Option<(u64, u32)>,
    },
    Windows {
        volume_serial: u32,
        file_index_high: u32,
        file_index_low: u32,
        creation_time_high: u32,
        creation_time_low: u32,
    },
    Unavailable,
}

impl DirectoryIdentity {
    fn capture(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::metadata(path)?;
        if !metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "root is not a directory",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self::Posix {
                dev: metadata.dev(),
                ino: metadata.ino(),
                birth_time: metadata
                    .created()
                    .ok()
                    .and_then(|created| created.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|created| (created.as_secs(), created.subsec_nanos())),
            })
        }
        #[cfg(windows)]
        {
            use crate::source_policy::RootIdentity;
            Ok(match crate::source_policy::capture_identity(path)? {
                RootIdentity::Windows {
                    volume_serial,
                    file_index_high,
                    file_index_low,
                    creation_time_high,
                    creation_time_low,
                } => Self::Windows {
                    volume_serial,
                    file_index_high,
                    file_index_low,
                    creation_time_high,
                    creation_time_low,
                },
                _ => Self::Unavailable,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(Self::Unavailable)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum RootBinding {
    Resolved {
        path: PathBuf,
        identity: DirectoryIdentity,
    },
    Missing(PathBuf),
    Unavailable,
}

impl RootBinding {
    fn observe(path: &Path) -> Self {
        match fs::canonicalize(path) {
            Ok(path) => match DirectoryIdentity::capture(&path) {
                Ok(identity) => Self::Resolved { path, identity },
                Err(_) => Self::Unavailable,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                for ancestor in path.ancestors().skip(1) {
                    match fs::canonicalize(ancestor) {
                        Ok(resolved) => {
                            return Self::Missing(
                                resolved.join(path.strip_prefix(ancestor).expect("ancestor")),
                            );
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => break,
                    }
                }
                Self::Unavailable
            }
            Err(_) => Self::Unavailable,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisibilitySnapshot {
    pub version: String,
    pub basis: String,
    pub settings_files: BTreeMap<PathBuf, SettingsObservation>,
    pub placements: BTreeMap<String, CatalogVisibility>,
    #[serde(default)]
    pub host_platform: Option<String>,
    user_root: PathBuf,
    rules: Option<Rules>,
    verified_attributes: BTreeMap<PathBuf, EntrypointAttributes>,
    #[serde(default)]
    root_identities: Option<BTreeMap<PathBuf, RootBinding>>,
}

impl VisibilitySnapshot {
    pub(crate) fn visibility_for_core_target_entrypoint(
        &self,
        target: &Path,
        source: &Path,
    ) -> CatalogVisibility {
        self.visibility_for_replacement_source(target, source)
    }

    pub(crate) fn visibility_for_updated_entrypoint(
        &self,
        entrypoint: &Path,
        content: &str,
    ) -> CatalogVisibility {
        self.evaluate(entrypoint, &parse_entrypoint_attributes(content))
    }

    pub(crate) fn visibility_for_preserved_entrypoint(
        &self,
        entrypoint: &Path,
    ) -> CatalogVisibility {
        self.visibility_for_replacement_source(entrypoint, entrypoint)
    }

    pub(crate) fn visibility_for_replacement_source(
        &self,
        entrypoint: &Path,
        source: &Path,
    ) -> CatalogVisibility {
        let Some(attributes) = fs::canonicalize(source)
            .ok()
            .and_then(|identity| self.verified_attributes.get(&identity))
        else {
            return CatalogVisibility::Unknown;
        };
        self.evaluate(entrypoint, attributes)
    }

    fn evaluate(&self, entrypoint: &Path, attributes: &EntrypointAttributes) -> CatalogVisibility {
        let Ok(relative) = entrypoint.strip_prefix(&self.user_root) else {
            return CatalogVisibility::Unknown;
        };
        if !entrypoint.is_absolute()
            || entrypoint
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            || relative.components().any(|part| part.as_os_str() == "_org")
        {
            return CatalogVisibility::Unknown;
        }
        let Some(dirname) = entrypoint
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
        else {
            return CatalogVisibility::Unknown;
        };
        let Some(declared) = &attributes.name else {
            return CatalogVisibility::Unknown;
        };
        let name = declared.as_deref().unwrap_or(dirname);
        if !ordinary_name(name) {
            return CatalogVisibility::Unknown;
        }
        let Some(rules) = &self.rules else {
            return CatalogVisibility::Unknown;
        };
        let Some(disabled) = &rules.disabled else {
            return CatalogVisibility::Unknown;
        };
        if disabled.iter().any(|selector| selector == name) {
            return if name == "hermes-agent" {
                CatalogVisibility::Unknown
            } else {
                CatalogVisibility::Off
            };
        }
        let ambiguous_selector = |selector: &String| {
            !ordinary_name(selector) || (selector == dirname && dirname != name)
        };
        if disabled.iter().any(ambiguous_selector) || rules.additional_roots {
            return CatalogVisibility::Unknown;
        }
        let Some(platforms) = &attributes.platforms else {
            return CatalogVisibility::Unknown;
        };
        let Some(host) = self.host_platform.as_deref().and_then(normalize_platform) else {
            return CatalogVisibility::Unknown;
        };
        if !platforms.is_empty() && !platforms.iter().any(|platform| platform == host) {
            return CatalogVisibility::Off;
        }
        let Some(channel_disabled) = &rules.channel_disabled else {
            return CatalogVisibility::Unknown;
        };
        if channel_disabled
            .iter()
            .any(|selector| selector == name || ambiguous_selector(selector))
            || !attributes.runtime_conditions.is_empty()
        {
            return CatalogVisibility::Unknown;
        }
        CatalogVisibility::On
    }

    pub(crate) fn validate_contract(&self) -> Result<(), &'static str> {
        if self.version != VERSION || self.host_platform.is_none() || self.root_identities.is_none()
        {
            return Err("legacy_hermes_visibility_requires_rescan");
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        self.validate_contract()?;
        if self.host_platform.as_deref() != Some(std::env::consts::OS) {
            return Err("hermes_visibility_host_platform_changed");
        }
        if self
            .root_identities
            .as_ref()
            .expect("checked")
            .iter()
            .any(|(path, identity)| RootBinding::observe(path) != *identity)
        {
            return Err("hermes_visibility_root_identity_changed");
        }
        if self.settings_files.iter().any(|(path, expected)| {
            SettingsObservation::from_read(&read_settings(path)) != *expected
        }) {
            return Err("hermes_visibility_settings_changed");
        }
        Ok(())
    }

    pub(crate) fn summary(&self) -> Value {
        let mut states = BTreeMap::<String, usize>::new();
        for visibility in self.placements.values() {
            let name = serde_json::to_value(visibility)
                .expect("enum serializes")
                .as_str()
                .expect("enum string")
                .to_owned();
            *states.entry(name).or_default() += 1;
        }
        let mut unsupported = self
            .verified_attributes
            .values()
            .flat_map(|attributes| attributes.runtime_conditions.iter().cloned())
            .collect::<BTreeSet<_>>();
        unsupported.extend(
            [
                "session_channel",
                "project_and_external_root_precedence",
                "effective_host_configuration",
            ]
            .map(str::to_owned),
        );
        json!({
            "agent": "hermes", "basis": self.basis,
            "placement_count": self.placements.len(), "state_counts": states,
            "unknown_count": self.placements.values().filter(|visibility| **visibility == CatalogVisibility::Unknown).count(),
            "settings_file_count": self.settings_files.values().filter(|observation| matches!(observation, SettingsObservation::Digest(_))).count(),
            "skill_configuration_known": self.rules.as_ref().is_some_and(|rules| rules.disabled.is_some() && rules.channel_disabled.is_some()),
            "additional_roots_configured": self.rules.as_ref().is_some_and(|rules| rules.additional_roots),
            "host_platform": self.host_platform, "host_config_validity": "unverified", "session_catalog_observed": false,
            "unsupported_runtime_conditions": unsupported,
        })
    }
}

pub(crate) fn observe(home: &Path, scan: &mut ScanResult) {
    let config = home.join(".hermes/config.yaml");
    let read = read_settings(&config);
    let mut snapshot = VisibilitySnapshot {
        version: VERSION.into(),
        basis: BASIS.into(),
        settings_files: BTreeMap::from([(config, SettingsObservation::from_read(&read))]),
        placements: BTreeMap::new(),
        host_platform: Some(std::env::consts::OS.into()),
        user_root: home.join(".hermes/skills"),
        rules: match read {
            Ok(None) => Some(Rules::empty()),
            Ok(Some(bytes)) => parse_config(&bytes),
            Err(_) => None,
        },
        verified_attributes: BTreeMap::new(),
        // Local Hermes rules have one supplied-Home user root. Other Agent
        // roots and Library bookkeeping do not change this projection's scope;
        // their package bytes retain existing content and Plan validation.
        root_identities: Some(BTreeMap::from([(
            home.join(".hermes/skills"),
            RootBinding::observe(&home.join(".hermes/skills")),
        )])),
    };
    for placement in &scan.placements {
        if placement.entrypoint_digest.is_some()
            && let Some((identity, attributes)) = scan.verified_hermes_attributes.get(&placement.id)
            && fs::canonicalize(&placement.entrypoint).ok().as_ref() == Some(identity)
        {
            snapshot
                .verified_attributes
                .entry(identity.clone())
                .and_modify(|existing| {
                    if existing != attributes {
                        *existing = EntrypointAttributes::default();
                    }
                })
                .or_insert_with(|| attributes.clone());
        }
    }
    for placement in scan
        .placements
        .iter_mut()
        .filter(|placement| placement.agent == Some(AgentKind::Hermes))
    {
        let visibility = if !placement.default_exposed {
            CatalogVisibility::NotDiscovered
        } else {
            scan.verified_hermes_attributes
                .get(&placement.id)
                .filter(|(identity, _)| {
                    fs::canonicalize(&placement.entrypoint).ok().as_ref() == Some(identity)
                })
                .map_or(CatalogVisibility::Unknown, |(_, attributes)| {
                    snapshot.evaluate(&placement.entrypoint, attributes)
                })
        };
        placement.default_exposed = matches!(
            visibility,
            CatalogVisibility::On | CatalogVisibility::Unknown
        );
        snapshot.placements.insert(placement.id.clone(), visibility);
    }
    scan.hermes_visibility = Some(snapshot);
    scan.verified_hermes_attributes.clear();
}

fn normalize_platform(platform: &str) -> Option<&'static str> {
    match platform.trim().to_ascii_lowercase().as_str() {
        "macos" | "darwin" => Some("macos"),
        "linux" => Some("linux"),
        "windows" | "win32" => Some("windows"),
        _ => None,
    }
}

fn bool_like(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "y" | "yes" | "n" | "no" | "on" | "off" | "true" | "false" | "null" | "~"
    )
}

fn ordinary_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= 64
        && !name.contains(['/', '\\', ':', '[', ']', '{', '}'])
        && !name.chars().any(char::is_control)
        && !bool_like(name)
}

fn yaml11_date_like(value: &str) -> bool {
    // CSafeLoader resolves a zero-padded YYYY-MM-DD scalar as a date.
    // Serde's YAML value loses quoting/anchor style, so quoted date names are
    // conservatively unsupported too. Timestamp forms contain ':' and already
    // fail the ordinary-name contract; bare non-padded dates remain strings.
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn string_list(value: Option<&Yaml>) -> Option<Vec<String>> {
    match value {
        None | Some(Yaml::Null) => Some(Vec::new()),
        Some(Yaml::String(value))
            if !value.trim().starts_with('[')
                && !bool_like(value.trim())
                && !yaml11_date_like(value.trim()) =>
        {
            Some(vec![value.trim().into()])
        }
        Some(Yaml::Sequence(values)) => values
            .iter()
            .map(|value| match value {
                Yaml::String(value)
                    if !bool_like(value.trim()) && !yaml11_date_like(value.trim()) =>
                {
                    Some(value.trim().to_owned())
                }
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

// The input file is bounded independently by read_settings. These budgets
// additionally bound alias expansion before allocating strings or containers.
#[derive(Default)]
struct YamlBudget {
    nodes: usize,
    scalar_bytes: usize,
}

struct YamlSeed<'a> {
    budget: &'a mut YamlBudget,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for YamlSeed<'_> {
    type Value = Yaml;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Yaml, D::Error> {
        self.budget.nodes += 1;
        if self.budget.nodes > 8192 || self.depth > 64 {
            return Err(de::Error::custom("YAML expansion budget exceeded"));
        }
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for YamlSeed<'_> {
    type Value = Yaml;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded YAML with unique string keys and no tags or merges")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Yaml, E> {
        self.budget.scalar_bytes += value.len();
        if self.budget.scalar_bytes > 1024 * 1024 {
            return Err(E::custom("YAML scalar expansion budget exceeded"));
        }
        Ok(Yaml::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Yaml, E> {
        self.budget.scalar_bytes += value.len();
        if self.budget.scalar_bytes > 1024 * 1024 {
            return Err(E::custom("YAML scalar expansion budget exceeded"));
        }
        Ok(Yaml::String(value))
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Yaml, E> {
        Ok(Yaml::Bool(value))
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Yaml, E> {
        Ok(Yaml::from(value))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Yaml, E> {
        Ok(Yaml::from(value))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Yaml, E> {
        Ok(Yaml::from(value))
    }
    fn visit_unit<E: de::Error>(self) -> Result<Yaml, E> {
        Ok(Yaml::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Yaml, E> {
        Ok(Yaml::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Yaml, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(YamlSeed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(Yaml::Sequence(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut source: A) -> Result<Yaml, A::Error> {
        let mut values = Mapping::new();
        while let Some(key) = source.next_key_seed(YamlSeed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            if !matches!(key, Yaml::String(_))
                || key.as_str() == Some("<<")
                || values.contains_key(&key)
            {
                return Err(de::Error::custom("unsupported or repeated YAML key"));
            }
            let value = source.next_value_seed(YamlSeed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(Yaml::Mapping(values))
    }
}

fn yaml_mapping(text: &str) -> Option<Mapping> {
    let value = YamlSeed {
        budget: &mut YamlBudget::default(),
        depth: 0,
    }
    .deserialize(yaml_serde::Deserializer::from_str(text))
    .ok()?;
    match value {
        Yaml::Mapping(map) => Some(map),
        _ => None,
    }
}

fn enabled_extra_roots(value: Option<&Yaml>) -> bool {
    match value {
        None | Some(Yaml::Null) | Some(Yaml::Bool(false)) => false,
        Some(Yaml::String(value)) => !value.trim().is_empty(),
        // Native Hermes stringifies every list member before resolving a path:
        // even a number, null, or collection can name an existing directory.
        Some(Yaml::Sequence(values)) => values.iter().any(|value| match value {
            Yaml::String(value) => !value.trim().is_empty(),
            _ => true,
        }),
        _ => false,
    }
}

fn parse_config(bytes: &[u8]) -> Option<Rules> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.starts_with('\u{feff}') {
        return None;
    }
    let config = yaml_mapping(text)?;
    let mut rules = Rules::empty();
    let Some(skills) = config.get("skills") else {
        return Some(rules);
    };
    let skills = skills.as_mapping()?;
    rules.disabled = string_list(skills.get("disabled"))
        .map(|names| names.into_iter().filter(|name| !name.is_empty()).collect());
    rules.channel_disabled = match skills.get("platform_disabled") {
        None | Some(Yaml::Null) => Some(Vec::new()),
        Some(Yaml::Mapping(channels)) => {
            channels
                .iter()
                .try_fold(Vec::new(), |mut names, (channel, values)| {
                    channel.as_str()?;
                    names.extend(
                        string_list(Some(values))?
                            .into_iter()
                            .filter(|name| !name.is_empty()),
                    );
                    Some(names)
                })
        }
        _ => None,
    };
    rules.additional_roots = enabled_extra_roots(skills.get("external_dirs"))
        || enabled_extra_roots(skills.get("create_dir"));
    Some(rules)
}

pub(crate) fn parse_entrypoint_attributes(content: &str) -> EntrypointAttributes {
    let prefix: String = content.chars().take(4000).collect();
    let prefix = prefix.strip_prefix('\u{feff}').unwrap_or(&prefix);
    // List decoding and prompt parsing remove different numbers of BOMs.
    if prefix.starts_with('\u{feff}') {
        return EntrypointAttributes::default();
    }
    if !prefix.starts_with("---") {
        // A whitespace-prefixed fence can differ between native surfaces;
        // never mistake its control fields for a verified absent declaration.
        if prefix.trim_start().starts_with("---") {
            return EntrypointAttributes::default();
        }
        return EntrypointAttributes {
            name: Some(None),
            platforms: Some(Vec::new()),
            runtime_conditions: BTreeSet::new(),
        };
    }
    let Some(start) = prefix.find('\n') else {
        return EntrypointAttributes::default();
    };
    if prefix[..start].trim_end_matches('\r') != "---" {
        return EntrypointAttributes::default();
    }
    let rest = &prefix[start + 1..];
    let mut end = None;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.ends_with('\n') && line.trim_end_matches(['\r', '\n']) == "---" {
            end = Some(offset);
            break;
        }
        offset += line.len();
    }
    let Some(end) = end else {
        return EntrypointAttributes::default();
    };
    let text = &rest[..end];
    let frontmatter = if text.trim().is_empty() {
        Mapping::new()
    } else {
        let Some(mapping) = yaml_mapping(text) else {
            return EntrypointAttributes::default();
        };
        mapping
    };
    let name = match frontmatter.get("name") {
        None => Some(None),
        Some(Yaml::String(name)) if ordinary_name(name) && !yaml11_date_like(name) => {
            Some(Some(name.clone()))
        }
        _ => None,
    };
    let platforms = string_list(frontmatter.get("platforms")).and_then(|platforms| {
        platforms
            .iter()
            .map(|platform| normalize_platform(platform).map(str::to_owned))
            .collect()
    });
    let mut runtime_conditions = BTreeSet::new();
    for key in ["environments", "requires_apps"] {
        if frontmatter.contains_key(key) {
            runtime_conditions.insert(key.to_owned());
        }
    }
    if let Some(metadata) = frontmatter.get("metadata") {
        match metadata.as_mapping() {
            Some(metadata) => {
                if let Some(hermes) = metadata.get("hermes") {
                    match hermes.as_mapping() {
                        Some(hermes) => {
                            for key in CONDITION_KEYS {
                                if hermes.contains_key(*key) {
                                    runtime_conditions.insert(format!("metadata.hermes.{key}"));
                                }
                            }
                        }
                        None => {
                            runtime_conditions.insert("malformed_metadata.hermes".into());
                        }
                    }
                }
            }
            None => {
                runtime_conditions.insert("malformed_metadata".into());
            }
        }
    }
    EntrypointAttributes {
        name,
        platforms,
        runtime_conditions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{ExplicitSkillRoot, ScanOptions, scan};
    use tempfile::TempDir;

    fn fixture(
        config: Option<&str>,
        metadata: &str,
        dirname: &str,
    ) -> (TempDir, ScanResult, PathBuf) {
        let home = TempDir::new().unwrap();
        let entrypoint = home
            .path()
            .join(".hermes/skills")
            .join(dirname)
            .join("SKILL.md");
        fs::create_dir_all(entrypoint.parent().unwrap()).unwrap();
        fs::write(&entrypoint, format!("---\n{metadata}\n---\nFixture body\n")).unwrap();
        if let Some(config) = config {
            fs::write(home.path().join(".hermes/config.yaml"), config).unwrap();
        }
        let mut options = ScanOptions::for_home(home.path());
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();
        (home, result, entrypoint)
    }

    fn projected(result: &ScanResult, entrypoint: &Path) -> CatalogVisibility {
        let placement = result
            .placements
            .iter()
            .find(|placement| placement.entrypoint == entrypoint)
            .unwrap();
        result.hermes_visibility.as_ref().unwrap().placements[&placement.id]
    }

    #[test]
    fn global_rules_match_verified_names_and_preserve_unknown_surface_differences() {
        for (config, metadata, expected) in [
            (
                "skills:\n  disabled: [research]",
                "name: research",
                CatalogVisibility::Off,
            ),
            (
                "skills:\n  disabled: research",
                "name: research",
                CatalogVisibility::Off,
            ),
            (
                "skills:\n  disabled: [Research]",
                "name: research",
                CatalogVisibility::On,
            ),
            (
                "skills:\n  disabled: [directory]",
                "name: research",
                CatalogVisibility::Unknown,
            ),
            (
                "skills:\n  disabled: [directory]",
                "description: unnamed",
                CatalogVisibility::Off,
            ),
            (
                "skills:\n  disabled: '[\"research\"]'",
                "name: research",
                CatalogVisibility::Unknown,
            ),
            (
                "skills:\n  disabled: [category/research]",
                "name: research",
                CatalogVisibility::Unknown,
            ),
            (
                "skills:\n  disabled: [hermes-agent]",
                "name: hermes-agent",
                CatalogVisibility::Unknown,
            ),
            (
                "skills:\n  disabled: [yes]",
                "name: research",
                CatalogVisibility::Unknown,
            ),
            (
                "skills:\n  disabled: [research, 7]",
                "name: research",
                CatalogVisibility::Unknown,
            ),
            (
                "skills: false",
                "name: research",
                CatalogVisibility::Unknown,
            ),
            ("skills: [", "name: research", CatalogVisibility::Unknown),
            ("false", "name: research", CatalogVisibility::Unknown),
            ("{}", "name: research", CatalogVisibility::On),
        ] {
            let (_home, result, entrypoint) = fixture(Some(config), metadata, "directory");
            assert_eq!(
                projected(&result, &entrypoint),
                expected,
                "{config}; {metadata}"
            );
        }
    }

    #[test]
    fn channel_rules_do_not_guess_a_session_and_global_off_has_priority() {
        for (config, expected) in [
            (
                "skills:\n  platform_disabled:\n    telegram: [research]",
                CatalogVisibility::Unknown,
            ),
            (
                "skills:\n  platform_disabled:\n    cli: [other]",
                CatalogVisibility::On,
            ),
            (
                "skills:\n  platform_disabled: true",
                CatalogVisibility::Unknown,
            ),
            (
                "skills:\n  disabled: [research]\n  platform_disabled: true",
                CatalogVisibility::Off,
            ),
            (
                "skills:\n  disabled: research\n  platform_disabled:\n    telegram: []",
                CatalogVisibility::Off,
            ),
        ] {
            let (_home, result, entrypoint) = fixture(Some(config), "name: research", "research");
            assert_eq!(projected(&result, &entrypoint), expected, "{config}");
        }
    }

    #[test]
    fn host_os_rules_are_typed_and_runtime_conditions_remain_unknown() {
        let opposite = if std::env::consts::OS == "windows" {
            "linux"
        } else {
            "windows"
        };
        let (_home, result, entrypoint) = fixture(
            None,
            &format!("name: research\nplatforms: [{opposite}]"),
            "research",
        );
        assert_eq!(projected(&result, &entrypoint), CatalogVisibility::Off);
        for metadata in [
            "name: research\nplatforms: false",
            "name: research\nplatforms: [android]",
            "name: research\nplatforms: [linux, 3]",
            "name: research\nplatforms: ['']",
            "name: research\nenvironments: [docker]",
            "name: research\nrequires_apps: [calendar]",
            "name: research\nmetadata:\n  hermes:\n    session_platforms: [cli]",
            "name: research\nmetadata:\n  hermes:\n    requires_tools: [read_file]",
            "name: research\nmetadata:\n  hermes:\n    fallback_for_toolsets: [browser]",
        ] {
            let (_home, result, entrypoint) = fixture(None, metadata, "research");
            assert_eq!(
                projected(&result, &entrypoint),
                CatalogVisibility::Unknown,
                "{metadata}"
            );
            let (_home, result, entrypoint) = fixture(
                Some("skills:\n  disabled: [research]"),
                metadata,
                "research",
            );
            assert_eq!(
                projected(&result, &entrypoint),
                CatalogVisibility::Off,
                "{metadata}"
            );
        }
        let (_home, result, entrypoint) = fixture(
            None,
            "name: research\nmetadata:\n  hermes:\n    version: '1.0'\n    author: local",
            "research",
        );
        assert_eq!(projected(&result, &entrypoint), CatalogVisibility::On);
    }

    #[test]
    fn metadata_is_bound_to_each_verified_placement_before_declared_identity_deduplication() {
        let (home, _, first) = fixture(
            Some("skills:\n  disabled: [alpha]"),
            "name: alpha\nsource: common\nversion: '1'",
            "first",
        );
        let second = home.path().join(".hermes/skills/second/SKILL.md");
        fs::create_dir_all(second.parent().unwrap()).unwrap();
        fs::write(
            &second,
            "---\nname: beta\nsource: common\nversion: '1'\n---\n",
        )
        .unwrap();
        let mut options = ScanOptions::for_home(home.path());
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(projected(&result, &first), CatalogVisibility::Off);
        assert_eq!(projected(&result, &second), CatalogVisibility::On);
        assert!(result.verified_hermes_attributes.is_empty());
    }

    #[test]
    fn core_and_source_update_apis_use_verified_source_metadata_and_target_logical_name() {
        let (home, result, source) = fixture(
            Some("skills:\n  disabled: [future]"),
            "description: no declared name",
            "original",
        );
        let snapshot = result.hermes_visibility.as_ref().unwrap();
        let target = home.path().join(".hermes/skills/future/SKILL.md");
        assert!(!target.exists());
        assert_eq!(
            snapshot.visibility_for_core_target_entrypoint(&target, &source),
            CatalogVisibility::Off
        );
        assert_eq!(
            snapshot.visibility_for_replacement_source(&target, &source),
            CatalogVisibility::Off
        );
        assert_eq!(
            snapshot.visibility_for_preserved_entrypoint(&source),
            CatalogVisibility::On
        );
        assert_eq!(
            snapshot.visibility_for_updated_entrypoint(&source, "---\nname: future\n---\n"),
            CatalogVisibility::Off
        );
        assert_eq!(
            snapshot.visibility_for_core_target_entrypoint(
                &target,
                &home.path().join("unverified/SKILL.md")
            ),
            CatalogVisibility::Unknown
        );
        assert_eq!(
            snapshot.visibility_for_updated_entrypoint(&source, "---\nname: [future]\n---\n"),
            CatalogVisibility::Unknown
        );
    }

    #[test]
    fn unsupported_roots_and_org_paths_are_unknown_while_undiscovered_support_stays_undiscovered() {
        let (home, _, _) = fixture(None, "name: research", "ordinary");
        for relative in [
            ".hermes/skills/_org/hidden",
            ".hermes/skills/ordinary/scripts/support",
            "external/other",
        ] {
            let directory = home.path().join(relative);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("SKILL.md"), "---\nname: research\n---\n").unwrap();
        }
        let mut options = ScanOptions::for_home(home.path());
        options.include_session_evidence = false;
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Hermes,
            path: home.path().join("external"),
        });
        let result = scan(&options).unwrap();
        for relative in [
            ".hermes/skills/_org/hidden/SKILL.md",
            "external/other/SKILL.md",
        ] {
            assert_eq!(
                projected(&result, &home.path().join(relative)),
                CatalogVisibility::Unknown
            );
        }
        assert_eq!(
            projected(
                &result,
                &home
                    .path()
                    .join(".hermes/skills/ordinary/scripts/support/SKILL.md")
            ),
            CatalogVisibility::NotDiscovered
        );
    }

    #[test]
    fn bounded_yaml_accepts_simple_anchors_and_unrelated_special_characters() {
        let config = "url: 'https://example.test/search?q=*&token=SECRET'\nnames: &names [research]\nskills:\n  disabled: *names\n";
        let (_home, result, entrypoint) = fixture(Some(config), "name: research", "research");
        assert_eq!(projected(&result, &entrypoint), CatalogVisibility::Off);
        let serialized = serde_json::to_string(result.hermes_visibility.as_ref().unwrap()).unwrap();
        assert!(!serialized.contains("SECRET"));
        assert!(!serialized.contains("example.test"));
        for config in [
            "base: &base {disabled: [research]}\nskills:\n  <<: *base\n",
            "skills:\n  disabled: !custom research\n",
            "skills: {}\nskills: {disabled: [research]}\n",
            "? [skills, other]\n: {}\n",
            "recursive: &recursive [*recursive]\nskills: {}\n",
        ] {
            assert!(parse_config(config.as_bytes()).is_none(), "{config}");
        }
    }

    #[test]
    fn yaml_alias_scalar_node_and_depth_expansion_are_bounded_before_building_values() {
        let large_scalar = "x".repeat(16 * 1024);
        let config = format!(
            "scalar: &scalar '{large_scalar}'\nexpanded: [{}]\nskills: {{}}",
            vec!["*scalar"; 70].join(",")
        );
        assert!(config.len() < 256 * 1024);
        assert!(parse_config(config.as_bytes()).is_none());
        let nodes = format!("ignored: [{}]\nskills: {{}}", vec!["0"; 8192].join(","));
        assert!(parse_config(nodes.as_bytes()).is_none());
        let depth = format!(
            "ignored: {}0{}\nskills: {{}}",
            "[".repeat(65),
            "]".repeat(65)
        );
        assert!(parse_config(depth.as_bytes()).is_none());
    }

    #[test]
    fn frontmatter_requires_valid_name_types_and_a_complete_first_four_thousand_characters() {
        for metadata in [
            "name: yes",
            "name: false",
            "name: 7",
            "name: [research]",
            "name: ' leading'",
            "name: {broken",
            "name: !tag research",
        ] {
            assert!(
                parse_entrypoint_attributes(&format!("---\n{metadata}\n---\n"))
                    .name
                    .is_none(),
                "{metadata}"
            );
        }
        assert!(
            parse_entrypoint_attributes(&format!("---\nname: {}\n---\n", "x".repeat(65)))
                .name
                .is_none()
        );
        assert!(
            parse_entrypoint_attributes(&format!(
                "---\nname: research\ndescription: {}\n---\n",
                "x".repeat(4000)
            ))
            .name
            .is_none()
        );
        assert!(
            parse_entrypoint_attributes("---\nname: research\n---")
                .name
                .is_none()
        );
        for whitespace in ["\n", " ", "\r\n", "\u{feff}\n"] {
            assert!(
                parse_entrypoint_attributes(&format!("{whitespace}---\nname: research\n---\n"))
                    .name
                    .is_none()
            );
        }
        assert_eq!(
            parse_entrypoint_attributes("\u{feff}---\nname: research\n---\n").name,
            Some(Some("research".into()))
        );
        for bom_count in [2, 3] {
            assert!(
                parse_entrypoint_attributes(&format!(
                    "{}---\nname: research\n---\n",
                    "\u{feff}".repeat(bom_count)
                ))
                .name
                .is_none()
            );
        }
        let attributes =
            parse_entrypoint_attributes("---\nname: &name research\ndescription: *name\n---\n");
        assert_eq!(attributes.name, Some(Some("research".into())));
    }

    #[test]
    fn yaml11_dates_and_timestamps_do_not_become_known_string_names() {
        for metadata in [
            "name: 2026-10-23",
            "name: '2026-10-23'",
            "name: \"2026-10-23\"",
            "date: &date 2026-10-23\nname: *date",
            "date: &date '2026-10-23'\nname: *date",
            "name: 2026-10-23T12:34:56Z",
            "name: 2026-1-3 1:02:03Z",
            "name: '2026-10-23T12:34:56Z'",
        ] {
            let (_home, result, entrypoint) = fixture(None, metadata, "research");
            assert_eq!(
                projected(&result, &entrypoint),
                CatalogVisibility::Unknown,
                "{metadata}"
            );
            assert!(result.placements[0].default_exposed);
        }
        // PyYAML's date-only resolver requires zero-padded month and day.
        for name in ["2026-1-3", "2026-01-3", "2026-1-03", "release-2026-10-23"] {
            let (_home, result, entrypoint) = fixture(None, &format!("name: {name}"), "research");
            assert_eq!(
                projected(&result, &entrypoint),
                CatalogVisibility::On,
                "{name}"
            );
        }
        let (_home, result, entrypoint) =
            fixture(None, "description: no declared name", "2026-10-23");
        assert_eq!(projected(&result, &entrypoint), CatalogVisibility::On);
    }

    #[test]
    fn non_string_external_directory_members_remain_potential_native_roots() {
        let (home, _, entrypoint) = fixture(
            Some("skills:\n  external_dirs: [123]\n"),
            "name: research",
            "research",
        );
        let external = home.path().join(".hermes/123/external");
        fs::create_dir_all(&external).unwrap();
        fs::write(
            external.join("SKILL.md"),
            "---\nname: EXTERNAL-UNAPPROVED\n---\nSECRET-BODY\n",
        )
        .unwrap();
        let mut options = ScanOptions::for_home(home.path());
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();
        assert_eq!(projected(&result, &entrypoint), CatalogVisibility::Unknown);
        assert_eq!(result.placements.len(), 1);
        assert_eq!(
            result.hermes_visibility.as_ref().unwrap().summary()["additional_roots_configured"],
            true
        );
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains("EXTERNAL-UNAPPROVED"));
        assert!(!serialized.contains("SECRET-BODY"));
        for member in ["false", "null", "{}", "[]"] {
            assert!(
                parse_config(format!("skills:\n  external_dirs: [{member}]\n").as_bytes())
                    .unwrap()
                    .additional_roots
            );
        }
    }

    #[test]
    fn configuration_strips_one_bom_and_rejects_multiple_boms_before_parsing_rules() {
        for (count, expected) in [
            (0, CatalogVisibility::Off),
            (1, CatalogVisibility::Off),
            (2, CatalogVisibility::Unknown),
            (3, CatalogVisibility::Unknown),
        ] {
            let config = format!(
                "{}skills:\n  disabled: [research]\n",
                "\u{feff}".repeat(count)
            );
            let (_home, result, entrypoint) = fixture(Some(&config), "name: research", "research");
            assert_eq!(
                projected(&result, &entrypoint),
                expected,
                "BOM count: {count}"
            );
        }
    }

    #[test]
    fn empty_or_inert_extra_roots_do_not_make_local_rules_unknown() {
        for controls in [
            "external_dirs: []",
            "external_dirs: ''",
            "create_dir: false",
            "external_dirs: {}",
            "create_dir: 0",
        ] {
            let (_home, result, entrypoint) = fixture(
                Some(&format!("skills:\n  {controls}")),
                "name: research",
                "research",
            );
            assert_eq!(
                projected(&result, &entrypoint),
                CatalogVisibility::On,
                "{controls}"
            );
        }
        let (_home, result, entrypoint) = fixture(
            Some("skills:\n  external_dirs: [/external]"),
            "name: research",
            "research",
        );
        assert_eq!(projected(&result, &entrypoint), CatalogVisibility::Unknown);
    }

    #[test]
    fn null_disabled_channel_and_platform_controls_equal_verified_absence() {
        for config in [
            "skills:\n  disabled: null\n",
            "skills:\n  platform_disabled: null\n",
            "skills:\n  disabled: ''\n  platform_disabled:\n    cli: null\n",
        ] {
            let (_home, result, entrypoint) =
                fixture(Some(config), "name: research\nplatforms: null", "research");
            assert_eq!(
                projected(&result, &entrypoint),
                CatalogVisibility::On,
                "{config}"
            );
        }
        let (_home, result, entrypoint) =
            fixture(Some("skills: null"), "name: research", "research");
        assert_eq!(projected(&result, &entrypoint), CatalogVisibility::Unknown);
    }

    #[test]
    fn snapshot_validation_freezes_settings_absence_digest_host_and_root_identity() {
        let (home, result, _) = fixture(None, "name: research", "research");
        let mut snapshot = result.hermes_visibility.unwrap();
        assert_eq!(snapshot.validate(), Ok(()));
        let mut legacy = serde_json::to_value(&snapshot).unwrap();
        legacy["version"] = json!("hermes-user-skill-local-rules-v1");
        for binding in legacy["root_identities"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            if let Some(posix) = binding["Resolved"]["identity"]["Posix"].as_object_mut() {
                posix.remove("birth_time");
                posix.insert("ctime".into(), json!(0));
                posix.insert("ctime_nsec".into(), json!(0));
            }
        }
        let legacy: VisibilitySnapshot = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            legacy.validate_contract(),
            Err("legacy_hermes_visibility_requires_rescan")
        );
        fs::write(home.path().join(".hermes/config.yaml"), "skills: {}\n").unwrap();
        assert_eq!(
            snapshot.validate(),
            Err("hermes_visibility_settings_changed")
        );
        assert_eq!(snapshot.validate_contract(), Ok(()));
        snapshot.host_platform = Some("different-os".into());
        assert_eq!(
            snapshot.validate(),
            Err("hermes_visibility_host_platform_changed")
        );
        snapshot.host_platform = None;
        assert_eq!(
            snapshot.validate_contract(),
            Err("legacy_hermes_visibility_requires_rescan")
        );

        let (home, result, _) = fixture(Some("skills: {}\n"), "name: research", "research");
        let snapshot = result.hermes_visibility.unwrap();
        let root = home.path().join(".hermes/skills");
        fs::rename(&root, home.path().join(".hermes/old-skills")).unwrap();
        fs::create_dir(&root).unwrap();
        assert_eq!(
            snapshot.validate(),
            Err("hermes_visibility_root_identity_changed")
        );
    }

    #[test]
    fn root_content_and_unrelated_agent_bookkeeping_preserve_visibility_identity() {
        let (home, result, _) = fixture(None, "name: research", "research");
        let snapshot = result.hermes_visibility.unwrap();
        assert_eq!(snapshot.root_identities.as_ref().unwrap().len(), 1);
        let root = home.path().join(".hermes/skills");
        let marker = root.join("bookkeeping-marker");
        fs::write(&marker, "ordinary child addition\n").unwrap();
        assert_eq!(snapshot.validate(), Ok(()));
        fs::remove_file(&marker).unwrap();
        fs::set_permissions(&root, fs::metadata(&root).unwrap().permissions()).unwrap();
        assert_eq!(snapshot.validate(), Ok(()));
        for unrelated in [".codex/skills", ".skillroster/library"] {
            fs::create_dir_all(home.path().join(unrelated)).unwrap();
        }
        assert_eq!(snapshot.validate(), Ok(()));

        // A user with only Codex Skills must not inherit a Hermes dependency
        // on the Library directory's normal state-management updates.
        fs::remove_dir_all(home.path().join(".hermes")).unwrap();
        let mut options = ScanOptions::for_home(home.path());
        options.include_session_evidence = false;
        let snapshot = scan(&options).unwrap().hermes_visibility.unwrap();
        let library = home.path().join(".skillroster/library");
        fs::write(library.join("normal-bookkeeping"), "state\n").unwrap();
        fs::set_permissions(&library, fs::metadata(&library).unwrap().permissions()).unwrap();
        assert_eq!(snapshot.validate(), Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn linked_user_root_retarget_requires_rescan_even_with_identical_skill_bytes() {
        use std::os::unix::fs::symlink;
        let (home, _, _) = fixture(None, "name: research", "research");
        let root = home.path().join(".hermes/skills");
        let first = home.path().join("first-source");
        let second = home.path().join("second-source");
        fs::rename(&root, &first).unwrap();
        fs::create_dir_all(second.join("research")).unwrap();
        fs::copy(
            first.join("research/SKILL.md"),
            second.join("research/SKILL.md"),
        )
        .unwrap();
        symlink(&first, &root).unwrap();
        let mut options = ScanOptions::for_home(home.path());
        options.include_session_evidence = false;
        let snapshot = scan(&options).unwrap().hermes_visibility.unwrap();
        assert_eq!(snapshot.validate(), Ok(()));
        fs::remove_file(&root).unwrap();
        symlink(&second, &root).unwrap();
        assert_eq!(
            snapshot.validate(),
            Err("hermes_visibility_root_identity_changed")
        );
    }

    #[cfg(unix)]
    #[test]
    fn configuration_symlinks_remain_unknown_without_reading_target_content() {
        use std::os::unix::fs::symlink;
        let (home, _, entrypoint) = fixture(None, "name: research", "research");
        let external = home.path().join("external-config.yaml");
        fs::write(
            &external,
            "skills:\n  disabled: [research]\nsecret: DO-NOT-READ\n",
        )
        .unwrap();
        let config = home.path().join(".hermes/config.yaml");
        symlink(&external, &config).unwrap();
        let mut options = ScanOptions::for_home(home.path());
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();
        assert_eq!(projected(&result, &entrypoint), CatalogVisibility::Unknown);
        let snapshot = result.hermes_visibility.unwrap();
        assert_eq!(
            snapshot.settings_files[&config],
            SettingsObservation::Unavailable
        );
        assert!(
            !serde_json::to_string(&snapshot)
                .unwrap()
                .contains("DO-NOT-READ")
        );
    }
}
