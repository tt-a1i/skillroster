use crate::harness::{
    AgentKind, SessionSignal, SkillDiscoverySemantics, known_agent_roots,
    session_record_observations,
};
use aho_corasick::{AhoCorasick, AhoCorasickBuilder};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const MAX_SKILL_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SKILL_PACKAGE_BYTES: u64 = 16 * 1024 * 1024;
pub const CONTENT_IDENTITY_ALGORITHM: &str = "sha256-content-v1";
const MAX_REMOTE_PLUGIN_INSTALL_BYTES: u64 = 16 * 1024;
const MAX_SESSION_DISCOVERY_FILES_PER_ROOT: usize = 10_000;
const MAX_SESSION_FILES_PER_ROOT: usize = 250;
const MAX_SESSION_BYTES_PER_AGENT: u64 = 4 * 1024 * 1024;
const MAX_SESSION_BYTES_PER_FILE: u64 = 512 * 1024;
const MAX_SESSION_LINES_PER_AGENT: usize = 20_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanOptions {
    pub home: PathBuf,
    pub explicit_skill_roots: Vec<ExplicitSkillRoot>,
    pub explicit_source_roots: Vec<PathBuf>,
    /// Durable permissions restore factual reads only. Unlike temporary
    /// `explicit_source_roots`, they never make a placement governable.
    #[serde(default)]
    pub durable_read_roots: Vec<DurableReadRoot>,
    #[serde(default)]
    pub managed_source_roots: Vec<PathBuf>,
    pub excluded_session_agents: BTreeSet<AgentKind>,
    pub include_session_evidence: bool,
    pub max_depth: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DurableReadRoot {
    pub permission_id: String,
    pub path: PathBuf,
    pub identity: crate::source_policy::RootIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExplicitSkillRoot {
    pub agent: AgentKind,
    pub path: PathBuf,
}

impl ScanOptions {
    pub fn for_home(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            explicit_skill_roots: Vec::new(),
            explicit_source_roots: Vec::new(),
            durable_read_roots: Vec::new(),
            managed_source_roots: Vec::new(),
            excluded_session_agents: BTreeSet::new(),
            include_session_evidence: true,
            max_depth: 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootKind {
    Skills,
    Sessions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootStatus {
    Included,
    Missing,
    Inaccessible,
    Excluded,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootObservation {
    pub agent: Option<AgentKind>,
    pub kind: RootKind,
    pub path: PathBuf,
    pub status: RootStatus,
    pub explicit: bool,
    pub detail: Option<String>,
    /// Whether discovery inspected the complete configured Skill-root depth.
    /// Session roots do not use this dimension and remain complete by default.
    #[serde(default)]
    pub discovery_complete: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
    pub source: Option<String>,
    pub version: Option<String>,
    pub revision: Option<String>,
    pub triggers: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    NotLink,
    Valid,
    Broken,
    EscapesRoot,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FingerprintCompleteness {
    Complete,
    Bounded,
    Unreadable,
    #[default]
    Unknown,
}

impl FingerprintCompleteness {
    pub fn id(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Bounded => "bounded",
            Self::Unreadable => "unreadable",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillPlacement {
    pub id: String,
    pub skill_id: String,
    pub agent: Option<AgentKind>,
    pub root: PathBuf,
    pub directory: PathBuf,
    pub entrypoint: PathBuf,
    /// Resolved package directory captured during Scan. Logical Agent roots may
    /// be symlink aliases of the same physical source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_directory: Option<PathBuf>,
    pub content_digest: String,
    /// SHA-256 of the exact SKILL.md bytes observed during Scan. Missing on
    /// legacy Snapshots cannot authorize a verified load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint_digest: Option<String>,
    /// Whether `content_digest` covers the complete Skill package. Bounded or
    /// unreadable fingerprints are inventory facts only and cannot authorize
    /// exact-duplicate governance.
    #[serde(default)]
    pub fingerprint_completeness: FingerprintCompleteness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_detail: Option<String>,
    pub link_target: Option<PathBuf>,
    pub link_status: LinkStatus,
    pub default_exposed: bool,
    /// Stable structural ownership of the placement path. This does not claim
    /// ownership or endorsement of content reached through a link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by_agent: Option<bool>,
    /// The bounded mutation authority observed for this placement. Missing on
    /// legacy Snapshots means unknown and must never authorize mutation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_scope: Option<MutationScope>,
    /// Whether SkillRoster may include this placement in a mutating governance Plan.
    /// External provider caches are observed and searchable, but never governable.
    #[serde(default = "default_true")]
    pub governable: bool,
    /// External provider identity when the placement comes from a discovered plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default)]
    pub executable_files: Vec<PathBuf>,
    #[serde(default)]
    pub declared_name_matches_directory: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationScope {
    Mutable,
    ProviderReadOnly,
    DurableReadOnly,
    UntrustedExternal,
}

impl MutationScope {
    pub fn id(self) -> &'static str {
        match self {
            Self::Mutable => "mutable",
            Self::ProviderReadOnly => "provider_read_only",
            Self::DurableReadOnly => "durable_read_only",
            Self::UntrustedExternal => "untrusted_external",
        }
    }
}

impl SkillPlacement {
    pub fn is_mutable(&self) -> bool {
        self.governable && self.mutation_scope == Some(MutationScope::Mutable)
    }
}

#[derive(Debug)]
pub struct PhysicalDirectoryDrift {
    pub placement_id: String,
    pub expected: Option<PathBuf>,
    pub current: Option<PathBuf>,
}

impl std::fmt::Display for PhysicalDirectoryDrift {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Placement {} physical source drifted; run skillroster scan",
            self.placement_id
        )
    }
}

impl std::error::Error for PhysicalDirectoryDrift {}

impl SkillPlacement {
    pub fn physical_directory_or_logical(&self) -> &Path {
        self.physical_directory
            .as_deref()
            .unwrap_or(&self.directory)
    }

    pub fn validated_physical_directory(
        &self,
    ) -> std::result::Result<PathBuf, PhysicalDirectoryDrift> {
        let expected = self
            .physical_directory
            .clone()
            .ok_or_else(|| PhysicalDirectoryDrift {
                placement_id: self.id.clone(),
                expected: None,
                current: None,
            })?;
        let current = std::fs::canonicalize(&self.entrypoint)
            .ok()
            .and_then(|entrypoint| entrypoint.parent().map(Path::to_path_buf));
        if current.as_ref() != Some(&expected) {
            return Err(PhysicalDirectoryDrift {
                placement_id: self.id.clone(),
                expected: Some(expected),
                current,
            });
        }
        Ok(expected)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScannedSkill {
    pub id: String,
    pub name: String,
    pub metadata: SkillMetadata,
    pub content_digest: String,
    /// Deterministic Agent Skills payload identity. Unlike `content_digest`,
    /// this excludes documented source-control metadata and must never be used
    /// for path drift, mutation, Receipt, or Undo verification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_identity_digest: Option<String>,
    pub digest_algorithm: String,
    pub summary: String,
    /// Whitespace-normalized complete SKILL.md text used only by the local FTS index.
    #[serde(default)]
    pub normalized_text: String,
    pub modified_at_unix: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageStage {
    Exposed,
    Matched,
    Loaded,
    Applied,
    Outcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceQuality {
    Observed,
    Inferred,
    Unknown,
}

/// A deterministic reason why a session denominator is not complete.  These
/// facts are recorded at the scanner boundary rather than inferred later from
/// aggregate counters, because several limits can produce the same counters.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCoverageLimitationCode {
    RootMissing,
    RootInaccessible,
    DiscoveryFileLimit,
    DiscoveryDepthLimit,
    DiscoveryWalkFailure,
    SampledFileLimit,
    SampledByteLimit,
    SampledLineLimit,
    FileByteLimit,
    FileMetadataFailure,
    FileReadFailure,
    FileZeroRead,
    FilePathNotUnicode,
    JsonExtractionLimit,
    JsonRecordLimit,
    LineAlignmentLoss,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCoverageScope {
    Root,
    File,
    Agent,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCoverageCountKind {
    Exact,
    LowerBound,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCoverageUnit {
    Roots,
    Files,
    Depth,
    Walks,
    Bytes,
    Lines,
    Records,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionCoverageLimitationSource {
    Roots,
    SessionDiscovery,
    SessionSampling,
    SessionJson,
    SessionJsonl,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SessionCoverageLimitation {
    pub code: SessionCoverageLimitationCode,
    pub scope: SessionCoverageScope,
    pub count_kind: SessionCoverageCountKind,
    pub observed: Option<u64>,
    pub limit: Option<u64>,
    pub unit: SessionCoverageUnit,
    /// Stable scanner boundary that produced this fact. This is deliberately
    /// not a free-form error message and is safe for Agent callers to branch
    /// on.
    pub source: SessionCoverageLimitationSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageEvidence {
    pub agent: AgentKind,
    pub skill_id: String,
    pub stage: UsageStage,
    pub quality: EvidenceQuality,
    pub event_count: u64,
    pub first_seen_unix: Option<u64>,
    pub last_seen_unix: Option<u64>,
    /// UTC month containing the observed records, or `None` when their event
    /// time could not be established without guessing.
    #[serde(default)]
    pub month_start_unix: Option<u64>,
    /// A stable digest of the source path, never session content.
    pub source_path_digest: String,
}

impl UsageEvidence {
    pub fn evidence_reference(&self) -> String {
        format!(
            "usage:{}:{}:{:?}:{}:{}",
            self.agent.id(),
            self.skill_id,
            self.stage,
            self.source_path_digest,
            self.month_start_unix
                .map_or_else(|| "unknown".to_owned(), |month| month.to_string())
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionCoverage {
    pub agent: AgentKind,
    pub roots_present: usize,
    #[serde(default)]
    pub roots_missing: usize,
    #[serde(default)]
    pub roots_inaccessible: usize,
    #[serde(default)]
    pub files_discovered: usize,
    pub files_observed: usize,
    #[serde(default)]
    pub files_partially_observed: usize,
    pub files_skipped: usize,
    pub denominator_reliable: bool,
    pub bytes_observed: u64,
    pub lines_observed: usize,
    pub truncated: bool,
    #[serde(default)]
    pub discovery_truncated: bool,
    pub first_seen_unix: Option<u64>,
    pub last_seen_unix: Option<u64>,
    /// `None` means this is a legacy payload written before typed limitation
    /// facts existed. New scans always write `Some`, including an empty list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limitations: Option<Vec<SessionCoverageLimitation>>,
}

impl SessionCoverage {
    pub fn denominator_is_reliable(&self) -> bool {
        self.limitations.is_some() && self.denominator_reliable
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanResult {
    pub roots: Vec<RootObservation>,
    pub skills: Vec<ScannedSkill>,
    pub placements: Vec<SkillPlacement>,
    pub usage: Vec<UsageEvidence>,
    pub coverage: Vec<SessionCoverage>,
    pub warnings: Vec<String>,
    /// Versioned presence marker for content-identity facts. Missing means a
    /// legacy Snapshot must be rescanned before identity grouping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_identity_algorithm: Option<String>,
    /// Durable source-root read permissions frozen before this Scan. Drifted
    /// permissions remain typed facts but are excluded from approved roots.
    #[serde(default)]
    pub source_root_policy: Vec<crate::source_policy::SourceRootPolicyFact>,
    /// Permission IDs whose bounded discovery or consumption checks observed
    /// drift, even if a later path check appears active again.
    #[serde(default)]
    pub durable_read_drifted_permission_ids: BTreeSet<String>,
}

#[derive(Clone, Debug)]
struct EntryCandidate {
    agent: Option<AgentKind>,
    root: PathBuf,
    governable: bool,
    provider: Option<String>,
    /// Canonical physical SKILL.md resolved during discovery. Safe reuse is
    /// bound to this exact file for the lifetime of one Scan.
    expected_physical_entrypoint: Option<PathBuf>,
    entrypoint: PathBuf,
    link_target: Option<PathBuf>,
    link_status: LinkStatus,
    default_exposed: bool,
}

#[derive(Clone, Debug)]
struct PhysicalPackageObservation {
    content: String,
    modified_at: Option<u64>,
    metadata: SkillMetadata,
    digest: String,
    content_identity_digest: Option<String>,
    fingerprint_completeness: FingerprintCompleteness,
    fingerprint_detail: Option<String>,
    checkpoint: Option<Vec<PackageFileCheckpoint>>,
    executable_relative_files: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct SkillRootPolicy {
    agent: Option<AgentKind>,
    explicit: bool,
    governable: bool,
    detail: Option<String>,
    provider: Option<String>,
}

#[derive(Clone, Copy, Default)]
struct DiscoveryState {
    hidden_ancestor: bool,
    inside_skill_package: bool,
    harness_excluded: bool,
}

#[derive(Clone, Copy, Default)]
struct SkillDiscoveryOutcome {
    depth_bounded: bool,
    non_unicode_skipped: bool,
}

impl SkillDiscoveryOutcome {
    fn merge(&mut self, other: Self) {
        self.depth_bounded |= other.depth_bounded;
        self.non_unicode_skipped |= other.non_unicode_skipped;
    }

    fn is_complete(self) -> bool {
        !self.depth_bounded && !self.non_unicode_skipped
    }
}

impl DiscoveryState {
    fn descend(self, agent: Option<AgentKind>, child_name: &str, containing_skill: bool) -> Self {
        Self {
            hidden_ancestor: self.hidden_ancestor || child_name.starts_with('.'),
            inside_skill_package: self.inside_skill_package || containing_skill,
            harness_excluded: self.harness_excluded
                || hermes_excludes_directory(agent, child_name, containing_skill),
        }
    }
}

#[derive(Clone, Debug)]
struct CodexPluginSkillRoot {
    plugin_id: String,
    path: PathBuf,
    detail: String,
}

#[derive(Default)]
struct PluginConfigState {
    sections: usize,
    enabled_values: Vec<bool>,
}

#[derive(Default)]
struct CodexPluginConfig {
    enabled: BTreeSet<(String, String)>,
    blocked: BTreeSet<(String, String)>,
}

#[derive(Deserialize)]
struct RemotePluginInstallMarker {
    schema_version: u64,
    remote_plugin_id: String,
}

fn default_true() -> bool {
    true
}

fn codex_plugin_skill_roots(home: &Path, warnings: &mut Vec<String>) -> Vec<CodexPluginSkillRoot> {
    let config_path = home.join(".codex/config.toml");
    let config = match fs::read_to_string(&config_path) {
        Ok(config) => parse_codex_plugin_config(&config, warnings),
        Err(error) if error.kind() == io::ErrorKind::NotFound => CodexPluginConfig::default(),
        Err(error) => {
            warnings.push(format!(
                "could not read Codex plugin configuration {}: {error}",
                config_path.display()
            ));
            return Vec::new();
        }
    };
    let cache_root = home.join(".codex/plugins/cache");
    let mut roots = BTreeMap::<String, CodexPluginSkillRoot>::new();

    for (plugin, marketplace) in &config.enabled {
        let plugin_id = format!("{plugin}@{marketplace}");
        let unresolved_plugin_cache = cache_root.join(marketplace).join(plugin);
        if let Some(root) = resolve_codex_plugin_skill_root(
            &cache_root,
            &unresolved_plugin_cache,
            &plugin_id,
            format!(
                "Codex enabled plugin source {plugin_id}; observed and searchable; not Roster-managed"
            ),
            warnings,
        ) {
            roots.insert(plugin_id, root);
        }
    }

    let marketplaces = match fs::read_dir(&cache_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return roots.into_values().collect();
        }
        Err(error) => {
            warnings.push(format!(
                "could not inspect Codex plugin cache {}: {error}",
                cache_root.display()
            ));
            return roots.into_values().collect();
        }
    };
    for marketplace_entry in marketplaces.filter_map(Result::ok) {
        let Some(marketplace) = marketplace_entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !safe_path_component(&marketplace) {
            continue;
        }
        let Some(marketplace_cache) = contained_directory(&cache_root, &marketplace_entry.path())
        else {
            continue;
        };
        let plugins = match fs::read_dir(&marketplace_cache) {
            Ok(entries) => entries,
            Err(error) => {
                warnings.push(format!(
                    "could not inspect Codex plugin marketplace cache {}: {error}",
                    marketplace_cache.display()
                ));
                continue;
            }
        };
        for plugin_entry in plugins.filter_map(Result::ok) {
            let Some(plugin) = plugin_entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !safe_path_component(&plugin) {
                continue;
            }
            let identity = (plugin.clone(), marketplace.clone());
            let plugin_id = format!("{plugin}@{marketplace}");
            if config.blocked.contains(&identity) || roots.contains_key(&plugin_id) {
                continue;
            }
            let Some(plugin_cache) = contained_directory(&cache_root, &plugin_entry.path()) else {
                continue;
            };
            if !valid_remote_plugin_install_marker(&plugin_cache, &plugin_id, warnings) {
                continue;
            }
            if let Some(root) = resolve_codex_plugin_skill_root(
                &cache_root,
                &plugin_cache,
                &plugin_id,
                format!(
                    "Codex installed remote plugin source {plugin_id}; observed and searchable; not Roster-managed"
                ),
                warnings,
            ) {
                roots.insert(plugin_id, root);
            }
        }
    }

    roots.into_values().collect()
}

fn resolve_codex_plugin_skill_root(
    cache_root: &Path,
    unresolved_plugin_cache: &Path,
    plugin_id: &str,
    detail: String,
    warnings: &mut Vec<String>,
) -> Option<CodexPluginSkillRoot> {
    let Some(plugin_cache) = contained_directory(cache_root, unresolved_plugin_cache) else {
        warnings.push(format!(
            "Codex plugin {plugin_id} has no contained local cache; skipped"
        ));
        return None;
    };
    let latest = plugin_cache.join("latest/skills");
    if let Some(path) = contained_directory(&plugin_cache, &latest) {
        return Some(CodexPluginSkillRoot {
            plugin_id: plugin_id.to_owned(),
            path,
            detail,
        });
    }
    let mut candidates = match fs::read_dir(&plugin_cache) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "latest")
            .filter_map(|entry| contained_directory(&plugin_cache, &entry.path().join("skills")))
            .collect::<Vec<_>>(),
        Err(error) => {
            warnings.push(format!(
                "could not inspect Codex plugin cache {}: {error}",
                plugin_cache.display()
            ));
            return None;
        }
    };
    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [path] => Some(CodexPluginSkillRoot {
            plugin_id: plugin_id.to_owned(),
            path: path.clone(),
            detail,
        }),
        [] => None,
        _ => {
            warnings.push(format!(
                "Codex plugin {plugin_id} has multiple cached Skill versions and no unique latest version; skipped"
            ));
            None
        }
    }
}

fn valid_remote_plugin_install_marker(
    plugin_cache: &Path,
    plugin_id: &str,
    warnings: &mut Vec<String>,
) -> bool {
    let marker = plugin_cache.join(".codex-remote-plugin-install.json");
    if !marker.exists() {
        return false;
    }
    let canonical_marker = match fs::canonicalize(&marker) {
        Ok(path) if path.starts_with(plugin_cache) => path,
        Ok(_) => {
            warnings.push(format!(
                "Codex remote plugin {plugin_id} install marker escapes its cache; skipped"
            ));
            return false;
        }
        Err(error) => {
            warnings.push(format!(
                "could not resolve Codex remote plugin {plugin_id} install marker: {error}"
            ));
            return false;
        }
    };
    let content = match read_bounded(&canonical_marker, MAX_REMOTE_PLUGIN_INSTALL_BYTES) {
        Ok((content, _)) => content,
        Err(error) => {
            warnings.push(format!(
                "could not read Codex remote plugin {plugin_id} install marker: {error}"
            ));
            return false;
        }
    };
    let marker = match serde_json::from_str::<RemotePluginInstallMarker>(&content) {
        Ok(marker) => marker,
        Err(error) => {
            warnings.push(format!(
                "Codex remote plugin {plugin_id} has an invalid install marker: {error}"
            ));
            return false;
        }
    };
    if marker.schema_version != 1
        || marker.remote_plugin_id.is_empty()
        || marker.remote_plugin_id.len() > 256
        || !marker
            .remote_plugin_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        warnings.push(format!(
            "Codex remote plugin {plugin_id} has an unsupported install marker; skipped"
        ));
        return false;
    }
    true
}

fn parse_codex_plugin_config(config: &str, warnings: &mut Vec<String>) -> CodexPluginConfig {
    let mut sections = BTreeMap::<String, PluginConfigState>::new();
    let mut current_plugin = None::<String>;
    for line in config.lines() {
        let line = strip_toml_comment(line).trim();
        if line.starts_with('[') {
            current_plugin = parse_plugin_section(line);
            if let Some(plugin_id) = &current_plugin {
                sections.entry(plugin_id.clone()).or_default().sections += 1;
            }
            continue;
        }
        let Some(plugin_id) = &current_plugin else {
            continue;
        };
        let value = line;
        let enabled = value
            .split_once('=')
            .filter(|(key, _)| key.trim() == "enabled")
            .map(|(_, value)| value.trim());
        if enabled == Some("true") {
            sections
                .entry(plugin_id.clone())
                .or_default()
                .enabled_values
                .push(true);
        } else if enabled == Some("false") {
            sections
                .entry(plugin_id.clone())
                .or_default()
                .enabled_values
                .push(false);
        }
    }

    let mut parsed = CodexPluginConfig::default();
    for (plugin_id, state) in sections {
        if state.sections != 1 || state.enabled_values.len() != 1 {
            warnings.push(format!(
                "Codex plugin {plugin_id} has ambiguous configuration; skipped"
            ));
            if let Ok(identity) = parse_codex_plugin_identifier(&plugin_id) {
                parsed.blocked.insert(identity);
            }
            continue;
        }
        let identity = match parse_codex_plugin_identifier(&plugin_id) {
            Ok(identity) => identity,
            Err(reason) => {
                if state.enabled_values[0] {
                    warnings.push(format!(
                        "Codex plugin identifier {plugin_id} is {reason}; skipped"
                    ));
                }
                continue;
            }
        };
        if state.enabled_values[0] {
            parsed.enabled.insert(identity);
        } else {
            parsed.blocked.insert(identity);
        }
    }
    parsed
}

fn parse_codex_plugin_identifier(plugin_id: &str) -> Result<(String, String), &'static str> {
    let Some((plugin, marketplace)) = plugin_id.split_once('@') else {
        return Err("invalid");
    };
    if plugin.is_empty()
        || marketplace.is_empty()
        || marketplace.contains('@')
        || !safe_path_component(plugin)
        || !safe_path_component(marketplace)
    {
        return Err("unsafe");
    }
    Ok((plugin.to_owned(), marketplace.to_owned()))
}

fn parse_plugin_section(line: &str) -> Option<String> {
    line.strip_prefix("[plugins.\"")
        .and_then(|value| value.strip_suffix("\"]"))
        .map(str::to_owned)
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character == '#' && !quoted {
            return &line[..index];
        }
    }
    line
}

fn safe_path_component(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn contained_directory(base: &Path, candidate: &Path) -> Option<PathBuf> {
    if !candidate.is_dir() {
        return None;
    }
    let base = fs::canonicalize(base).ok()?;
    let candidate = fs::canonicalize(candidate).ok()?;
    candidate.starts_with(&base).then_some(candidate)
}

pub fn scan(options: &ScanOptions) -> io::Result<ScanResult> {
    require_unicode_scan_options(options)?;
    let mut result = ScanResult {
        content_identity_algorithm: Some(CONTENT_IDENTITY_ALGORITHM.into()),
        ..ScanResult::default()
    };
    let known = known_agent_roots(&options.home);
    let plugin_roots = codex_plugin_skill_roots(&options.home, &mut result.warnings);
    // These paths are already an explicit trust decision from the caller.
    // Inventory and link-containment decisions use the resolved physical
    // sources so an alias and its canonical path have identical semantics.
    let confirmed_source_roots = normalized_confirmed_source_roots(&options.explicit_source_roots);
    let mut shared_roots = vec![
        options.home.join(".agents_skills"),
        options.home.join(".skillroster/library"),
    ];
    shared_roots.extend(options.managed_source_roots.iter().cloned());
    let shared_roots = normalized_confirmed_source_roots(&shared_roots);
    let approved_roots = known
        .iter()
        .flat_map(|roots| roots.skill_roots.iter().cloned())
        .chain(shared_roots.iter().cloned())
        .chain(plugin_roots.iter().map(|root| root.path.clone()))
        .chain(
            options
                .explicit_skill_roots
                .iter()
                .map(|root| root.path.clone()),
        )
        .chain(options.explicit_source_roots.iter().cloned())
        .chain(confirmed_source_roots.iter().cloned())
        .collect::<Vec<_>>();
    let approved_roots = normalized_confirmed_source_roots(&approved_roots);
    // Durable paths were already canonicalized and identity-frozen by the
    // caller. Never canonicalize them again here: a post-freeze retarget must
    // not become an approved root.
    let mut approved_roots = approved_roots;
    approved_roots.extend(
        options
            .durable_read_roots
            .iter()
            .map(|root| root.path.clone()),
    );
    approved_roots.sort();
    approved_roots.dedup();
    let mut candidates = Vec::new();

    for roots in &known {
        for root in &roots.skill_roots {
            observe_skill_root(
                root,
                SkillRootPolicy {
                    agent: Some(roots.agent),
                    explicit: false,
                    governable: true,
                    detail: None,
                    provider: None,
                },
                &approved_roots,
                options.max_depth,
                &mut result,
                &mut candidates,
            );
        }
    }
    for root in shared_roots.iter().filter(|path| path.exists()) {
        observe_skill_root(
            root,
            SkillRootPolicy {
                agent: None,
                explicit: false,
                governable: true,
                detail: None,
                provider: None,
            },
            &approved_roots,
            options.max_depth,
            &mut result,
            &mut candidates,
        );
    }
    for root in &confirmed_source_roots {
        observe_skill_root(
            root,
            SkillRootPolicy {
                agent: None,
                explicit: true,
                governable: true,
                detail: None,
                provider: None,
            },
            &approved_roots,
            options.max_depth,
            &mut result,
            &mut candidates,
        );
    }
    for root in &options.durable_read_roots {
        observe_durable_skill_root(
            root,
            SkillRootPolicy {
                agent: None,
                explicit: true,
                governable: false,
                detail: Some("durable exact local read permission; not governable".into()),
                provider: None,
            },
            &approved_roots,
            options.max_depth,
            &mut result,
            &mut candidates,
        );
    }
    for root in &options.explicit_skill_roots {
        observe_skill_root(
            &root.path,
            SkillRootPolicy {
                agent: Some(root.agent),
                explicit: true,
                governable: true,
                detail: None,
                provider: None,
            },
            &approved_roots,
            options.max_depth,
            &mut result,
            &mut candidates,
        );
    }
    for root in &plugin_roots {
        observe_skill_root(
            &root.path,
            SkillRootPolicy {
                agent: None,
                explicit: false,
                governable: false,
                detail: Some(root.detail.clone()),
                provider: Some(root.plugin_id.clone()),
            },
            &approved_roots,
            options.max_depth,
            &mut result,
            &mut candidates,
        );
    }

    materialize_candidates(
        candidates,
        &options.durable_read_roots,
        &confirmed_source_roots,
        &mut result,
    );

    if options.include_session_evidence {
        for roots in &known {
            if options.excluded_session_agents.contains(&roots.agent) {
                observe_excluded_session_roots(
                    roots.agent,
                    &roots.session_roots,
                    "excluded by local lifecycle policy",
                    &mut result,
                );
            } else {
                scan_sessions(roots.agent, &roots.session_roots, &mut result);
            }
        }
    } else {
        for roots in &known {
            observe_excluded_session_roots(
                roots.agent,
                &roots.session_roots,
                "session evidence disabled",
                &mut result,
            );
        }
    }

    result.skills.sort_by(|a, b| a.id.cmp(&b.id));
    result.placements.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

fn require_unicode_scan_options(options: &ScanOptions) -> io::Result<()> {
    let paths = std::iter::once(options.home.as_path())
        .chain(
            options
                .explicit_skill_roots
                .iter()
                .map(|root| root.path.as_path()),
        )
        .chain(options.explicit_source_roots.iter().map(PathBuf::as_path))
        .chain(
            options
                .durable_read_roots
                .iter()
                .map(|root| root.path.as_path()),
        )
        .chain(options.managed_source_roots.iter().map(PathBuf::as_path));
    if paths.into_iter().all(|path| path.to_str().is_some()) {
        Ok(())
    } else {
        Err(non_unicode_identity_error())
    }
}

fn normalized_confirmed_source_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut normalized = roots
        .iter()
        .map(|root| fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn observe_excluded_session_roots(
    agent: AgentKind,
    roots: &[PathBuf],
    detail: &str,
    result: &mut ScanResult,
) {
    for root in roots {
        result.roots.push(RootObservation {
            agent: Some(agent),
            kind: RootKind::Sessions,
            path: root.clone(),
            status: RootStatus::Excluded,
            explicit: false,
            detail: Some(detail.into()),
            discovery_complete: true,
        });
    }
}

fn observe_skill_root(
    root: &Path,
    policy: SkillRootPolicy,
    approved_roots: &[PathBuf],
    max_depth: usize,
    result: &mut ScanResult,
    candidates: &mut Vec<EntryCandidate>,
) {
    let status = match fs::read_dir(root) {
        Ok(_) => RootStatus::Included,
        Err(error) if error.kind() == io::ErrorKind::NotFound => RootStatus::Missing,
        Err(_) => RootStatus::Inaccessible,
    };
    result.roots.push(RootObservation {
        agent: policy.agent,
        kind: RootKind::Skills,
        path: root.to_path_buf(),
        status,
        explicit: policy.explicit,
        detail: policy.detail.clone(),
        discovery_complete: true,
    });
    if status != RootStatus::Included {
        return;
    }

    match discover_entrypoints(
        &policy,
        root,
        approved_roots,
        root,
        max_depth,
        DiscoveryState::default(),
        candidates,
    ) {
        Ok(outcome) if outcome.is_complete() => {}
        Ok(outcome) => {
            let mut details = Vec::new();
            if outcome.depth_bounded {
                details.push(format!("Skill discovery was bounded at depth {max_depth}"));
            }
            if outcome.non_unicode_skipped {
                details.push(
                    "Skill discovery skipped a non-Unicode path that cannot have stable identity"
                        .to_owned(),
                );
            }
            let detail = details.join("; ");
            result.warnings.push(format!(
                "could not completely inspect skill root {}: {detail}",
                root.display()
            ));
            if let Some(observation) = result.roots.last_mut() {
                observation.discovery_complete = false;
                observation.detail = Some(match observation.detail.take() {
                    Some(existing) => format!("{existing}; {detail}"),
                    None => detail,
                });
            }
        }
        Err(error) => {
            result.warnings.push(format!(
                "could not completely inspect skill root {}: {error}",
                root.display()
            ));
            if let Some(observation) = result.roots.last_mut() {
                observation.status = RootStatus::Inaccessible;
                observation.discovery_complete = false;
                observation.detail = Some(error.to_string());
            }
        }
    }
}

fn observe_durable_skill_root(
    root: &DurableReadRoot,
    policy: SkillRootPolicy,
    approved_roots: &[PathBuf],
    max_depth: usize,
    result: &mut ScanResult,
    candidates: &mut Vec<EntryCandidate>,
) {
    observe_durable_skill_root_with_hook(
        root,
        policy,
        approved_roots,
        max_depth,
        result,
        candidates,
        |_| {},
    );
}

fn observe_durable_skill_root_with_hook(
    root: &DurableReadRoot,
    policy: SkillRootPolicy,
    approved_roots: &[PathBuf],
    max_depth: usize,
    result: &mut ScanResult,
    candidates: &mut Vec<EntryCandidate>,
    mut before_enumerate: impl FnMut(&Path),
) {
    let reject = |detail: &str, result: &mut ScanResult| {
        result
            .durable_read_drifted_permission_ids
            .insert(root.permission_id.clone());
        result.roots.push(RootObservation {
            agent: policy.agent,
            kind: RootKind::Skills,
            path: root.path.clone(),
            status: RootStatus::Inaccessible,
            explicit: policy.explicit,
            detail: Some(detail.into()),
            discovery_complete: false,
        });
        result.warnings.push(format!(
            "excluded durable source root {}: {detail}",
            root.path.display()
        ));
    };
    if !crate::source_policy::identity_matches_exact(&root.path, &root.identity) {
        reject("identity drift before enumeration", result);
        return;
    }
    before_enumerate(&root.path);
    if !crate::source_policy::identity_matches_exact(&root.path, &root.identity) {
        reject("identity drift immediately before enumeration", result);
        return;
    }
    let root_index = result.roots.len();
    let candidate_start = candidates.len();
    observe_skill_root(
        &root.path,
        policy,
        approved_roots,
        max_depth,
        result,
        candidates,
    );
    if !crate::source_policy::identity_matches_exact(&root.path, &root.identity) {
        result
            .durable_read_drifted_permission_ids
            .insert(root.permission_id.clone());
        candidates.truncate(candidate_start);
        if let Some(observation) = result.roots.get_mut(root_index) {
            observation.status = RootStatus::Inaccessible;
            observation.discovery_complete = false;
            observation.detail =
                Some("identity drift during enumeration; candidates discarded".into());
        }
        result.warnings.push(format!(
            "discarded durable source-root candidates after identity drift: {}",
            root.path.display()
        ));
    }
}

fn discover_entrypoints(
    policy: &SkillRootPolicy,
    placement_root: &Path,
    approved_roots: &[PathBuf],
    directory: &Path,
    max_depth: usize,
    state: DiscoveryState,
    output: &mut Vec<EntryCandidate>,
) -> io::Result<SkillDiscoveryOutcome> {
    let depth = directory
        .strip_prefix(placement_root)
        .map(|path| path.components().count())
        .unwrap_or(max_depth.saturating_add(1));
    if depth > max_depth {
        return Ok(SkillDiscoveryOutcome {
            depth_bounded: true,
            non_unicode_skipped: false,
        });
    }
    let mut outcome = SkillDiscoveryOutcome::default();
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let directory_has_skill = directory.join("SKILL.md").is_file();
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let file_name = entry.file_name();
        if file_name == "SKILL.md" {
            let (target, link_status) = inspect_link(approved_roots, &path, &metadata);
            let expected_physical_entrypoint = fs::canonicalize(&path)
                .ok()
                .filter(|path| path.to_str().is_some());
            let default_exposed =
                default_exposure_for_candidate(policy, placement_root, &path, depth, state);
            output.push(EntryCandidate {
                agent: policy.agent,
                root: placement_root.to_path_buf(),
                governable: policy.governable,
                provider: policy.provider.clone(),
                expected_physical_entrypoint,
                entrypoint: path,
                link_target: target,
                link_status,
                default_exposed,
            });
        } else if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            let Some(child_name) = file_name.to_str() else {
                outcome.non_unicode_skipped = true;
                continue;
            };
            let child_state = state.descend(policy.agent, child_name, directory_has_skill);
            outcome.merge(discover_entrypoints(
                policy,
                placement_root,
                approved_roots,
                &path,
                max_depth,
                child_state,
                output,
            )?);
        } else if metadata.file_type().is_symlink() {
            // A linked Skill directory is a placement too, but it is never
            // traversed recursively before the boundary has been evaluated.
            let linked_entrypoint = path.join("SKILL.md");
            let (target, status) = inspect_link(approved_roots, &path, &metadata);
            let Some(child_name) = file_name.to_str() else {
                outcome.non_unicode_skipped = true;
                continue;
            };
            if linked_entrypoint.exists() || status != LinkStatus::Valid {
                let expected_physical_entrypoint = fs::canonicalize(&linked_entrypoint)
                    .ok()
                    .filter(|path| path.to_str().is_some());
                let child_state = state.descend(policy.agent, child_name, directory_has_skill);
                let default_exposed = default_exposure_for_candidate(
                    policy,
                    placement_root,
                    &linked_entrypoint,
                    depth + 1,
                    child_state,
                );
                output.push(EntryCandidate {
                    agent: policy.agent,
                    root: placement_root.to_path_buf(),
                    governable: policy.governable,
                    provider: policy.provider.clone(),
                    expected_physical_entrypoint,
                    entrypoint: linked_entrypoint,
                    link_target: target,
                    link_status: status,
                    default_exposed,
                });
            }
        }
    }
    Ok(outcome)
}

const HERMES_EXCLUDED_SKILL_DIRS: &[&str] = &[
    ".git",
    ".github",
    ".hub",
    ".archive",
    ".venv",
    "venv",
    "node_modules",
    "site-packages",
    "__pycache__",
    ".tox",
    ".nox",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
];

const HERMES_SKILL_SUPPORT_DIRS: &[&str] = &["references", "templates", "assets", "scripts"];

fn hermes_excludes_directory(agent: Option<AgentKind>, name: &str, containing_skill: bool) -> bool {
    agent == Some(AgentKind::Hermes)
        && (HERMES_EXCLUDED_SKILL_DIRS.contains(&name)
            || (containing_skill && HERMES_SKILL_SUPPORT_DIRS.contains(&name)))
}

fn default_exposure_for_candidate(
    policy: &SkillRootPolicy,
    placement_root: &Path,
    entrypoint: &Path,
    depth: usize,
    state: DiscoveryState,
) -> bool {
    let Some(agent) = policy.agent else {
        return false;
    };
    match agent.skill_discovery_semantics() {
        SkillDiscoverySemantics::Codex => {
            let relative = entrypoint.strip_prefix(placement_root).ok();
            let scanning_system_root = placement_root
                .file_name()
                .is_some_and(|name| name == ".system");
            let visible_below_system_root = relative
                .and_then(Path::parent)
                .and_then(|path| {
                    let mut components = path.components();
                    (components.next()?.as_os_str() == ".system").then(|| {
                        components.all(|component| {
                            component
                                .as_os_str()
                                .to_str()
                                .is_some_and(|name| !name.starts_with('.'))
                        })
                    })
                })
                .unwrap_or(false);
            !state.harness_excluded
                && if scanning_system_root {
                    !state.hidden_ancestor
                } else if visible_below_system_root {
                    true
                } else {
                    !state.hidden_ancestor
                }
        }
        SkillDiscoverySemantics::ClaudeCode => {
            depth == 1
                && !state.hidden_ancestor
                && !state.harness_excluded
                && entrypoint
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !name.starts_with('.'))
        }
        SkillDiscoverySemantics::Pi => {
            !state.hidden_ancestor && !state.inside_skill_package && !state.harness_excluded
        }
        SkillDiscoverySemantics::Hermes => !state.harness_excluded,
        SkillDiscoverySemantics::Conservative => true,
    }
}

fn inspect_link(
    approved_roots: &[PathBuf],
    path: &Path,
    metadata: &fs::Metadata,
) -> (Option<PathBuf>, LinkStatus) {
    if !metadata.file_type().is_symlink() {
        return (None, LinkStatus::NotLink);
    }
    let raw_target = match fs::read_link(path) {
        Ok(target) => target,
        Err(_) => return (None, LinkStatus::Broken),
    };
    if raw_target.to_str().is_none() {
        return (None, LinkStatus::Broken);
    }
    let target = if raw_target.is_absolute() {
        raw_target
    } else {
        path.parent().unwrap_or(Path::new(".")).join(raw_target)
    };
    let normalized_target = lexical_normalize(&target);
    // Trust the resolved destination, not the spelling of the path used to
    // reach it. A confirmed canonical source and a symlink alias to that
    // source must therefore have identical semantics. If the target is
    // broken, resolve its nearest existing ancestor so an in-root missing
    // target remains Broken while an indirect escape remains fail-closed.
    let resolved_target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(_) => {
            let status = canonical_existing_ancestor(&normalized_target)
                .filter(|ancestor| is_within_resolved_root(ancestor, approved_roots))
                .map_or(LinkStatus::EscapesRoot, |_| LinkStatus::Broken);
            return (Some(normalized_target), status);
        }
    };
    if !is_within_resolved_root(&resolved_target, approved_roots) {
        return (Some(normalized_target), LinkStatus::EscapesRoot);
    }
    if fs::metadata(path).is_err() {
        return (Some(normalized_target), LinkStatus::Broken);
    }
    (Some(normalized_target), LinkStatus::Valid)
}

fn canonical_existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find_map(|ancestor| fs::canonicalize(ancestor).ok())
}

fn is_within_resolved_root(path: &Path, approved_roots: &[PathBuf]) -> bool {
    approved_roots.iter().any(|root| path.starts_with(root))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn durable_candidate_binding_is_current(
    candidate: &EntryCandidate,
    anchor: &DurableReadRoot,
) -> bool {
    let Some(expected) = candidate
        .expected_physical_entrypoint
        .as_deref()
        .and_then(Path::parent)
    else {
        return false;
    };
    expected.starts_with(&anchor.path)
        && crate::source_policy::identity_matches_exact(&anchor.path, &anchor.identity)
}

fn candidate_binding_is_current(candidate: &EntryCandidate) -> bool {
    let Some(expected_entrypoint) = candidate.expected_physical_entrypoint.as_ref() else {
        return false;
    };
    fs::canonicalize(&candidate.entrypoint)
        .ok()
        .is_some_and(|current| current == *expected_entrypoint)
}

fn materialize_candidates(
    candidates: Vec<EntryCandidate>,
    durable_read_roots: &[DurableReadRoot],
    temporary_source_roots: &[PathBuf],
    result: &mut ScanResult,
) {
    let _ = materialize_candidates_with_hook(
        candidates,
        durable_read_roots,
        temporary_source_roots,
        result,
        |_| {},
    );
}

fn observe_physical_skill_package(
    physical_entrypoint: &Path,
    physical_directory: &Path,
    display_entrypoint: &Path,
    display_directory: &Path,
    result: &mut ScanResult,
) -> PhysicalPackageObservation {
    let (content, modified_at) = match read_bounded(physical_entrypoint, MAX_SKILL_FILE_BYTES) {
        Ok(value) => value,
        Err(error) => {
            result.warnings.push(format!(
                "could not read Skill entrypoint {}: {error}",
                display_entrypoint.display()
            ));
            (String::new(), None)
        }
    };
    let metadata = parse_skill_markdown(&content);
    let (
        digest,
        content_identity_digest,
        fingerprint_completeness,
        fingerprint_detail,
        checkpoint,
        executable_relative_files,
    ) = match digest_skill_directory(physical_directory) {
        Ok(fingerprint) => (
            fingerprint.digest,
            (fingerprint.completeness == FingerprintCompleteness::Complete)
                .then_some(fingerprint.content_identity_digest),
            fingerprint.completeness,
            fingerprint.detail,
            fingerprint.checkpoint,
            fingerprint.executable_relative_files,
        ),
        Err(error) => {
            let completeness = if is_non_unicode_identity_error(&error) {
                FingerprintCompleteness::Unreadable
            } else if error.kind() == io::ErrorKind::InvalidData {
                FingerprintCompleteness::Bounded
            } else {
                FingerprintCompleteness::Unreadable
            };
            result.warnings.push(format!(
                "could not completely fingerprint Skill package {}: {error}",
                display_directory.display()
            ));
            let executable_relative_files = package_checkpoint(physical_directory)
                .ok()
                .flatten()
                .map(|checkpoints| executable_files_from_checkpoints(&checkpoints))
                .unwrap_or_default();
            (
                stable_digest(content.as_bytes()),
                None,
                completeness,
                Some(error.to_string()),
                None,
                executable_relative_files,
            )
        }
    };
    PhysicalPackageObservation {
        content,
        modified_at,
        metadata,
        digest,
        content_identity_digest,
        fingerprint_completeness,
        fingerprint_detail,
        checkpoint,
        executable_relative_files,
    }
}

fn unreadable_package_observation(candidate: &EntryCandidate) -> PhysicalPackageObservation {
    let entrypoint = candidate
        .entrypoint
        .to_str()
        .expect("discovery excludes non-Unicode entrypoints");
    PhysicalPackageObservation {
        content: String::new(),
        modified_at: None,
        metadata: SkillMetadata::default(),
        digest: stable_digest(entrypoint.as_bytes()),
        content_identity_digest: None,
        fingerprint_completeness: FingerprintCompleteness::Unreadable,
        fingerprint_detail: Some("Skill package was not read because its link is unsafe".into()),
        checkpoint: None,
        executable_relative_files: Vec::new(),
    }
}

fn materialize_candidates_with_hook(
    candidates: Vec<EntryCandidate>,
    durable_read_roots: &[DurableReadRoot],
    temporary_source_roots: &[PathBuf],
    result: &mut ScanResult,
    mut before_read: impl FnMut(&Path),
) -> usize {
    let mut skills = BTreeMap::<String, ScannedSkill>::new();
    let mut physical_packages = BTreeMap::<PathBuf, PhysicalPackageObservation>::new();
    let mut drifted_physical_packages = BTreeMap::<PathBuf, String>::new();
    let mut physical_package_observations = 0;
    for candidate in candidates {
        let directory = candidate
            .entrypoint
            .parent()
            .unwrap_or(&candidate.root)
            .to_path_buf();
        let expected_directory = candidate
            .expected_physical_entrypoint
            .as_deref()
            .and_then(Path::parent);
        let temporary_override = expected_directory.is_some_and(|resolved| {
            temporary_source_roots
                .iter()
                .any(|root| resolved.starts_with(root))
        });
        let durable_anchor = (!temporary_override)
            .then(|| {
                durable_read_roots.iter().find(|root| {
                    candidate.entrypoint.starts_with(&root.path)
                        || expected_directory
                            .is_some_and(|resolved| resolved.starts_with(&root.path))
                })
            })
            .flatten();
        before_read(&candidate.entrypoint);
        let placement_binding_valid_before_read = candidate_binding_is_current(&candidate);
        let durable_binding_valid_before_read = durable_anchor
            .is_none_or(|root| durable_candidate_binding_is_current(&candidate, root));
        let initially_safe = !matches!(
            candidate.link_status,
            LinkStatus::Broken | LinkStatus::EscapesRoot
        ) && placement_binding_valid_before_read
            && durable_binding_valid_before_read;
        let (observation, package_validation_error) = if initially_safe {
            let physical_entrypoint = candidate
                .expected_physical_entrypoint
                .as_ref()
                .expect("safe binding has a physical entrypoint");
            let physical_directory =
                expected_directory.expect("safe binding has a physical directory");
            let cacheable = durable_anchor.is_none() && !temporary_override;
            if cacheable {
                if let Some(reason) = drifted_physical_packages.get(physical_entrypoint) {
                    (
                        unreadable_package_observation(&candidate),
                        Some(reason.clone()),
                    )
                } else if let Some(observation) =
                    physical_packages.get(physical_entrypoint).cloned()
                {
                    let validation_error = match package_checkpoint(physical_directory) {
                        Ok(Some(current))
                            if observation.checkpoint.as_ref() == Some(&current) =>
                        {
                            None
                        }
                        Ok(Some(_)) => {
                            Some("physical package changed before cache reuse".into())
                        }
                        Ok(None) => Some(
                            "physical package could not be completely revalidated before cache reuse"
                                .into(),
                        ),
                        Err(error) => Some(format!(
                            "physical package could not be revalidated before cache reuse: {error}"
                        )),
                    };
                    if let Some(reason) = validation_error {
                        drifted_physical_packages
                            .insert(physical_entrypoint.clone(), reason.clone());
                        (unreadable_package_observation(&candidate), Some(reason))
                    } else {
                        (observation, None)
                    }
                } else {
                    physical_package_observations += 1;
                    let observation = observe_physical_skill_package(
                        physical_entrypoint,
                        physical_directory,
                        &candidate.entrypoint,
                        &directory,
                        result,
                    );
                    if observation.checkpoint.is_some() {
                        physical_packages.insert(physical_entrypoint.clone(), observation.clone());
                    }
                    (observation, None)
                }
            } else {
                physical_package_observations += 1;
                (
                    observe_physical_skill_package(
                        physical_entrypoint,
                        physical_directory,
                        &candidate.entrypoint,
                        &directory,
                        result,
                    ),
                    None,
                )
            }
        } else {
            result.warnings.push(format!(
                "did not read unsafe Skill link {}",
                candidate.entrypoint.display()
            ));
            (unreadable_package_observation(&candidate), None)
        };
        let PhysicalPackageObservation {
            mut content,
            mut modified_at,
            mut metadata,
            mut digest,
            mut content_identity_digest,
            mut fingerprint_completeness,
            mut fingerprint_detail,
            checkpoint: _,
            executable_relative_files,
        } = observation;
        let mut executable_files = executable_relative_files
            .into_iter()
            .map(|relative| directory.join(relative))
            .collect::<Vec<_>>();
        let placement_binding_valid_after = candidate_binding_is_current(&candidate);
        let durable_binding_valid_after = durable_anchor
            .is_none_or(|root| durable_candidate_binding_is_current(&candidate, root));
        let safe_to_read = initially_safe
            && placement_binding_valid_after
            && durable_binding_valid_after
            && package_validation_error.is_none();
        if !placement_binding_valid_before_read
            || !placement_binding_valid_after
            || !durable_binding_valid_before_read
            || !durable_binding_valid_after
            || package_validation_error.is_some()
        {
            if let Some(physical_entrypoint) = candidate.expected_physical_entrypoint.as_ref() {
                physical_packages.remove(physical_entrypoint);
            }
            if let Some(root) = durable_anchor {
                result
                    .durable_read_drifted_permission_ids
                    .insert(root.permission_id.clone());
            }
            let drift_kind = if let Some(reason) = package_validation_error.as_deref() {
                reason
            } else if durable_anchor.is_some() {
                "durable source-root binding drift"
            } else {
                "placement binding drift"
            };
            result.warnings.push(format!(
                "discarded Skill data after {drift_kind}: {}",
                candidate.entrypoint.display()
            ));
            content.clear();
            modified_at = None;
            metadata = SkillMetadata::default();
            let entrypoint = candidate
                .entrypoint
                .to_str()
                .expect("discovery excludes non-Unicode entrypoints");
            digest = stable_digest(entrypoint.as_bytes());
            content_identity_digest = None;
            fingerprint_completeness = FingerprintCompleteness::Unreadable;
            fingerprint_detail = Some(format!(
                "Skill package data was discarded because {drift_kind}"
            ));
            executable_files.clear();
        };
        let identity_basis = match (&metadata.source, &metadata.version, &metadata.revision) {
            (Some(source), Some(version), _) => format!("source:{source}@{version}"),
            (Some(source), _, Some(revision)) => format!("source:{source}@{revision}"),
            _ if safe_to_read && !content.is_empty() => {
                content_identity_digest.as_ref().map_or_else(
                    || {
                        let entrypoint = candidate
                            .entrypoint
                            .to_str()
                            .expect("discovery excludes non-Unicode entrypoints");
                        format!("incomplete-content:{entrypoint}:{digest}")
                    },
                    |identity| format!("content:{identity}"),
                )
            }
            _ => {
                let entrypoint = candidate
                    .entrypoint
                    .to_str()
                    .expect("discovery excludes non-Unicode entrypoints");
                format!("unreadable-link:{entrypoint}")
            }
        };
        let skill_id = format!("skill_{}", stable_digest(identity_basis.as_bytes()));
        let name = metadata
            .name
            .clone()
            .or_else(|| {
                directory
                    .file_name()
                    .and_then(|name| name.to_str().map(str::to_owned))
            })
            .unwrap_or_else(|| "unnamed".into());
        let normalized_text = normalize_search_text(&content);
        let physical_directory = safe_to_read.then(|| {
            fs::canonicalize(&candidate.entrypoint)
                .ok()
                .and_then(|entrypoint| entrypoint.parent().map(Path::to_path_buf))
                .or_else(|| fs::canonicalize(&directory).ok())
                .unwrap_or_else(|| directory.clone())
        });
        let declared_name_matches_directory = metadata.name.as_ref().and_then(|declared| {
            directory.file_name().map(|directory_name| {
                directory_name
                    .to_str()
                    .is_some_and(|name| declared.eq_ignore_ascii_case(name))
            })
        });
        skills
            .entry(skill_id.clone())
            .or_insert_with(|| ScannedSkill {
                id: skill_id.clone(),
                name,
                metadata,
                content_digest: digest.clone(),
                content_identity_digest: content_identity_digest.clone(),
                digest_algorithm: "sha256-v1".into(),
                summary: summarize_markdown(&content, 320),
                normalized_text,
                modified_at_unix: modified_at,
            });
        let root = candidate
            .root
            .to_str()
            .expect("discovery excludes non-Unicode roots");
        let entrypoint = candidate
            .entrypoint
            .to_str()
            .expect("discovery excludes non-Unicode entrypoints");
        let placement_basis = format!(
            "{}\0{}\0{}",
            candidate.agent.map(AgentKind::id).unwrap_or("explicit"),
            root,
            entrypoint
        );
        let mutation_scope = if candidate.provider.is_some() {
            MutationScope::ProviderReadOnly
        } else if durable_anchor.is_some() {
            MutationScope::DurableReadOnly
        } else if !candidate.governable
            || candidate.link_status == LinkStatus::EscapesRoot
            || !safe_to_read
        {
            MutationScope::UntrustedExternal
        } else {
            MutationScope::Mutable
        };
        result.placements.push(SkillPlacement {
            id: format!("placement_{}", stable_digest(placement_basis.as_bytes())),
            skill_id,
            agent: candidate.agent,
            root: candidate.root,
            directory,
            entrypoint: candidate.entrypoint,
            physical_directory,
            content_digest: digest,
            entrypoint_digest: safe_to_read.then(|| stable_digest(content.as_bytes())),
            fingerprint_completeness,
            fingerprint_detail,
            link_target: candidate.link_target,
            link_status: candidate.link_status,
            default_exposed: candidate.agent.is_some() && candidate.default_exposed,
            owned_by_agent: Some(candidate.agent.is_some()),
            mutation_scope: Some(mutation_scope),
            governable: mutation_scope == MutationScope::Mutable,
            provider: candidate.provider,
            executable_files,
            declared_name_matches_directory,
        });
    }
    result.skills = skills.into_values().collect();
    physical_package_observations
}

pub fn parse_skill_markdown(markdown: &str) -> SkillMetadata {
    let mut metadata = SkillMetadata::default();
    let mut markdown_lines = markdown.lines();
    if markdown_lines.next() != Some("---") {
        return metadata;
    }
    let mut frontmatter = Vec::new();
    let mut closed = false;
    for line in markdown_lines.by_ref() {
        if line == "---" {
            closed = true;
            break;
        }
        frontmatter.push(line);
    }
    if !closed {
        return metadata;
    }
    let lines = frontmatter;
    let mut list_key: Option<String> = None;
    let mut top_level_key: Option<String> = None;
    let mut metadata_child_indentation: Option<usize> = None;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let indentation = leading_whitespace(line);
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("- ") {
            if list_key.as_deref() == Some("triggers") {
                metadata.triggers.push(unquote(value));
            }
            index += 1;
            continue;
        }
        if indentation > 0 {
            if top_level_key.as_deref() == Some("metadata") {
                let Some((key, value)) = trimmed.split_once(':') else {
                    index += 1;
                    continue;
                };
                let child_indentation = *metadata_child_indentation.get_or_insert(indentation);
                if indentation != child_indentation {
                    index += 1;
                    continue;
                }
                let key = key.trim();
                let value = value.trim();
                list_key = Some(key.to_owned());
                if key == "skillroster-routing-triggers" {
                    if let Some(value) = quoted_metadata_string(value) {
                        extend_routing_trigger_values(&mut metadata, value);
                    }
                }
            }
            index += 1;
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            index += 1;
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        top_level_key = Some(key.to_owned());
        metadata_child_indentation = None;
        list_key = Some(key.to_owned());
        if value.is_empty() {
            index += 1;
            continue;
        }
        if value.starts_with(['>', '|']) {
            let mut block = Vec::new();
            index += 1;
            while index < lines.len() {
                let continuation = lines[index];
                if !continuation.trim().is_empty()
                    && leading_whitespace(continuation) <= indentation
                {
                    break;
                }
                if !continuation.trim().is_empty() {
                    block.push(continuation.trim());
                }
                index += 1;
            }
            set_metadata_scalar(&mut metadata, key, block.join(" "));
            continue;
        }
        match key {
            "triggers" => {
                extend_trigger_values(&mut metadata, value);
            }
            _ => set_metadata_scalar(&mut metadata, key, unquote(value)),
        }
        index += 1;
    }
    metadata
}

fn extend_trigger_values(metadata: &mut SkillMetadata, value: &str) {
    metadata.triggers.extend(
        value
            .trim_matches(['[', ']'])
            .split(',')
            .map(unquote)
            .filter(|value| !value.is_empty()),
    );
}

fn extend_routing_trigger_values(metadata: &mut SkillMetadata, value: &str) {
    metadata.triggers.extend(
        value
            .split(';')
            .map(unquote)
            .filter(|value| !value.is_empty()),
    );
}

fn quoted_metadata_string(value: &str) -> Option<&str> {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
}

fn leading_whitespace(value: &str) -> usize {
    value
        .chars()
        .take_while(|character| character.is_whitespace())
        .count()
}

fn set_metadata_scalar(metadata: &mut SkillMetadata, key: &str, value: String) {
    if value.is_empty() {
        return;
    }
    match key {
        "name" => metadata.name = Some(value),
        "description" => metadata.description = Some(value),
        "source" | "repository" => metadata.source = Some(value),
        "version" => metadata.version = Some(value),
        "revision" | "rev" | "commit" => metadata.revision = Some(value),
        _ => {}
    }
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches(['\'', '"']).trim().to_string()
}

fn summarize_markdown(markdown: &str, limit: usize) -> String {
    let mut lines = markdown.lines();
    let body = if lines.next() == Some("---") {
        let mut closed = false;
        for line in lines.by_ref() {
            if line == "---" {
                closed = true;
                break;
            }
        }
        if closed {
            lines.collect::<Vec<_>>()
        } else {
            markdown.lines().collect::<Vec<_>>()
        }
    } else {
        markdown.lines().collect::<Vec<_>>()
    };
    let summary = body
        .into_iter()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ");
    summary.chars().take(limit).collect()
}

fn read_bounded(path: &Path, maximum: u64) -> io::Result<(String, Option<u64>)> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("file exceeds {maximum} byte safety limit"),
        ));
    }
    let mut content = String::new();
    File::open(path)?.read_to_string(&mut content)?;
    Ok((content, modified_unix(&metadata)))
}

fn modified_unix(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn stable_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug)]
struct PackageFingerprint {
    digest: String,
    content_identity_digest: String,
    completeness: FingerprintCompleteness,
    detail: Option<String>,
    checkpoint: Option<Vec<PackageFileCheckpoint>>,
    executable_relative_files: Vec<PathBuf>,
}

const NON_UNICODE_IDENTITY_DETAIL: &str = "non-Unicode path cannot participate in stable identity";

#[derive(Debug)]
struct NonUnicodeIdentityPath;

impl std::fmt::Display for NonUnicodeIdentityPath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(NON_UNICODE_IDENTITY_DETAIL)
    }
}

impl std::error::Error for NonUnicodeIdentityPath {}

fn non_unicode_identity_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, NonUnicodeIdentityPath)
}

pub(crate) fn is_non_unicode_identity_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::InvalidData
        && error
            .get_ref()
            .and_then(|source| source.downcast_ref::<NonUnicodeIdentityPath>())
            .is_some()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PackageFileCheckpoint {
    relative_path: PathBuf,
    kind: PackageFileKind,
    len: u64,
    modified_nanos: Option<u128>,
    readonly: bool,
    mode: u32,
    symlink_target: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PackageFileKind {
    File,
    Symlink,
}

fn package_file_checkpoints(
    files: &[(PathBuf, PathBuf, fs::FileType)],
) -> io::Result<Vec<PackageFileCheckpoint>> {
    files
        .iter()
        .map(|(relative_path, path, _)| {
            if relative_path.to_str().is_none() {
                return Err(non_unicode_identity_error());
            }
            let metadata = fs::symlink_metadata(path)?;
            let file_type = metadata.file_type();
            let symlink_target = file_type
                .is_symlink()
                .then(|| fs::read_link(path))
                .transpose()?;
            if symlink_target
                .as_deref()
                .is_some_and(|target| target.to_str().is_none())
            {
                return Err(non_unicode_identity_error());
            }
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode()
            };
            #[cfg(not(unix))]
            let mode = 0;
            Ok(PackageFileCheckpoint {
                relative_path: relative_path.clone(),
                kind: if file_type.is_symlink() {
                    PackageFileKind::Symlink
                } else {
                    PackageFileKind::File
                },
                len: metadata.len(),
                modified_nanos: metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos()),
                readonly: metadata.permissions().readonly(),
                mode,
                symlink_target,
            })
        })
        .collect()
}

fn package_checkpoint(directory: &Path) -> io::Result<Option<Vec<PackageFileCheckpoint>>> {
    let mut files = Vec::new();
    let complete = collect_skill_files(directory, directory, 0, 8, &mut files)?;
    if !complete {
        return Ok(None);
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    package_file_checkpoints(&files).map(Some)
}

fn executable_files_from_checkpoints(checkpoints: &[PackageFileCheckpoint]) -> Vec<PathBuf> {
    checkpoints
        .iter()
        .filter_map(|checkpoint| {
            let extension_is_script = checkpoint
                .relative_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "sh" | "bash"
                            | "zsh"
                            | "fish"
                            | "py"
                            | "pl"
                            | "rb"
                            | "js"
                            | "mjs"
                            | "cjs"
                            | "ps1"
                            | "bat"
                            | "cmd"
                    )
                });
            (checkpoint.kind == PackageFileKind::File
                && (extension_is_script || checkpoint.mode & 0o111 != 0))
                .then(|| checkpoint.relative_path.clone())
        })
        .collect()
}

fn digest_skill_directory(directory: &Path) -> io::Result<PackageFingerprint> {
    let mut files = Vec::new();
    let complete = collect_skill_files(directory, directory, 0, 8, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let checkpoints = package_file_checkpoints(&files)?;
    let executable_relative_files = executable_files_from_checkpoints(&checkpoints);

    let mut digest = Sha256::new();
    let mut content_identity_digest = Sha256::new();
    let mut total_bytes = 0_u64;
    for (relative_path, path, file_type) in files {
        let is_source_control_metadata = relative_path == Path::new(".gitignore");
        let relative_path_bytes = relative_path
            .to_str()
            .expect("package checkpoints reject non-Unicode paths");
        digest.update(relative_path_bytes.as_bytes());
        digest.update([0]);
        if !is_source_control_metadata {
            content_identity_digest.update(relative_path_bytes.as_bytes());
            content_identity_digest.update([0]);
        }
        if file_type.is_symlink() {
            let target = fs::read_link(path)?;
            let target = target.to_str().ok_or_else(non_unicode_identity_error)?;
            digest.update(b"symlink\0");
            digest.update(target.as_bytes());
            if !is_source_control_metadata {
                content_identity_digest.update(b"symlink\0");
                content_identity_digest.update(target.as_bytes());
            }
        } else {
            let metadata = fs::metadata(&path)?;
            total_bytes = total_bytes.saturating_add(metadata.len());
            if total_bytes > MAX_SKILL_PACKAGE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "package exceeds {MAX_SKILL_PACKAGE_BYTES} byte fingerprint safety limit"
                    ),
                ));
            }
            let bytes = fs::read(path)?;
            digest.update(&bytes);
            if !is_source_control_metadata {
                content_identity_digest.update(&bytes);
            }
        }
        digest.update([0xff]);
        if !is_source_control_metadata {
            content_identity_digest.update([0xff]);
        }
    }
    Ok(PackageFingerprint {
        digest: format!("{:x}", digest.finalize()),
        content_identity_digest: format!("{:x}", content_identity_digest.finalize()),
        completeness: if complete {
            FingerprintCompleteness::Complete
        } else {
            FingerprintCompleteness::Bounded
        },
        detail: (!complete).then(|| "Skill package fingerprint was bounded at depth 8".into()),
        checkpoint: complete.then_some(checkpoints),
        executable_relative_files,
    })
}

fn collect_skill_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    max_depth: usize,
    output: &mut Vec<(PathBuf, PathBuf, fs::FileType)>,
) -> io::Result<bool> {
    if depth > max_depth {
        return Ok(false);
    }
    let mut complete = true;
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        if matches!(
            name.to_str(),
            Some(".git" | "target" | "node_modules" | ".DS_Store")
        ) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if file_type.is_dir() && !file_type.is_symlink() {
            complete &= collect_skill_files(root, &path, depth + 1, max_depth, output)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            let relative = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_path_buf();
            output.push((relative, path, file_type));
        }
    }
    Ok(complete)
}

fn scan_sessions(agent: AgentKind, roots: &[PathBuf], result: &mut ScanResult) {
    let mut coverage = SessionCoverage {
        agent,
        roots_present: 0,
        roots_missing: 0,
        roots_inaccessible: 0,
        files_discovered: 0,
        files_observed: 0,
        files_partially_observed: 0,
        files_skipped: 0,
        denominator_reliable: false,
        bytes_observed: 0,
        lines_observed: 0,
        truncated: false,
        discovery_truncated: false,
        first_seen_unix: None,
        last_seen_unix: None,
        limitations: Some(Vec::new()),
    };
    let mut limitations =
        BTreeMap::<SessionCoverageLimitationCode, SessionCoverageLimitation>::new();
    let mut skill_ids_by_name = BTreeMap::<String, BTreeSet<String>>::new();
    for skill in &result.skills {
        skill_ids_by_name
            .entry(skill.name.to_ascii_lowercase())
            .or_default()
            .insert(skill.id.clone());
    }
    let skill_lookup = skill_ids_by_name
        .iter()
        .filter(|(_, ids)| ids.len() == 1)
        .map(|(name, ids)| {
            (
                ids.iter().next().expect("length checked").clone(),
                name.clone(),
            )
        })
        .collect::<Vec<_>>();
    let patterns = skill_lookup
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>();
    let matcher = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .build(&patterns)
        .ok();
    let mut skill_ids_by_reference = BTreeMap::<String, BTreeSet<String>>::new();
    for placement in &result.placements {
        let mut entrypoints = vec![placement.entrypoint.clone()];
        if let Some(target) = &placement.link_target {
            entrypoints.push(target.join("SKILL.md"));
        }
        if placement.link_status != LinkStatus::EscapesRoot {
            if let Ok(resolved) = fs::canonicalize(&placement.entrypoint) {
                entrypoints.push(resolved);
            }
        }
        for entrypoint in entrypoints {
            let Some(entrypoint) = entrypoint.to_str() else {
                continue;
            };
            skill_ids_by_reference
                .entry(normalize_reference_text(entrypoint))
                .or_default()
                .insert(placement.skill_id.clone());
        }
    }
    let reference_lookup = skill_ids_by_reference
        .into_iter()
        .filter_map(|(reference, ids)| {
            (ids.len() == 1).then(|| (ids.into_iter().next().expect("length checked"), reference))
        })
        .collect::<Vec<_>>();
    let reference_patterns = reference_lookup
        .iter()
        .map(|(_, reference)| reference.as_str())
        .collect::<Vec<_>>();
    let reference_matcher = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .build(&reference_patterns)
        .ok();
    let mut events = BTreeMap::<(String, UsageStage, String, Option<u64>), UsageEvidence>::new();
    let mut bytes_observed = 0_u64;
    let mut lines_observed = 0_usize;

    for root in roots {
        let status = match fs::read_dir(root) {
            Ok(_) => {
                coverage.roots_present += 1;
                RootStatus::Included
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => RootStatus::Missing,
            Err(_) => RootStatus::Inaccessible,
        };
        match status {
            RootStatus::Missing => {
                coverage.roots_missing += 1;
                record_session_limitation(
                    &mut limitations,
                    SessionCoverageLimitationCode::RootMissing,
                    SessionCoverageScope::Root,
                    SessionCoverageCountKind::Exact,
                    Some(coverage.roots_missing as u64),
                    None,
                    SessionCoverageLimitationSource::Roots,
                );
            }
            RootStatus::Inaccessible => {
                coverage.roots_inaccessible += 1;
                record_session_limitation(
                    &mut limitations,
                    SessionCoverageLimitationCode::RootInaccessible,
                    SessionCoverageScope::Root,
                    SessionCoverageCountKind::Exact,
                    Some(coverage.roots_inaccessible as u64),
                    None,
                    SessionCoverageLimitationSource::Roots,
                );
            }
            RootStatus::Included | RootStatus::Excluded => {}
        }
        result.roots.push(RootObservation {
            agent: Some(agent),
            kind: RootKind::Sessions,
            path: root.clone(),
            status,
            explicit: false,
            detail: None,
            discovery_complete: true,
        });
        if status != RootStatus::Included {
            continue;
        }
        let mut files = Vec::new();
        let discovery = match collect_session_files(
            root,
            0,
            6,
            MAX_SESSION_DISCOVERY_FILES_PER_ROOT + 1,
            &mut files,
        ) {
            Ok(discovery) => discovery,
            Err(_) => {
                record_session_limitation(
                    &mut limitations,
                    SessionCoverageLimitationCode::DiscoveryWalkFailure,
                    SessionCoverageScope::Root,
                    SessionCoverageCountKind::Unknown,
                    None,
                    None,
                    SessionCoverageLimitationSource::SessionDiscovery,
                );
                coverage.files_skipped += 1;
                coverage.truncated = true;
                continue;
            }
        };
        if discovery.file_limit {
            record_session_limitation(
                &mut limitations,
                SessionCoverageLimitationCode::DiscoveryFileLimit,
                SessionCoverageScope::Root,
                SessionCoverageCountKind::LowerBound,
                Some(files.len() as u64),
                Some(MAX_SESSION_DISCOVERY_FILES_PER_ROOT as u64),
                SessionCoverageLimitationSource::SessionDiscovery,
            );
            coverage.discovery_truncated = true;
            coverage.truncated = true;
        }
        if discovery.depth_limit {
            record_session_limitation(
                &mut limitations,
                SessionCoverageLimitationCode::DiscoveryDepthLimit,
                SessionCoverageScope::Root,
                SessionCoverageCountKind::Unknown,
                None,
                Some(6),
                SessionCoverageLimitationSource::SessionDiscovery,
            );
            coverage.discovery_truncated = true;
            coverage.truncated = true;
        }
        let discovered_count = files.len();
        coverage.files_discovered = coverage.files_discovered.saturating_add(discovered_count);
        files.sort_by_cached_key(|path| {
            (std::cmp::Reverse(session_file_modified(path)), path.clone())
        });
        files.truncate(MAX_SESSION_DISCOVERY_FILES_PER_ROOT);
        if files.len() > MAX_SESSION_FILES_PER_ROOT {
            coverage.files_skipped = coverage
                .files_skipped
                .saturating_add(discovered_count - MAX_SESSION_FILES_PER_ROOT);
            coverage.truncated = true;
            record_session_limitation(
                &mut limitations,
                SessionCoverageLimitationCode::SampledFileLimit,
                SessionCoverageScope::Root,
                discovered_count_kind(&discovery),
                Some(discovered_count as u64),
                Some(MAX_SESSION_FILES_PER_ROOT as u64),
                SessionCoverageLimitationSource::SessionSampling,
            );
        }
        files.truncate(MAX_SESSION_FILES_PER_ROOT);
        for (index, file) in files.iter().enumerate() {
            let remaining_bytes = MAX_SESSION_BYTES_PER_AGENT.saturating_sub(bytes_observed);
            if remaining_bytes == 0 || lines_observed >= MAX_SESSION_LINES_PER_AGENT {
                coverage.files_skipped = coverage
                    .files_skipped
                    .saturating_add(files.len().saturating_sub(index));
                coverage.truncated = true;
                if remaining_bytes == 0 {
                    record_session_limitation(
                        &mut limitations,
                        SessionCoverageLimitationCode::SampledByteLimit,
                        SessionCoverageScope::Agent,
                        SessionCoverageCountKind::Exact,
                        Some(bytes_observed),
                        Some(MAX_SESSION_BYTES_PER_AGENT),
                        SessionCoverageLimitationSource::SessionSampling,
                    );
                }
                if lines_observed >= MAX_SESSION_LINES_PER_AGENT {
                    record_session_limitation(
                        &mut limitations,
                        SessionCoverageLimitationCode::SampledLineLimit,
                        SessionCoverageScope::Agent,
                        SessionCoverageCountKind::Exact,
                        Some(lines_observed as u64),
                        Some(MAX_SESSION_LINES_PER_AGENT as u64),
                        SessionCoverageLimitationSource::SessionSampling,
                    );
                }
                break;
            }
            let Some(file_identity) = file.to_str() else {
                coverage.files_skipped += 1;
                record_session_limitation(
                    &mut limitations,
                    SessionCoverageLimitationCode::FilePathNotUnicode,
                    SessionCoverageScope::File,
                    SessionCoverageCountKind::Exact,
                    Some(coverage.files_skipped as u64),
                    None,
                    SessionCoverageLimitationSource::SessionSampling,
                );
                continue;
            };
            let metadata = match fs::metadata(file) {
                Ok(metadata) => metadata,
                _ => {
                    coverage.files_skipped += 1;
                    record_session_limitation(
                        &mut limitations,
                        SessionCoverageLimitationCode::FileMetadataFailure,
                        SessionCoverageScope::File,
                        SessionCoverageCountKind::Unknown,
                        None,
                        None,
                        SessionCoverageLimitationSource::SessionSampling,
                    );
                    continue;
                }
            };
            let is_json = file.extension().and_then(|extension| extension.to_str()) == Some("json");
            let sample_limit = remaining_bytes.min(MAX_SESSION_BYTES_PER_FILE);
            let file_byte_limit_applies = metadata.len() > MAX_SESSION_BYTES_PER_FILE;
            let agent_byte_limit_applies =
                remaining_bytes < MAX_SESSION_BYTES_PER_FILE && remaining_bytes < metadata.len();
            if agent_byte_limit_applies {
                record_session_limitation(
                    &mut limitations,
                    SessionCoverageLimitationCode::SampledByteLimit,
                    SessionCoverageScope::Agent,
                    SessionCoverageCountKind::LowerBound,
                    Some(bytes_observed),
                    Some(MAX_SESSION_BYTES_PER_AGENT),
                    SessionCoverageLimitationSource::SessionSampling,
                );
            }
            let (sample, sample_bytes, alignment_lost) =
                match read_session_tail_with_facts(file, metadata.len(), sample_limit, !is_json) {
                    Ok(sample) => sample,
                    Err(_) => {
                        coverage.files_skipped += 1;
                        record_session_limitation(
                            &mut limitations,
                            SessionCoverageLimitationCode::FileReadFailure,
                            SessionCoverageScope::File,
                            SessionCoverageCountKind::Unknown,
                            None,
                            None,
                            SessionCoverageLimitationSource::SessionSampling,
                        );
                        continue;
                    }
                };
            if alignment_lost {
                record_session_limitation(
                    &mut limitations,
                    SessionCoverageLimitationCode::LineAlignmentLoss,
                    SessionCoverageScope::File,
                    SessionCoverageCountKind::Unknown,
                    None,
                    None,
                    SessionCoverageLimitationSource::SessionJsonl,
                );
            }
            if file_byte_limit_applies {
                record_session_limitation(
                    &mut limitations,
                    SessionCoverageLimitationCode::FileByteLimit,
                    SessionCoverageScope::File,
                    SessionCoverageCountKind::Exact,
                    Some(sample_bytes),
                    Some(MAX_SESSION_BYTES_PER_FILE),
                    SessionCoverageLimitationSource::SessionSampling,
                );
            }
            let mut file_partially_observed = sample_bytes < metadata.len();
            if file_partially_observed {
                coverage.truncated = true;
            }
            if sample_bytes == 0 && metadata.len() != 0 {
                coverage.files_skipped += 1;
                record_session_limitation(
                    &mut limitations,
                    SessionCoverageLimitationCode::FileZeroRead,
                    SessionCoverageScope::File,
                    SessionCoverageCountKind::Exact,
                    Some(0),
                    Some(metadata.len()),
                    SessionCoverageLimitationSource::SessionSampling,
                );
                continue;
            }
            bytes_observed = bytes_observed.saturating_add(sample_bytes);
            let timestamp = modified_unix(&metadata);
            coverage.files_observed += 1;
            update_window(
                &mut coverage.first_seen_unix,
                &mut coverage.last_seen_unix,
                timestamp,
            );
            let source_path_digest = stable_digest(file_identity.as_bytes());
            let sample = String::from_utf8_lossy(&sample);
            let complete_json = is_json
                && sample_bytes == metadata.len()
                && serde_json::from_str::<serde_json::Value>(&sample).is_ok();
            let records = if complete_json {
                let physical_lines = sample.lines().count().max(1);
                vec![(sample.into_owned(), physical_lines)]
            } else if is_json {
                let extracted = extract_complete_json_objects_with_facts(&sample);
                if extracted.extraction_limited {
                    record_session_limitation(
                        &mut limitations,
                        SessionCoverageLimitationCode::JsonRecordLimit,
                        SessionCoverageScope::File,
                        SessionCoverageCountKind::LowerBound,
                        Some(extracted.records.len() as u64),
                        Some(MAX_SESSION_LINES_PER_AGENT as u64),
                        SessionCoverageLimitationSource::SessionJson,
                    );
                }
                if extracted.parse_boundary {
                    record_session_limitation(
                        &mut limitations,
                        SessionCoverageLimitationCode::JsonExtractionLimit,
                        SessionCoverageScope::File,
                        SessionCoverageCountKind::Unknown,
                        None,
                        Some(8 * MAX_SESSION_BYTES_PER_FILE),
                        SessionCoverageLimitationSource::SessionJson,
                    );
                }
                extracted.records
            } else {
                sample
                    .lines()
                    .map(|line| (line.to_owned(), 1))
                    .collect::<Vec<_>>()
            };
            for (line, physical_lines) in records {
                if lines_observed.saturating_add(physical_lines) > MAX_SESSION_LINES_PER_AGENT {
                    file_partially_observed = true;
                    coverage.truncated = true;
                    record_session_limitation(
                        &mut limitations,
                        SessionCoverageLimitationCode::SampledLineLimit,
                        SessionCoverageScope::Agent,
                        SessionCoverageCountKind::Exact,
                        Some(lines_observed as u64),
                        Some(MAX_SESSION_LINES_PER_AGENT as u64),
                        SessionCoverageLimitationSource::SessionSampling,
                    );
                    break;
                }
                lines_observed += physical_lines;
                let record_timestamp = session_record_timestamp(agent, &line);
                let record_month = record_timestamp.and_then(month_start_unix);
                let observations = session_record_observations(agent, &line);
                if !observations.is_empty() {
                    let mut seen_event_skill_stages = BTreeSet::new();
                    for observation in observations {
                        let stage = usage_stage(observation.signal);
                        let quality = EvidenceQuality::Observed;
                        let mut observed_skill_ids = if observation.explicit_references.is_empty() {
                            observed_reference_skill_ids(
                                &observation.record_text,
                                reference_matcher.as_ref(),
                                &reference_lookup,
                            )
                        } else {
                            BTreeSet::new()
                        };
                        for reference in observation.explicit_references {
                            if result.skills.iter().any(|skill| skill.id == reference) {
                                observed_skill_ids.insert(reference.clone());
                            }
                            if let Some(ids) =
                                skill_ids_by_name.get(&reference.to_ascii_lowercase())
                            {
                                if ids.len() == 1 {
                                    observed_skill_ids.extend(ids.iter().cloned());
                                }
                            }
                        }
                        for skill_id in observed_skill_ids {
                            if !seen_event_skill_stages.insert((
                                observation.event_index,
                                stage,
                                skill_id.clone(),
                            )) {
                                continue;
                            }
                            let key = (
                                skill_id.clone(),
                                stage,
                                source_path_digest.clone(),
                                record_month,
                            );
                            let event = events.entry(key).or_insert_with(|| UsageEvidence {
                                agent,
                                skill_id,
                                stage,
                                quality,
                                event_count: 0,
                                first_seen_unix: None,
                                last_seen_unix: None,
                                month_start_unix: record_month,
                                source_path_digest: source_path_digest.clone(),
                            });
                            event.event_count += 1;
                            update_window(
                                &mut event.first_seen_unix,
                                &mut event.last_seen_unix,
                                record_timestamp,
                            );
                        }
                    }
                    continue;
                }
                let lower = line.to_ascii_lowercase();
                let Some(matcher) = &matcher else { continue };
                for matched in matcher.find_iter(&lower) {
                    let (skill_id, _) = &skill_lookup[matched.pattern().as_usize()];
                    let stage = UsageStage::Exposed;
                    let quality = EvidenceQuality::Inferred;
                    let key = (
                        skill_id.clone(),
                        stage,
                        source_path_digest.clone(),
                        record_month,
                    );
                    let event = events.entry(key).or_insert_with(|| UsageEvidence {
                        agent,
                        skill_id: skill_id.clone(),
                        stage,
                        quality,
                        event_count: 0,
                        first_seen_unix: None,
                        last_seen_unix: None,
                        month_start_unix: record_month,
                        source_path_digest: source_path_digest.clone(),
                    });
                    event.event_count += 1;
                    update_window(
                        &mut event.first_seen_unix,
                        &mut event.last_seen_unix,
                        record_timestamp,
                    );
                }
            }
            if file_partially_observed {
                coverage.files_partially_observed += 1;
            }
        }
    }
    coverage.bytes_observed = bytes_observed;
    coverage.lines_observed = lines_observed;
    coverage.limitations = Some(limitations.into_values().collect());
    coverage.denominator_reliable = coverage.roots_present > 0
        && coverage.roots_missing == 0
        && coverage.roots_inaccessible == 0
        && coverage.files_skipped == 0
        && coverage.files_partially_observed == 0
        && !coverage.discovery_truncated
        && coverage.limitations.as_ref().is_none_or(Vec::is_empty);
    result.usage.extend(events.into_values());
    result.coverage.push(coverage);
}

fn session_file_modified(path: &Path) -> u64 {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| modified_unix(&metadata))
        .unwrap_or_default()
}

fn record_session_limitation(
    limitations: &mut BTreeMap<SessionCoverageLimitationCode, SessionCoverageLimitation>,
    code: SessionCoverageLimitationCode,
    scope: SessionCoverageScope,
    count_kind: SessionCoverageCountKind,
    observed: Option<u64>,
    limit: Option<u64>,
    source: SessionCoverageLimitationSource,
) {
    let fact = limitations
        .entry(code)
        .or_insert_with(|| SessionCoverageLimitation {
            code,
            scope,
            count_kind,
            observed,
            limit,
            unit: limitation_unit(code),
            source,
        });
    debug_assert_eq!(fact.scope, scope);
    debug_assert_eq!(fact.source, source);
    fact.count_kind = fact.count_kind.max(count_kind);
    if let Some(value) = observed {
        fact.observed = Some(fact.observed.unwrap_or_default().max(value));
    }
    if let Some(value) = limit {
        fact.limit = Some(fact.limit.unwrap_or(value).min(value));
    }
}

const fn limitation_unit(code: SessionCoverageLimitationCode) -> SessionCoverageUnit {
    match code {
        SessionCoverageLimitationCode::RootMissing
        | SessionCoverageLimitationCode::RootInaccessible => SessionCoverageUnit::Roots,
        SessionCoverageLimitationCode::DiscoveryFileLimit
        | SessionCoverageLimitationCode::SampledFileLimit
        | SessionCoverageLimitationCode::FileMetadataFailure
        | SessionCoverageLimitationCode::FileReadFailure
        | SessionCoverageLimitationCode::FileZeroRead
        | SessionCoverageLimitationCode::FilePathNotUnicode => SessionCoverageUnit::Files,
        SessionCoverageLimitationCode::DiscoveryDepthLimit => SessionCoverageUnit::Depth,
        SessionCoverageLimitationCode::DiscoveryWalkFailure => SessionCoverageUnit::Walks,
        SessionCoverageLimitationCode::SampledByteLimit
        | SessionCoverageLimitationCode::FileByteLimit => SessionCoverageUnit::Bytes,
        SessionCoverageLimitationCode::SampledLineLimit
        | SessionCoverageLimitationCode::LineAlignmentLoss => SessionCoverageUnit::Lines,
        SessionCoverageLimitationCode::JsonExtractionLimit => SessionCoverageUnit::Bytes,
        SessionCoverageLimitationCode::JsonRecordLimit => SessionCoverageUnit::Records,
    }
}

fn read_session_tail_with_facts(
    path: &Path,
    file_bytes: u64,
    maximum: u64,
    align_to_line: bool,
) -> io::Result<(Vec<u8>, u64, bool)> {
    let mut file = File::open(path)?;
    let offset = file_bytes.saturating_sub(maximum);
    file.seek(SeekFrom::Start(offset))?;
    let mut sample = Vec::with_capacity(maximum as usize);
    file.take(maximum).read_to_end(&mut sample)?;
    let bytes_read = sample.len() as u64;
    let mut alignment_lost = false;
    if offset != 0 && align_to_line {
        if let Some(newline) = sample.iter().position(|byte| *byte == b'\n') {
            alignment_lost = newline > 0;
            sample.drain(..=newline);
        } else {
            alignment_lost = !sample.is_empty();
            sample.clear();
        }
    }
    Ok((sample, bytes_read, alignment_lost))
}

#[derive(Default)]
struct JsonExtractionFacts {
    records: Vec<(String, usize)>,
    extraction_limited: bool,
    parse_boundary: bool,
}

fn extract_complete_json_objects_with_facts(sample: &str) -> JsonExtractionFacts {
    let bytes = sample.as_bytes();
    let mut candidates = Vec::<(usize, usize)>::new();
    // A tail can begin either inside or outside a JSON string. Try both bounded
    // parser states, then retain only the largest non-overlapping valid values.
    for initial_in_string in [false, true] {
        let mut stack = Vec::new();
        let mut in_string = initial_in_string;
        let mut escaped = false;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b'{' => stack.push(index),
                b'}' => {
                    let Some(start) = stack.pop() else {
                        continue;
                    };
                    candidates.push((start, index + 1));
                }
                _ => {}
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates.sort_by_key(|(start, end)| (*start, std::cmp::Reverse(*end)));
    let mut selected = Vec::<(usize, usize)>::new();
    let mut covered_until = 0;
    let mut parse_bytes = 0_u64;
    let mut parse_boundary = false;
    for (start, end) in candidates {
        if end <= covered_until {
            continue;
        }
        let candidate_bytes = (end - start) as u64;
        if parse_bytes.saturating_add(candidate_bytes) > 8 * MAX_SESSION_BYTES_PER_FILE {
            parse_boundary = true;
            continue;
        }
        parse_bytes += candidate_bytes;
        if serde_json::from_slice::<serde_json::Value>(&bytes[start..end]).is_ok() {
            selected.push((start, end));
            covered_until = covered_until.max(end);
        }
    }
    selected.sort_by_key(|(start, _)| *start);
    let extraction_limited = selected.len() > MAX_SESSION_LINES_PER_AGENT;
    if extraction_limited {
        selected.drain(..selected.len() - MAX_SESSION_LINES_PER_AGENT);
    }
    JsonExtractionFacts {
        records: selected
            .into_iter()
            .map(|(start, end)| {
                let value = sample[start..end].to_owned();
                let lines = value.lines().count().max(1);
                (value, lines)
            })
            .collect(),
        extraction_limited,
        parse_boundary,
    }
}

#[cfg(test)]
fn extract_complete_json_objects(sample: &str) -> Vec<(String, usize)> {
    extract_complete_json_objects_with_facts(sample).records
}

#[derive(Default)]
struct SessionDiscoveryOutcome {
    file_limit: bool,
    depth_limit: bool,
}

const fn discovered_count_kind(discovery: &SessionDiscoveryOutcome) -> SessionCoverageCountKind {
    if discovery.file_limit || discovery.depth_limit {
        SessionCoverageCountKind::LowerBound
    } else {
        SessionCoverageCountKind::Exact
    }
}

fn collect_session_files(
    directory: &Path,
    depth: usize,
    max_depth: usize,
    maximum: usize,
    output: &mut Vec<PathBuf>,
) -> io::Result<SessionDiscoveryOutcome> {
    let mut outcome = SessionDiscoveryOutcome::default();
    collect_session_files_with_outcome(directory, depth, max_depth, maximum, output, &mut outcome)?;
    Ok(outcome)
}

fn collect_session_files_with_outcome(
    directory: &Path,
    depth: usize,
    max_depth: usize,
    maximum: usize,
    output: &mut Vec<PathBuf>,
    outcome: &mut SessionDiscoveryOutcome,
) -> io::Result<()> {
    if depth > max_depth {
        outcome.depth_limit = true;
        return Ok(());
    }
    if output.len() >= maximum {
        outcome.file_limit = true;
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        if output.len() >= maximum {
            outcome.file_limit = true;
            return Ok(());
        }
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_session_files_with_outcome(
                &path,
                depth + 1,
                max_depth,
                maximum,
                output,
                outcome,
            )?;
        } else if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("json" | "jsonl" | "log")
        ) {
            output.push(path);
        }
    }
    Ok(())
}

const fn usage_stage(signal: SessionSignal) -> UsageStage {
    match signal {
        SessionSignal::Outcome => UsageStage::Outcome,
        SessionSignal::Applied => UsageStage::Applied,
        SessionSignal::Loaded => UsageStage::Loaded,
        SessionSignal::Matched => UsageStage::Matched,
    }
}

fn observed_reference_skill_ids(
    line: &str,
    matcher: Option<&AhoCorasick>,
    reference_lookup: &[(String, String)],
) -> BTreeSet<String> {
    let Some(matcher) = matcher else {
        return BTreeSet::new();
    };
    let decoded = decoded_record_text(line).unwrap_or_else(|| line.to_owned());
    let searchable = normalize_reference_text(&decoded);
    matcher
        .find_iter(&searchable)
        .map(|matched| reference_lookup[matched.pattern().as_usize()].0.clone())
        .collect()
}

fn normalize_reference_text(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}

fn decoded_record_text(line: &str) -> Option<String> {
    fn collect_strings(value: &serde_json::Value, depth: usize, output: &mut Vec<String>) {
        match value {
            serde_json::Value::String(value) => {
                output.push(value.clone());
                if depth < 4 {
                    if let Ok(decoded) = serde_json::from_str::<serde_json::Value>(value) {
                        collect_strings(&decoded, depth + 1, output);
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    collect_strings(value, depth, output);
                }
            }
            serde_json::Value::Object(values) => {
                for value in values.values() {
                    collect_strings(value, depth, output);
                }
            }
            _ => {}
        }
    }

    let value = serde_json::from_str(line).ok()?;
    let mut strings = Vec::new();
    collect_strings(&value, 0, &mut strings);
    Some(strings.join("\n"))
}

fn update_window(first: &mut Option<u64>, last: &mut Option<u64>, timestamp: Option<u64>) {
    let Some(timestamp) = timestamp else { return };
    *first = Some(first.map_or(timestamp, |current| current.min(timestamp)));
    *last = Some(last.map_or(timestamp, |current| current.max(timestamp)));
}

fn session_record_timestamp(agent: AgentKind, record: &str) -> Option<u64> {
    let value = serde_json::from_str::<serde_json::Value>(record).ok()?;
    let mut timestamps = BTreeSet::new();
    for field in ["timestamp", "created_at", "createdAt"] {
        if let Some(timestamp) = value.get(field).and_then(timestamp_value_unix) {
            timestamps.insert(timestamp);
        }
    }
    // Codex wraps some JSONL record metadata in `payload`. This is an Agent
    // envelope, unlike arbitrary tool arguments or message content, which must
    // never be treated as event time.
    if agent == AgentKind::Codex {
        if let Some(payload) = value.get("payload") {
            for field in ["timestamp", "created_at", "createdAt"] {
                if let Some(timestamp) = payload.get(field).and_then(timestamp_value_unix) {
                    timestamps.insert(timestamp);
                }
            }
        }
    }
    (timestamps.len() == 1).then(|| *timestamps.iter().next().expect("length checked"))
}

fn timestamp_value_unix(value: &serde_json::Value) -> Option<u64> {
    if let Some(value) = value.as_f64() {
        return normalize_numeric_unix_timestamp(value);
    }
    let value = value.as_str()?;
    DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|timestamp| timestamp.timestamp().try_into().ok())
        .or_else(|| {
            value
                .parse::<f64>()
                .ok()
                .and_then(normalize_numeric_unix_timestamp)
        })
}

fn normalize_numeric_unix_timestamp(value: f64) -> Option<u64> {
    // Numeric timestamp units are not self-describing. Accept only the single
    // seconds/milliseconds/microseconds/nanoseconds interpretation that lands
    // in the era in which local coding-agent sessions can exist.
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    const EARLIEST_AGENT_SESSION: f64 = 946_684_800.0; // 2000-01-01 UTC
    const LATEST_AGENT_SESSION: f64 = 4_102_444_800.0; // 2100-01-01 UTC
    let candidates = [
        value,
        value / 1_000.0,
        value / 1_000_000.0,
        value / 1_000_000_000.0,
    ]
    .into_iter()
    .filter(|candidate| *candidate >= EARLIEST_AGENT_SESSION && *candidate < LATEST_AGENT_SESSION)
    .map(|candidate| candidate.floor() as u64)
    .collect::<BTreeSet<_>>();
    (candidates.len() == 1).then(|| *candidates.iter().next().expect("length checked"))
}

fn month_start_unix(timestamp: u64) -> Option<u64> {
    let timestamp = i64::try_from(timestamp).ok()?;
    let date = Utc.timestamp_opt(timestamp, 0).single()?.date_naive();
    date.with_day(1)?
        .and_hms_opt(0, 0, 0)?
        .and_utc()
        .timestamp()
        .try_into()
        .ok()
}

pub fn skill_search_text(skill: &ScannedSkill) -> String {
    let mut parts = vec![skill.name.clone(), skill.normalized_text.clone()];
    if let Some(description) = &skill.metadata.description {
        parts.push(description.clone());
    }
    parts.extend(skill.metadata.triggers.clone());
    parts.join(" ")
}

fn normalize_search_text(markdown: &str) -> String {
    markdown.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn inspect_skill_identity(entrypoint: &Path) -> io::Result<(String, String)> {
    let directory = entrypoint
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Skill has no directory"))?;
    let (content, _) = read_bounded(entrypoint, MAX_SKILL_FILE_BYTES)?;
    let metadata = parse_skill_markdown(&content);
    let fingerprint = digest_skill_directory(directory)?;
    if fingerprint.completeness != FingerprintCompleteness::Complete {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            fingerprint
                .detail
                .unwrap_or_else(|| "Skill package fingerprint is incomplete".into()),
        ));
    }
    let digest = fingerprint.digest;
    let identity_basis = match (&metadata.source, &metadata.version, &metadata.revision) {
        (Some(source), Some(version), _) => format!("source:{source}@{version}"),
        (Some(source), _, Some(revision)) => format!("source:{source}@{revision}"),
        _ => format!("content:{}", fingerprint.content_identity_digest),
    };
    Ok((
        format!("skill_{}", stable_digest(identity_basis.as_bytes())),
        digest,
    ))
}

pub fn placements_by_skill(scan: &ScanResult) -> BTreeMap<&str, Vec<&SkillPlacement>> {
    let mut grouped = BTreeMap::<&str, Vec<&SkillPlacement>>::new();
    for placement in &scan.placements {
        grouped
            .entry(&placement.skill_id)
            .or_default()
            .push(placement);
    }
    grouped
}

pub fn agents_with_usage(scan: &ScanResult) -> BTreeSet<AgentKind> {
    scan.usage
        .iter()
        .filter(|usage| {
            usage.stage > UsageStage::Exposed && usage.quality == EvidenceQuality::Observed
        })
        .map(|usage| usage.agent)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_directory(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("skillroster-{name}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn plugin_config_requires_one_safe_explicitly_enabled_section() {
        let mut warnings = Vec::new();
        let config = parse_codex_plugin_config(
            r#"
[plugins."browser@openai-bundled"] # enabled browser provider
enabled  =  true
[plugins."disabled@openai-bundled"]
enabled	=	false
[plugins."../escape@openai-bundled"]
enabled = true
[plugins."duplicate@openai-bundled"]
enabled = true
[plugins."duplicate@openai-bundled"]
enabled = true
"#,
            &mut warnings,
        );

        assert_eq!(
            config.enabled,
            BTreeSet::from([("browser".to_owned(), "openai-bundled".to_owned())])
        );
        assert!(
            config
                .blocked
                .contains(&("disabled".to_owned(), "openai-bundled".to_owned()))
        );
        assert!(warnings.iter().any(|warning| warning.contains("unsafe")));
        assert!(warnings.iter().any(|warning| warning.contains("ambiguous")));
    }

    #[test]
    fn installed_remote_plugin_skills_are_searchable_but_not_governable() {
        let home = temp_directory("remote-plugin");
        let plugin = home.join(".codex/plugins/cache/openai-curated-remote/data-analytics");
        let skill = plugin.join("0.2.8/skills/design-kpis");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: design-kpis\ndescription: Design KPI frameworks and guardrails\n---\n",
        )
        .unwrap();
        fs::write(
            plugin.join(".codex-remote-plugin-install.json"),
            r#"{"schema_version":1,"remote_plugin_id":"Plugin_fc9843a6"}"#,
        )
        .unwrap();

        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "design-kpis");
        assert_eq!(result.placements.len(), 1);
        assert!(!result.placements[0].governable);
        assert_eq!(result.placements[0].owned_by_agent, Some(false));
        assert_eq!(
            result.placements[0].mutation_scope,
            Some(MutationScope::ProviderReadOnly)
        );
        assert_eq!(
            result.placements[0].provider.as_deref(),
            Some("data-analytics@openai-curated-remote")
        );
        assert!(result.roots.iter().any(|root| {
            root.detail
                .as_deref()
                .is_some_and(|detail| detail.contains("installed remote plugin"))
        }));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn explicit_disable_hides_an_installed_remote_plugin() {
        let home = temp_directory("disabled-remote-plugin");
        let plugin = home.join(".codex/plugins/cache/openai-curated-remote/data-analytics");
        let skill = plugin.join("0.2.8/skills/design-kpis");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: design-kpis\n---\n").unwrap();
        fs::write(
            plugin.join(".codex-remote-plugin-install.json"),
            r#"{"schema_version":1,"remote_plugin_id":"Plugin_fc9843a6"}"#,
        )
        .unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(
            home.join(".codex/config.toml"),
            "[plugins.\"data-analytics@openai-curated-remote\"]\nenabled = false\n",
        )
        .unwrap();

        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert!(result.skills.is_empty());
        assert!(result.placements.is_empty());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn invalid_remote_plugin_install_marker_fails_closed() {
        let home = temp_directory("invalid-remote-plugin");
        let plugin = home.join(".codex/plugins/cache/openai-curated-remote/data-analytics");
        let skill = plugin.join("0.2.8/skills/design-kpis");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: design-kpis\n---\n").unwrap();
        fs::write(
            plugin.join(".codex-remote-plugin-install.json"),
            r#"{"schema_version":2,"remote_plugin_id":"Plugin_fc9843a6"}"#,
        )
        .unwrap();

        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert!(result.skills.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("unsupported install marker"))
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remote_plugin_install_marker_link_cannot_escape_its_cache() {
        let home = temp_directory("escaping-remote-plugin-marker");
        let outside = temp_directory("outside-remote-plugin-marker");
        let plugin = home.join(".codex/plugins/cache/openai-curated-remote/data-analytics");
        let skill = plugin.join("0.2.8/skills/design-kpis");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: design-kpis\n---\n").unwrap();
        let outside_marker = outside.join("install.json");
        fs::write(
            &outside_marker,
            r#"{"schema_version":1,"remote_plugin_id":"Plugin_fc9843a6"}"#,
        )
        .unwrap();
        std::os::unix::fs::symlink(
            &outside_marker,
            plugin.join(".codex-remote-plugin-install.json"),
        )
        .unwrap();

        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert!(result.skills.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("install marker escapes its cache"))
        );
        fs::remove_dir_all(home).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn ambiguous_installed_remote_plugin_versions_fail_closed() {
        let home = temp_directory("ambiguous-remote-plugin");
        let plugin = home.join(".codex/plugins/cache/openai-curated-remote/data-analytics");
        for version in ["0.2.7", "0.2.8"] {
            let skill = plugin.join(version).join("skills/design-kpis");
            fs::create_dir_all(&skill).unwrap();
            fs::write(skill.join("SKILL.md"), "---\nname: design-kpis\n---\n").unwrap();
        }
        fs::write(
            plugin.join(".codex-remote-plugin-install.json"),
            r#"{"schema_version":1,"remote_plugin_id":"Plugin_fc9843a6"}"#,
        )
        .unwrap();

        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert!(result.skills.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("multiple cached Skill versions"))
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn enabled_codex_plugin_skills_are_searchable_but_not_governable() {
        let home = temp_directory("enabled-plugin");
        let skill = home
            .join(".codex/plugins/cache/openai-bundled/browser/1.0.0/skills")
            .join("control-browser");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: control-browser\ndescription: Use an authenticated browser session\n---\n",
        )
        .unwrap();
        let disabled = home
            .join(".codex/plugins/cache/openai-bundled/disabled/1.0.0/skills")
            .join("not-visible");
        fs::create_dir_all(&disabled).unwrap();
        fs::write(
            disabled.join("SKILL.md"),
            "---\nname: not-visible\ndescription: Must stay hidden\n---\n",
        )
        .unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(
            home.join(".codex/config.toml"),
            "[plugins.\"browser@openai-bundled\"]\nenabled = true\n\n[plugins.\"disabled@openai-bundled\"]\nenabled = false\n",
        )
        .unwrap();

        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "control-browser");
        assert_eq!(result.placements.len(), 1);
        assert_eq!(result.placements[0].agent, None);
        assert!(!result.placements[0].default_exposed);
        assert!(!result.placements[0].governable);
        assert_eq!(
            result.placements[0].provider.as_deref(),
            Some("browser@openai-bundled")
        );
        assert!(result.roots.iter().any(|root| {
            root.detail
                .as_deref()
                .is_some_and(|detail| detail.contains("browser@openai-bundled"))
        }));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn ambiguous_plugin_cache_versions_fail_closed() {
        let home = temp_directory("ambiguous-plugin");
        for version in ["1.0.0", "2.0.0"] {
            let skill = home
                .join(".codex/plugins/cache/openai-bundled/browser")
                .join(version)
                .join("skills/control-browser");
            fs::create_dir_all(&skill).unwrap();
            fs::write(skill.join("SKILL.md"), "---\nname: control-browser\n---\n").unwrap();
        }
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(
            home.join(".codex/config.toml"),
            "[plugins.\"browser@openai-bundled\"]\nenabled = true\n",
        )
        .unwrap();

        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert!(result.skills.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("multiple cached Skill versions"))
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn plugin_cache_link_cannot_escape_the_codex_cache_root() {
        let home = temp_directory("escaping-plugin-cache");
        let outside = temp_directory("outside-plugin-cache");
        let outside_skill = outside.join("1.0.0/skills/escape");
        fs::create_dir_all(&outside_skill).unwrap();
        fs::write(
            outside_skill.join("SKILL.md"),
            "---\nname: escaped-plugin-skill\n---\n",
        )
        .unwrap();
        let marketplace = home.join(".codex/plugins/cache/openai-bundled");
        fs::create_dir_all(&marketplace).unwrap();
        std::os::unix::fs::symlink(&outside, marketplace.join("browser")).unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(
            home.join(".codex/config.toml"),
            "[plugins.\"browser@openai-bundled\"]\nenabled = true\n",
        )
        .unwrap();

        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert!(result.skills.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("no contained local cache"))
        );
        fs::remove_dir_all(home).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn parses_relevant_frontmatter_without_copying_body() {
        let metadata = parse_skill_markdown(
            "---\nname: research\ndescription: Find facts\nsource: example/repo\nrevision: abc\ntriggers: [search, verify]\n---\nSecret prompt body",
        );
        assert_eq!(metadata.name.as_deref(), Some("research"));
        assert_eq!(metadata.revision.as_deref(), Some("abc"));
        assert_eq!(metadata.triggers, ["search", "verify"]);
    }

    #[test]
    fn parses_folded_skill_descriptions_without_treating_nested_metadata_as_top_level() {
        let markdown = "---\nname: agent-skills-manager\ndescription: >\n  Manage skills across AI coding agents with one shared library.\n  Use for migration, distribution, and symlink repair.\nmetadata:\n  version: nested-value-must-not-win\ntriggers:\n- migrate skills\n- repair symlinks\nversion: 1.7.0\n---\nBody";
        for markdown in [markdown.to_owned(), markdown.replace('\n', "\r\n")] {
            let metadata = parse_skill_markdown(&markdown);

            assert_eq!(metadata.name.as_deref(), Some("agent-skills-manager"));
            assert_eq!(
                metadata.description.as_deref(),
                Some(
                    "Manage skills across AI coding agents with one shared library. Use for migration, distribution, and symlink repair."
                )
            );
            assert_eq!(metadata.triggers, ["migrate skills", "repair symlinks"]);
            assert_eq!(metadata.version.as_deref(), Some("1.7.0"));
            assert_eq!(summarize_markdown(&markdown, 320), "Body");
        }
    }

    #[test]
    fn parses_standard_metadata_routing_triggers() {
        let metadata = parse_skill_markdown(
            "---\nname: skillroster\ndescription: Govern Skills\nmetadata:\n  bootstrap-version: 1.8.6\n  skillroster-routing-triggers: \"inventory installed Agent Skills; create Skill Receipt\"\n---\nBody",
        );

        assert_eq!(
            metadata.triggers,
            ["inventory installed Agent Skills", "create Skill Receipt"]
        );
        assert!(metadata.version.is_none());
    }

    #[test]
    fn ignores_nested_or_non_string_metadata_routing_triggers() {
        for markdown in [
            "---\nname: nested\nmetadata:\n  custom:\n    skillroster-routing-triggers: \"nested trigger\"\n---\n",
            "---\nname: sequence\nmetadata:\n  skillroster-routing-triggers:\n    - sequence trigger\n---\n",
            "---\nname: flow-sequence\nmetadata:\n  skillroster-routing-triggers: [inventory, apply]\n---\n",
            "---\nname: flow-map\nmetadata:\n  skillroster-routing-triggers: {route: inventory}\n---\n",
            "---\nname: boolean\nmetadata:\n  skillroster-routing-triggers: true\n---\n",
            "---\nname: unquoted\nmetadata:\n  skillroster-routing-triggers: inventory installed Skills\n---\n",
        ] {
            assert!(parse_skill_markdown(markdown).triggers.is_empty());
        }
    }

    #[test]
    fn explicit_root_discovers_nested_skills_and_exact_copies_share_identity() {
        let root = temp_directory("scan");
        for name in ["one", "nested/two"] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join("SKILL.md"),
                "---\nname: example\ndescription: Example skill\n---\nDo the same thing.",
            )
            .unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.placements.len(), 2);
        assert!(result.placements.iter().all(|placement| {
            placement.owned_by_agent == Some(true)
                && placement.mutation_scope == Some(MutationScope::Mutable)
                && placement.governable
        }));
        assert!(result.roots.iter().any(|seen| seen.explicit));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_marks_oversized_package_fingerprint_as_bounded() {
        let root = temp_directory("bounded-package-size");
        let skill = root.join("oversized");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: oversized\n---\n").unwrap();
        fs::write(skill.join("repair.sh"), "#!/bin/sh\n").unwrap();
        let asset = File::create(skill.join("asset.bin")).unwrap();
        asset.set_len(MAX_SKILL_PACKAGE_BYTES + 1).unwrap();

        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert_eq!(result.placements.len(), 1);
        assert_eq!(
            result.placements[0].fingerprint_completeness,
            FingerprintCompleteness::Bounded
        );
        assert!(
            result.placements[0]
                .fingerprint_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("fingerprint safety limit"))
        );
        assert_eq!(
            result.placements[0].executable_files,
            vec![skill.join("repair.sh")]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn configured_non_unicode_root_fails_before_snapshot_construction() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let mut options = ScanOptions::for_home("/tmp/skillroster-unicode-home");
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: PathBuf::from(OsString::from_vec(vec![b'/', 0x80])),
        });

        let error = scan(&options).unwrap_err();

        assert!(is_non_unicode_identity_error(&error));
        assert_eq!(error.to_string(), NON_UNICODE_IDENTITY_DETAIL);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scan_skips_distinct_non_unicode_skill_paths_without_identity_collision() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let root = temp_directory("non-unicode-entrypoints");
        let valid = root.join("valid");
        fs::create_dir(&valid).unwrap();
        fs::write(valid.join("SKILL.md"), "---\nname: valid\n---\n").unwrap();
        for bytes in [vec![0x80], vec![0x81]] {
            let directory = root.join(OsString::from_vec(bytes));
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("SKILL.md"), "---\nname: hidden\n---\n").unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;

        let result = scan(&options).unwrap();

        assert_eq!(result.placements.len(), 1);
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "valid");
        let observed = result.roots.iter().find(|seen| seen.path == root).unwrap();
        assert!(!observed.discovery_complete);
        assert!(
            observed
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("non-Unicode"))
        );
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains('\u{fffd}'));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_unicode_package_member_makes_exact_fingerprint_unreadable() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let root = temp_directory("non-unicode-package-member");
        let skill = root.join("valid-skill");
        fs::create_dir(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: valid-skill\ndescription: Unicode entrypoint\n---\n",
        )
        .unwrap();
        for (bytes, content) in [(vec![0x80], "first"), (vec![0x81], "second")] {
            fs::write(skill.join(OsString::from_vec(bytes)), content).unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;

        let first = scan(&options).unwrap();
        let second = scan(&options).unwrap();

        assert_eq!(first.placements.len(), 1);
        assert_eq!(first.placements[0].id, second.placements[0].id);
        assert_eq!(
            first.placements[0].fingerprint_completeness,
            FingerprintCompleteness::Unreadable
        );
        assert_eq!(
            first.placements[0].fingerprint_detail.as_deref(),
            Some(NON_UNICODE_IDENTITY_DETAIL)
        );
        assert!(first.skills[0].content_identity_digest.is_none());
        assert!(serde_json::to_string(&first).is_ok());
        assert!(inspect_skill_identity(&skill.join("SKILL.md")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_marks_deep_package_fingerprint_as_bounded() {
        let root = temp_directory("bounded-package-depth");
        let skill = root.join("deep-package");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: deep-package\n---\n").unwrap();
        let mut deep = skill.clone();
        for index in 0..9 {
            deep.push(format!("level-{index}"));
        }
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("behavior.sh"), "echo distinct\n").unwrap();

        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert_eq!(result.placements.len(), 1);
        assert_eq!(
            result.placements[0].fingerprint_completeness,
            FingerprintCompleteness::Bounded
        );
        assert!(
            result.placements[0]
                .fingerprint_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("bounded at depth 8"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_reports_bounded_skill_root_discovery() {
        let root = temp_directory("bounded-root-depth");
        let skill = root.join("one/two/three");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: too-deep\n---\n").unwrap();

        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        options.max_depth = 1;
        let result = scan(&options).unwrap();

        assert!(result.skills.is_empty());
        let observed = result
            .roots
            .iter()
            .find(|observed| observed.path == root)
            .unwrap();
        assert_eq!(observed.status, RootStatus::Included);
        assert!(!observed.discovery_complete);
        assert!(
            observed
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("bounded at depth 1"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_scan_payloads_default_to_untrusted_coverage() {
        let root = temp_directory("legacy-scan-payload");
        let skill = root.join("legacy");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: legacy\n---\n").unwrap();
        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        let current = scan(&options).unwrap();
        let mut legacy = serde_json::to_value(current).unwrap();
        legacy["roots"][0]
            .as_object_mut()
            .unwrap()
            .remove("discovery_complete");
        legacy["placements"][0]
            .as_object_mut()
            .unwrap()
            .remove("fingerprint_completeness");

        let restored: ScanResult = serde_json::from_value(legacy).unwrap();

        assert!(!restored.roots[0].discovery_complete);
        assert_eq!(
            restored.placements[0].fingerprint_completeness,
            FingerprintCompleteness::Unknown
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn explicit_source_root_alias_and_canonical_path_have_identical_semantics() {
        let root = temp_directory("source-root");
        let home = root.join("home");
        let source_root = root.join("sources");
        let source_alias = root.join("source-alias");
        let source_skill = source_root.join("external");
        fs::create_dir_all(&source_skill).unwrap();
        fs::write(
            source_skill.join("SKILL.md"),
            "---\nname: external\ndescription: External source Skill\n---\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&source_root, &source_alias).unwrap();
        let codex_root = home.join(".codex/skills");
        let claude_root = home.join(".claude/skills");
        fs::create_dir_all(&codex_root).unwrap();
        fs::create_dir_all(&claude_root).unwrap();
        std::os::unix::fs::symlink(source_alias.join("external"), codex_root.join("external"))
            .unwrap();
        std::os::unix::fs::symlink(&source_skill, claude_root.join("external")).unwrap();

        let scan_with = |source: PathBuf| {
            let mut options = ScanOptions::for_home(&home);
            options.explicit_source_roots = vec![source];
            options.include_session_evidence = false;
            scan(&options).unwrap()
        };
        let canonical_result = scan_with(source_root.clone());
        let alias_result = scan_with(source_alias.clone());

        let mut deduplicated_options = ScanOptions::for_home(&home);
        deduplicated_options.explicit_source_roots = vec![source_alias, source_root.clone()];
        deduplicated_options.include_session_evidence = false;
        let deduplicated_result = scan(&deduplicated_options).unwrap();

        assert_eq!(
            serde_json::to_value(&canonical_result).unwrap(),
            serde_json::to_value(&alias_result).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&canonical_result).unwrap(),
            serde_json::to_value(&deduplicated_result).unwrap()
        );
        let result = canonical_result;

        assert!(result.warnings.is_empty());
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.placements.len(), 3);
        assert_eq!(
            result
                .placements
                .iter()
                .filter(|placement| placement.default_exposed)
                .count(),
            2
        );
        let source_observations = result
            .roots
            .iter()
            .filter(|seen| seen.explicit && seen.agent.is_none())
            .collect::<Vec<_>>();
        assert_eq!(source_observations.len(), 1);
        assert_eq!(
            source_observations[0].path,
            fs::canonicalize(source_root).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn durable_read_discards_content_replaced_after_the_pre_read_identity_check() {
        let temp = tempfile::tempdir().unwrap();
        let source_input = temp.path().join("source");
        fs::create_dir(&source_input).unwrap();
        fs::write(
            source_input.join("SKILL.md"),
            "---\nname: before\n---\noriginal body\n",
        )
        .unwrap();
        let source = fs::canonicalize(&source_input).unwrap();
        let agent_root = temp.path().join("agent");
        fs::create_dir(&agent_root).unwrap();
        let linked = agent_root.join("external");
        std::os::unix::fs::symlink(&source, &linked).unwrap();
        let anchor = DurableReadRoot {
            permission_id: "sroot_replaced_during_read".into(),
            path: source.clone(),
            identity: crate::source_policy::capture_identity(&source).unwrap(),
        };
        let candidate = EntryCandidate {
            agent: Some(AgentKind::Codex),
            root: agent_root,
            governable: true,
            provider: None,
            expected_physical_entrypoint: Some(source.join("SKILL.md")),
            entrypoint: linked.join("SKILL.md"),
            link_target: Some(source.clone()),
            link_status: LinkStatus::Valid,
            default_exposed: true,
        };
        let mut result = ScanResult::default();
        let mut replaced = false;
        materialize_candidates_with_hook(vec![candidate], &[anchor], &[], &mut result, |_| {
            if replaced {
                return;
            }
            fs::remove_dir_all(&source).unwrap();
            fs::create_dir(&source).unwrap();
            fs::write(
                source.join("SKILL.md"),
                "---\nname: replacement-secret\n---\nDO-NOT-ADOPT-SECRET\n",
            )
            .unwrap();
            replaced = true;
        });

        assert_eq!(result.placements.len(), 1);
        assert_eq!(
            result.placements[0].fingerprint_completeness,
            FingerprintCompleteness::Unreadable
        );
        assert!(!result.placements[0].governable);
        assert_eq!(result.placements[0].owned_by_agent, Some(true));
        assert_eq!(
            result.placements[0].mutation_scope,
            Some(MutationScope::DurableReadOnly)
        );
        assert!(result.placements[0].physical_directory.is_none());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("replacement-secret"));
        assert!(!encoded.contains("DO-NOT-ADOPT-SECRET"));
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("binding drift"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn durable_read_rejects_an_agent_symlink_retargeted_before_read() {
        let temp = tempfile::tempdir().unwrap();
        let source_input = temp.path().join("source");
        fs::create_dir(&source_input).unwrap();
        fs::write(
            source_input.join("SKILL.md"),
            "---\nname: permitted\n---\npermitted body\n",
        )
        .unwrap();
        let source = fs::canonicalize(&source_input).unwrap();
        let external = temp.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(
            external.join("SKILL.md"),
            "---\nname: external-marker\n---\nEXTERNAL-SECRET-MARKER\n",
        )
        .unwrap();
        let agent_root = temp.path().join("agent");
        fs::create_dir(&agent_root).unwrap();
        let linked = agent_root.join("external");
        std::os::unix::fs::symlink(&source, &linked).unwrap();
        let anchor = DurableReadRoot {
            permission_id: "sroot_retargeted_entrypoint".into(),
            path: source.clone(),
            identity: crate::source_policy::capture_identity(&source).unwrap(),
        };
        let candidate = EntryCandidate {
            agent: Some(AgentKind::Codex),
            root: agent_root,
            governable: true,
            provider: None,
            entrypoint: linked.join("SKILL.md"),
            expected_physical_entrypoint: Some(source.join("SKILL.md")),
            link_target: Some(source),
            link_status: LinkStatus::Valid,
            default_exposed: true,
        };
        let mut result = ScanResult::default();
        materialize_candidates_with_hook(vec![candidate], &[anchor], &[], &mut result, |_| {
            fs::remove_file(&linked).unwrap();
            std::os::unix::fs::symlink(&external, &linked).unwrap();
        });

        assert_eq!(result.placements.len(), 1);
        assert_eq!(
            result.placements[0].fingerprint_completeness,
            FingerprintCompleteness::Unreadable
        );
        assert!(!result.placements[0].governable);
        assert!(result.placements[0].physical_directory.is_none());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("external-marker"));
        assert!(!encoded.contains("EXTERNAL-SECRET-MARKER"));
    }

    #[cfg(unix)]
    #[test]
    fn shared_physical_package_is_observed_once_with_logical_placement_facts() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("scripts")).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: shared\n---\nshared body\n",
        )
        .unwrap();
        fs::write(source.join("scripts/run.sh"), "#!/bin/sh\n").unwrap();
        let source = fs::canonicalize(source).unwrap();
        let physical_entrypoint = source.join("SKILL.md");

        let mut candidates = Vec::new();
        for (agent, directory_name) in [
            (AgentKind::Codex, "codex"),
            (AgentKind::ClaudeCode, "claude"),
            (AgentKind::Pi, "pi"),
        ] {
            let agent_root = temp.path().join(directory_name);
            fs::create_dir(&agent_root).unwrap();
            let linked = agent_root.join("shared");
            std::os::unix::fs::symlink(&source, &linked).unwrap();
            candidates.push(EntryCandidate {
                agent: Some(agent),
                root: agent_root,
                governable: true,
                provider: None,
                expected_physical_entrypoint: Some(physical_entrypoint.clone()),
                entrypoint: linked.join("SKILL.md"),
                link_target: Some(source.clone()),
                link_status: LinkStatus::Valid,
                default_exposed: true,
            });
        }

        let mut result = ScanResult::default();
        let stats = materialize_candidates_with_hook(candidates, &[], &[], &mut result, |_| {});

        assert_eq!(stats, 1);
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.placements.len(), 3);
        assert!(result.placements.iter().all(|placement| {
            placement.physical_directory.as_ref() == Some(&source)
                && placement.executable_files == vec![placement.directory.join("scripts/run.sh")]
        }));
    }

    #[cfg(unix)]
    #[test]
    fn changed_physical_package_is_not_reused_by_a_later_alias() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("references")).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: shared\n---\nshared body\n",
        )
        .unwrap();
        let supporting_file = source.join("references/guide.md");
        fs::write(&supporting_file, "original\n").unwrap();
        let source = fs::canonicalize(source).unwrap();
        let physical_entrypoint = source.join("SKILL.md");

        let mut candidates = Vec::new();
        for (agent, directory_name) in [
            (AgentKind::Codex, "codex"),
            (AgentKind::ClaudeCode, "claude"),
            (AgentKind::Pi, "pi"),
        ] {
            let agent_root = temp.path().join(directory_name);
            fs::create_dir(&agent_root).unwrap();
            let linked = agent_root.join("shared");
            std::os::unix::fs::symlink(&source, &linked).unwrap();
            candidates.push(EntryCandidate {
                agent: Some(agent),
                root: agent_root,
                governable: true,
                provider: None,
                expected_physical_entrypoint: Some(physical_entrypoint.clone()),
                entrypoint: linked.join("SKILL.md"),
                link_target: Some(source.clone()),
                link_status: LinkStatus::Valid,
                default_exposed: true,
            });
        }

        let mut result = ScanResult::default();
        let mut candidate_index = 0;
        let observations =
            materialize_candidates_with_hook(candidates, &[], &[], &mut result, |_| {
                if candidate_index == 1 {
                    fs::write(&supporting_file, "changed supporting content\n").unwrap();
                }
                candidate_index += 1;
            });

        assert_eq!(observations, 1);
        assert_eq!(result.placements.len(), 3);
        assert_eq!(
            result.placements[0].fingerprint_completeness,
            FingerprintCompleteness::Complete
        );
        assert!(result.placements[0].governable);
        assert_eq!(
            result.placements[1].fingerprint_completeness,
            FingerprintCompleteness::Unreadable
        );
        assert!(!result.placements[1].governable);
        assert!(result.placements[1].physical_directory.is_none());
        assert_eq!(
            result.placements[2].fingerprint_completeness,
            FingerprintCompleteness::Unreadable
        );
        assert!(!result.placements[2].governable);
        assert!(result.placements[2].physical_directory.is_none());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("physical package changed before cache reuse"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_alias_retargeted_before_read_is_not_observed_or_reused() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: permitted\n---\npermitted body\n",
        )
        .unwrap();
        let source = fs::canonicalize(source).unwrap();
        let external = temp.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(
            external.join("SKILL.md"),
            "---\nname: external-marker\n---\nEXTERNAL-SECRET-MARKER\n",
        )
        .unwrap();
        let agent_root = temp.path().join("agent");
        fs::create_dir(&agent_root).unwrap();
        let linked = agent_root.join("shared");
        std::os::unix::fs::symlink(&source, &linked).unwrap();
        let candidate = EntryCandidate {
            agent: Some(AgentKind::Codex),
            root: agent_root,
            governable: true,
            provider: None,
            expected_physical_entrypoint: Some(source.join("SKILL.md")),
            entrypoint: linked.join("SKILL.md"),
            link_target: Some(source),
            link_status: LinkStatus::Valid,
            default_exposed: true,
        };
        let mut result = ScanResult::default();
        let stats =
            materialize_candidates_with_hook(vec![candidate], &[], &[], &mut result, |_| {
                fs::remove_file(&linked).unwrap();
                std::os::unix::fs::symlink(&external, &linked).unwrap();
            });

        assert_eq!(stats, 0);
        assert_eq!(result.placements.len(), 1);
        assert_eq!(
            result.placements[0].fingerprint_completeness,
            FingerprintCompleteness::Unreadable
        );
        assert!(!result.placements[0].governable);
        assert!(result.placements[0].physical_directory.is_none());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("external-marker"));
        assert!(!encoded.contains("EXTERNAL-SECRET-MARKER"));
    }

    #[cfg(unix)]
    #[test]
    fn durable_root_replacement_before_enumeration_discards_all_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let source_input = temp.path().join("source");
        fs::create_dir_all(source_input.join("permitted-name")).unwrap();
        fs::write(
            source_input.join("permitted-name/SKILL.md"),
            "---\nname: permitted-name\n---\n",
        )
        .unwrap();
        let source = fs::canonicalize(&source_input).unwrap();
        let anchor = DurableReadRoot {
            permission_id: "sroot_replaced_before_enumeration".into(),
            path: source.clone(),
            identity: crate::source_policy::capture_identity(&source).unwrap(),
        };
        let mut result = ScanResult::default();
        let mut candidates = Vec::new();
        observe_durable_skill_root_with_hook(
            &anchor,
            SkillRootPolicy {
                agent: None,
                explicit: true,
                governable: false,
                detail: Some("durable read-only fixture".into()),
                provider: None,
            },
            std::slice::from_ref(&source),
            5,
            &mut result,
            &mut candidates,
            |_| {
                fs::remove_dir_all(&source).unwrap();
                fs::create_dir_all(source.join("substitute-secret-name")).unwrap();
                fs::write(
                    source.join("substitute-secret-name/SKILL.md"),
                    "---\nname: SUBSTITUTE-SECRET-NAME\n---\n",
                )
                .unwrap();
            },
        );

        assert!(candidates.is_empty());
        assert_eq!(result.roots.len(), 1);
        assert_eq!(result.roots[0].status, RootStatus::Inaccessible);
        assert!(!result.roots[0].discovery_complete);
        assert!(
            result
                .durable_read_drifted_permission_ids
                .contains("sroot_replaced_before_enumeration")
        );
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("substitute-secret-name"));
        assert!(!encoded.contains("SUBSTITUTE-SECRET-NAME"));
    }

    #[test]
    fn package_digest_distinguishes_different_supporting_files() {
        let root = temp_directory("package-digest");
        for (name, script) in [("one", "first"), ("two", "second")] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join("SKILL.md"),
                "---\nname: example\ndescription: Example skill\n---\nSame instructions.",
            )
            .unwrap();
            fs::write(directory.join("run.sh"), script).unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();
        assert_eq!(result.skills.len(), 2);
        assert_eq!(result.placements.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn content_identity_ignores_gitignore_but_keeps_supporting_files() {
        let root = temp_directory("content-identity");
        let first = root.join("one");
        let second = root.join("two");
        for directory in [&first, &second] {
            fs::create_dir_all(directory).unwrap();
            fs::write(
                directory.join("SKILL.md"),
                "---\nname: example\ndescription: Example skill\n---\nSame instructions.",
            )
            .unwrap();
            fs::write(directory.join("run.sh"), "same behavior").unwrap();
        }
        fs::write(second.join(".gitignore"), "local-only\n").unwrap();

        let (first_id, first_package_digest) =
            inspect_skill_identity(&first.join("SKILL.md")).unwrap();
        let (second_id, second_package_digest) =
            inspect_skill_identity(&second.join("SKILL.md")).unwrap();
        assert_eq!(first_id, second_id);
        assert_ne!(first_package_digest, second_package_digest);

        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();
        assert_eq!(
            result.content_identity_algorithm.as_deref(),
            Some(CONTENT_IDENTITY_ALGORITHM)
        );
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.placements.len(), 2);
        let first_content_identity = digest_skill_directory(&first)
            .unwrap()
            .content_identity_digest;
        assert_eq!(
            result.skills[0].content_identity_digest.as_deref(),
            Some(first_content_identity.as_str())
        );

        fs::write(second.join("run.sh"), "different behavior").unwrap();
        let (changed_id, _) = inspect_skill_identity(&second.join("SKILL.md")).unwrap();
        assert_ne!(first_id, changed_id);
        let changed = scan(&options).unwrap();
        assert_eq!(changed.skills.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn all_eight_direct_adapters_discover_their_known_skill_root() {
        let home = temp_directory("eight-adapters");
        for roots in known_agent_roots(&home) {
            let directory = roots.skill_roots[0].join(format!("{}-fixture", roots.agent.id()));
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join("SKILL.md"),
                format!(
                    "---\nname: {}-fixture\ndescription: adapter fixture\n---\n",
                    roots.agent.id()
                ),
            )
            .unwrap();
        }
        let mut options = ScanOptions::for_home(home.clone());
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();
        let discovered = result
            .placements
            .iter()
            .filter_map(|placement| placement.agent)
            .collect::<BTreeSet<_>>();
        assert_eq!(discovered, AgentKind::ALL.into_iter().collect());
        assert_eq!(result.skills.len(), 8);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn codex_nested_records_produce_matched_and_loaded_usage() {
        let home = temp_directory("codex-nested-usage");
        let skill = home.join(".codex/skills/research-directory");
        let sessions = home.join(".codex/sessions");
        fs::create_dir_all(&skill).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: primary-research\ndescription: Primary-source research\n---\n",
        )
        .unwrap();
        let entrypoint = skill.join("SKILL.md");
        let generic = home.join(".codex/skills/plan");
        fs::create_dir_all(&generic).unwrap();
        fs::write(
            generic.join("SKILL.md"),
            "---\nname: plan\ndescription: Planning workflow\n---\n",
        )
        .unwrap();
        let records = [
            serde_json::json!({
                "type": "session_meta",
                "payload": {
                    "base_instructions": "primary-research is available in the local Skill catalog"
                }
            }),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": format!(
                            "Plan this with [$primary-research]({})",
                            entrypoint.display()
                        )
                    }]
                }
            }),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "name": "exec",
                    "input": format!(
                        "await tools.exec_command({{cmd: \"sed -n '1,80p' {}\"}})",
                        entrypoint.display()
                    )
                }
            }),
        ]
        .into_iter()
        .map(|record| record.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        fs::write(sessions.join("session.jsonl"), records).unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let skill_id = &result
            .skills
            .iter()
            .find(|skill| skill.name == "primary-research")
            .unwrap()
            .id;
        let generic_skill_id = &result
            .skills
            .iter()
            .find(|skill| skill.name == "plan")
            .unwrap()
            .id;
        let stages = result
            .usage
            .iter()
            .filter(|usage| usage.skill_id == *skill_id && usage.agent == AgentKind::Codex)
            .map(|usage| (usage.stage, usage.quality))
            .collect::<Vec<_>>();

        assert!(stages.contains(&(UsageStage::Exposed, EvidenceQuality::Inferred)));
        assert!(
            stages.contains(&(UsageStage::Matched, EvidenceQuality::Observed)),
            "entrypoint={} placements={:?} usage={:?}",
            entrypoint.display(),
            result.placements,
            result.usage
        );
        assert!(
            stages.contains(&(UsageStage::Loaded, EvidenceQuality::Observed)),
            "entrypoint={} placements={:?} usage={:?}",
            entrypoint.display(),
            result.placements,
            result.usage
        );
        assert!(
            result
                .usage
                .iter()
                .filter(|usage| usage.skill_id == *generic_skill_id)
                .all(|usage| usage.stage == UsageStage::Exposed)
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn record_timestamps_are_nested_unit_aware_and_fail_closed() {
        assert_eq!(
            session_record_timestamp(
                AgentKind::Codex,
                r#"{"payload":{"createdAt":"2024-02-03T04:05:06Z"}}"#
            ),
            Some(1_706_933_106)
        );
        assert_eq!(
            session_record_timestamp(AgentKind::ClaudeCode, r#"{"timestamp":1706933106000.0}"#),
            Some(1_706_933_106)
        );
        assert_eq!(
            session_record_timestamp(AgentKind::Pi, r#"{"timestamp":"1706933106000000"}"#),
            Some(1_706_933_106)
        );
        assert_eq!(
            session_record_timestamp(AgentKind::OpenCode, r#"{"time":30}"#),
            None
        );
        assert_eq!(
            session_record_timestamp(AgentKind::Hermes, r#"{"timestamp":0}"#),
            None
        );
        assert_eq!(
            session_record_timestamp(
                AgentKind::Codex,
                r#"{"timestamp":"2024-01-01T00:00:00Z","payload":{"created_at":"2024-02-01T00:00:00Z"}}"#
            ),
            None
        );
        assert_eq!(
            session_record_timestamp(
                AgentKind::Codex,
                r#"{"tool":{"arguments":{"created_at":"2024-02-03T04:05:06Z"}}}"#
            ),
            None
        );
    }

    #[test]
    fn session_usage_is_partitioned_by_event_month_before_persistence() {
        let home = temp_directory("usage-event-months");
        let skill = home.join(".codex/skills/example");
        let sessions = home.join(".codex/sessions");
        fs::create_dir_all(&skill).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: example\n---\n").unwrap();
        fs::write(
            sessions.join("session.jsonl"),
            [
                r#"{"timestamp":"2024-01-15T00:00:00Z","invoked_skill":"example"}"#,
                r#"{"timestamp":"2024-02-15T00:00:00Z","invoked_skill":"example"}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let observed = result
            .usage
            .iter()
            .filter(|usage| usage.stage == UsageStage::Applied)
            .map(|usage| (usage.month_start_unix, usage.event_count))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            observed,
            BTreeSet::from([(Some(1_704_067_200), 1), (Some(1_706_745_600), 1)])
        );

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn large_current_session_tail_contributes_observed_usage() {
        let home = temp_directory("large-current-session");
        let skill = home.join(".codex/skills/research");
        let sessions = home.join(".codex/sessions");
        fs::create_dir_all(&skill).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: research\ndescription: Research workflow\n---\n",
        )
        .unwrap();

        let mut records = vec![b'x'; MAX_SESSION_BYTES_PER_AGENT as usize + 1];
        records.push(b'\n');
        records.extend_from_slice(br#"{"type":"load_skill","loaded_skill":"research"}"#);
        records.push(b'\n');
        fs::write(sessions.join("current.jsonl"), records).unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let skill_id = &result
            .skills
            .iter()
            .find(|skill| skill.name == "research")
            .unwrap()
            .id;

        assert!(result.usage.iter().any(|usage| {
            usage.skill_id == *skill_id
                && usage.agent == AgentKind::Codex
                && usage.stage == UsageStage::Loaded
                && usage.quality == EvidenceQuality::Observed
        }));
        assert_eq!(result.coverage[0].files_observed, 1);
        assert_eq!(result.coverage[0].files_partially_observed, 1);
        assert!(result.coverage[0].truncated);
        assert!(!result.coverage[0].denominator_reliable);
        let limitations = result.coverage[0].limitations.as_ref().unwrap();
        assert!(!limitations.iter().any(|limitation| {
            limitation.code == SessionCoverageLimitationCode::SampledByteLimit
        }));
        assert!(limitations.iter().any(|limitation| {
            limitation.code == SessionCoverageLimitationCode::FileByteLimit
                && limitation.scope == SessionCoverageScope::File
                && limitation.limit == Some(MAX_SESSION_BYTES_PER_FILE)
        }));
        assert!(limitations.iter().any(|limitation| {
            limitation.code == SessionCoverageLimitationCode::LineAlignmentLoss
                && limitation.source == SessionCoverageLimitationSource::SessionJsonl
        }));

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn session_denominator_requires_every_known_root_to_be_observable() {
        let home = temp_directory("mixed-session-roots");
        let included = home.join("included");
        let missing = home.join("missing");
        fs::create_dir_all(&included).unwrap();
        let mut result = ScanResult::default();

        scan_sessions(AgentKind::Codex, &[included.clone(), missing], &mut result);

        let coverage = result
            .coverage
            .iter()
            .find(|coverage| coverage.agent == AgentKind::Codex)
            .unwrap();
        assert_eq!(coverage.roots_present, 1);
        assert_eq!(coverage.roots_missing, 1);
        assert_eq!(coverage.roots_inaccessible, 0);
        assert!(!coverage.denominator_reliable);
        assert!(
            coverage
                .limitations
                .as_ref()
                .unwrap()
                .iter()
                .any(|limitation| {
                    limitation.code == SessionCoverageLimitationCode::RootMissing
                        && limitation.scope == SessionCoverageScope::Root
                })
        );

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn session_discovery_depth_is_a_typed_denominator_blocker() {
        let home = temp_directory("deep-session-root");
        let mut nested = home.join(".codex/sessions");
        for index in 0..8 {
            nested.push(format!("level-{index}"));
        }
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("session.jsonl"), "{}\n").unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let coverage = result
            .coverage
            .iter()
            .find(|coverage| coverage.agent == AgentKind::Codex)
            .unwrap();

        assert!(!coverage.denominator_reliable);
        assert!(coverage.discovery_truncated);
        assert!(
            coverage
                .limitations
                .as_ref()
                .unwrap()
                .iter()
                .any(|limitation| {
                    limitation.code == SessionCoverageLimitationCode::DiscoveryDepthLimit
                        && limitation.scope == SessionCoverageScope::Root
                        && limitation.limit == Some(6)
                })
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn bounded_session_budget_is_spread_across_large_recent_files() {
        let home = temp_directory("large-recent-sessions");
        let sessions = home.join(".codex/sessions");
        fs::create_dir_all(&sessions).unwrap();
        for name in ["research", "review"] {
            let skill = home.join(".codex/skills").join(name);
            fs::create_dir_all(&skill).unwrap();
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name} workflow\n---\n"),
            )
            .unwrap();
            let mut records = vec![b'x'; MAX_SESSION_BYTES_PER_AGENT as usize + 1];
            records.push(b'\n');
            records.extend_from_slice(
                format!(r#"{{"type":"load_skill","loaded_skill":"{name}"}}"#).as_bytes(),
            );
            records.push(b'\n');
            fs::write(sessions.join(format!("{name}.jsonl")), records).unwrap();
        }

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let loaded = result
            .usage
            .iter()
            .filter(|usage| {
                usage.agent == AgentKind::Codex
                    && usage.stage == UsageStage::Loaded
                    && usage.quality == EvidenceQuality::Observed
            })
            .map(|usage| usage.skill_id.as_str())
            .collect::<BTreeSet<_>>();
        let expected = result
            .skills
            .iter()
            .map(|skill| skill.id.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(loaded, expected);
        assert_eq!(result.coverage[0].files_observed, 2);
        assert_eq!(result.coverage[0].files_partially_observed, 2);
        assert!(result.coverage[0].bytes_observed <= MAX_SESSION_BYTES_PER_AGENT);

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn large_monolithic_json_tail_is_bounded_and_contributes_usage() {
        let home = temp_directory("large-monolithic-json");
        let sessions = home.join(".hermes/sessions");
        fs::create_dir_all(&sessions).unwrap();
        for (name, padding_bytes) in [("research", 5 * 1024 * 1024), ("review", 700 * 1024)] {
            let skill = home.join(".hermes/skills").join(name);
            fs::create_dir_all(&skill).unwrap();
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name} workflow\n---\n"),
            )
            .unwrap();
            let document = format!(
                r#"{{"padding":"{}","event":{{"type":"load_skill","loaded_skill":"{name}"}}}}"#,
                "x".repeat(padding_bytes)
            );
            fs::write(sessions.join(format!("{name}.json")), document).unwrap();
        }

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let loaded = result
            .usage
            .iter()
            .filter(|usage| usage.stage == UsageStage::Loaded)
            .filter_map(|usage| {
                result
                    .skills
                    .iter()
                    .find(|skill| skill.id == usage.skill_id)
                    .map(|skill| skill.name.as_str())
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(loaded, BTreeSet::from(["research", "review"]));
        let coverage = result
            .coverage
            .iter()
            .find(|coverage| coverage.agent == AgentKind::Hermes)
            .unwrap();
        assert_eq!(coverage.files_observed, 2);
        assert_eq!(coverage.files_partially_observed, 2);
        assert!(coverage.bytes_observed <= 2 * MAX_SESSION_BYTES_PER_FILE);

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn repeated_events_in_one_json_document_keep_their_count() {
        let home = temp_directory("repeated-json-events");
        let skill = home.join(".hermes/skills/research");
        let sessions = home.join(".hermes/sessions");
        fs::create_dir_all(&skill).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: research\ndescription: Research workflow\n---\n",
        )
        .unwrap();
        fs::write(
            sessions.join("session.json"),
            serde_json::json!([
                {"type": "load_skill", "skill_id": "research", "skill_name": "research", "loaded_skill": "research"},
                {"type": "load_skill", "skill_id": "research", "skill_name": "research", "loaded_skill": "research"}
            ])
            .to_string(),
        )
        .unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let loaded = result
            .usage
            .iter()
            .find(|usage| usage.agent == AgentKind::Hermes && usage.stage == UsageStage::Loaded)
            .unwrap();
        assert_eq!(loaded.event_count, 2);

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn json_object_extraction_is_bounded_for_many_siblings() {
        let sample = format!("[{}]", vec!["{}"; 100_000].join(","));
        let records = extract_complete_json_objects(&sample);
        assert_eq!(records.len(), MAX_SESSION_LINES_PER_AGENT);
        assert!(
            records
                .iter()
                .all(|(record, lines)| record == "{}" && *lines == 1)
        );
    }

    #[test]
    fn aligned_session_tail_records_discarded_partial_line() {
        let home = temp_directory("aligned-session-tail");
        let path = home.join("session.jsonl");
        fs::create_dir_all(&home).unwrap();
        fs::write(&path, "discarded-partial\nkept\n").unwrap();

        let metadata = fs::metadata(&path).unwrap();
        let (sample, bytes_read, alignment_lost) =
            read_session_tail_with_facts(&path, metadata.len(), 10, true).unwrap();

        assert_eq!(bytes_read, 10);
        assert_eq!(sample, b"kept\n");
        assert!(alignment_lost);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn sampled_file_count_is_lower_bound_when_discovery_is_incomplete() {
        assert_eq!(
            discovered_count_kind(&SessionDiscoveryOutcome::default()),
            SessionCoverageCountKind::Exact
        );
        assert_eq!(
            discovered_count_kind(&SessionDiscoveryOutcome {
                file_limit: true,
                depth_limit: false,
            }),
            SessionCoverageCountKind::LowerBound
        );
        assert_eq!(
            discovered_count_kind(&SessionDiscoveryOutcome {
                file_limit: false,
                depth_limit: true,
            }),
            SessionCoverageCountKind::LowerBound
        );

        let mut limitations = BTreeMap::new();
        for (count_kind, observed) in [
            (SessionCoverageCountKind::Exact, 300),
            (SessionCoverageCountKind::LowerBound, 10_001),
        ] {
            record_session_limitation(
                &mut limitations,
                SessionCoverageLimitationCode::SampledFileLimit,
                SessionCoverageScope::Root,
                count_kind,
                Some(observed),
                Some(MAX_SESSION_FILES_PER_ROOT as u64),
                SessionCoverageLimitationSource::SessionSampling,
            );
        }
        let limitation = limitations
            .get(&SessionCoverageLimitationCode::SampledFileLimit)
            .unwrap();
        assert_eq!(limitation.count_kind, SessionCoverageCountKind::LowerBound);
        assert_eq!(limitation.observed, Some(10_001));
    }

    #[test]
    fn json_record_limit_is_a_typed_denominator_blocker() {
        let home = temp_directory("json-record-limit");
        let skill = home.join(".hermes/skills/research");
        let sessions = home.join(".hermes/sessions");
        fs::create_dir_all(&skill).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: research\n---\n").unwrap();
        fs::write(
            sessions.join("session.json"),
            format!(
                r#"{{"padding":"{}","events":[{}]}}"#,
                "x".repeat(MAX_SESSION_BYTES_PER_FILE as usize + 1),
                vec!["{}"; MAX_SESSION_LINES_PER_AGENT + 1].join(",")
            ),
        )
        .unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let coverage = result
            .coverage
            .iter()
            .find(|coverage| coverage.agent == AgentKind::Hermes)
            .unwrap();
        assert!(!coverage.denominator_reliable);
        assert!(
            coverage
                .limitations
                .as_ref()
                .unwrap()
                .iter()
                .any(|limitation| {
                    limitation.code == SessionCoverageLimitationCode::JsonRecordLimit
                        && limitation.unit == SessionCoverageUnit::Records
                })
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn realistic_nested_agent_records_bind_observed_usage() {
        let home = temp_directory("nested-agent-records");
        let placements = [
            (AgentKind::ClaudeCode, home.join(".claude/skills/research")),
            (AgentKind::Pi, home.join(".pi/agent/skills/research")),
            (AgentKind::Hermes, home.join(".hermes/skills/research")),
            (AgentKind::Cursor, home.join(".cursor/skills/research")),
        ];
        for (_, placement) in &placements {
            fs::create_dir_all(placement).unwrap();
            fs::write(
                placement.join("SKILL.md"),
                "---\nname: research\ndescription: Research workflow\n---\n",
            )
            .unwrap();
        }

        let claude_sessions = home.join(".claude/projects");
        fs::create_dir_all(&claude_sessions).unwrap();
        fs::write(
            claude_sessions.join("session.jsonl"),
            serde_json::json!({
                "type": "assistant",
                "message": {"role": "assistant", "content": [{
                    "type": "tool_use",
                    "name": "Skill",
                    "input": {"skill": "research", "args": ""}
                }]}
            })
            .to_string(),
        )
        .unwrap();

        for (agent, root, placement) in [
            (
                AgentKind::Pi,
                home.join(".pi/agent/sessions"),
                home.join(".pi/agent/skills/research"),
            ),
            (
                AgentKind::Cursor,
                home.join(".cursor/projects"),
                home.join(".cursor/skills/research"),
            ),
        ] {
            fs::create_dir_all(&root).unwrap();
            let record = match agent {
                AgentKind::Pi => serde_json::json!({
                    "type": "message",
                    "message": {"role": "assistant", "content": [{
                        "type": "toolCall",
                        "name": "read",
                        "arguments": {"path": placement.join("SKILL.md")}
                    }]}
                }),
                AgentKind::Cursor => serde_json::json!({
                    "role": "assistant",
                    "message": {"role": "assistant", "content": [{
                        "type": "tool_use",
                        "name": "Read",
                        "input": {"path": placement.join("SKILL.md")}
                    }]}
                }),
                _ => unreachable!(),
            };
            fs::write(root.join("session.jsonl"), record.to_string()).unwrap();
        }

        let hermes_sessions = home.join(".hermes/sessions");
        fs::create_dir_all(&hermes_sessions).unwrap();
        fs::write(
            hermes_sessions.join("session.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "request": {"body": {"messages": [{
                    "role": "assistant",
                    "tool_calls": [{
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": serde_json::json!({
                                "path": home.join(".hermes/skills/research/SKILL.md")
                            }).to_string()
                        }
                    }]
                }]}}
            }))
            .unwrap(),
        )
        .unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let observed = result
            .usage
            .iter()
            .filter(|usage| usage.quality == EvidenceQuality::Observed)
            .map(|usage| (usage.agent, usage.stage))
            .collect::<BTreeSet<_>>();

        assert!(observed.contains(&(AgentKind::ClaudeCode, UsageStage::Applied)));
        for agent in [AgentKind::Pi, AgentKind::Hermes, AgentKind::Cursor] {
            assert!(observed.contains(&(agent, UsageStage::Loaded)));
        }
        for agent in [
            AgentKind::ClaudeCode,
            AgentKind::Pi,
            AgentKind::Hermes,
            AgentKind::Cursor,
        ] {
            assert!(
                result
                    .coverage
                    .iter()
                    .find(|coverage| coverage.agent == agent)
                    .unwrap()
                    .denominator_reliable
            );
        }

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn explicit_skill_invocation_does_not_elevate_other_paths_in_the_same_record() {
        let home = temp_directory("mixed-skill-record");
        for name in ["research", "review"] {
            let skill = home.join(".claude/skills").join(name);
            fs::create_dir_all(&skill).unwrap();
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name} workflow\n---\n"),
            )
            .unwrap();
        }
        let sessions = home.join(".claude/projects");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("session.jsonl"),
            serde_json::json!({
                "type": "assistant",
                "message": {"role": "assistant", "content": [
                    {
                        "type": "tool_use",
                        "name": "Skill",
                        "input": {"skill": "research", "args": ""}
                    },
                    {
                        "type": "tool_use",
                        "name": "Read",
                        "input": {
                            "path": home.join(".claude/skills/review/SKILL.md")
                        }
                    }
                ]}
            })
            .to_string(),
        )
        .unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let research_id = &result
            .skills
            .iter()
            .find(|skill| skill.name == "research")
            .unwrap()
            .id;
        let review_id = &result
            .skills
            .iter()
            .find(|skill| skill.name == "review")
            .unwrap()
            .id;

        assert!(
            result.usage.iter().any(|usage| {
                usage.skill_id == *research_id && usage.stage == UsageStage::Applied
            })
        );
        assert!(
            !result.usage.iter().any(|usage| {
                usage.skill_id == *review_id && usage.stage == UsageStage::Applied
            })
        );
        assert!(
            result
                .usage
                .iter()
                .any(|usage| usage.skill_id == *review_id && usage.stage == UsageStage::Loaded)
        );

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn inaccessible_session_root_is_not_reported_as_missing() {
        let home = temp_directory("inaccessible-session-root");
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(home.join(".codex/sessions"), "not a directory").unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();
        let coverage = result
            .coverage
            .iter()
            .find(|coverage| coverage.agent == AgentKind::Codex)
            .unwrap();
        assert_eq!(coverage.roots_present, 0);
        assert_eq!(coverage.roots_missing, 0);
        assert_eq!(coverage.roots_inaccessible, 1);
        assert!(result.roots.iter().any(|root| {
            root.agent == Some(AgentKind::Codex)
                && root.kind == RootKind::Sessions
                && root.status == RootStatus::Inaccessible
        }));

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn newest_session_is_selected_after_discovering_more_than_file_sample_limit() {
        let home = temp_directory("newest-session-selection");
        let skill = home.join(".codex/skills/research");
        let sessions = home.join(".codex/sessions");
        fs::create_dir_all(&skill).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: research\ndescription: Research workflow\n---\n",
        )
        .unwrap();
        let old_time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10);
        for index in 0..=MAX_SESSION_FILES_PER_ROOT {
            let path = sessions.join(format!("old-{index:03}.jsonl"));
            fs::write(&path, "{}\n").unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(old_time))
                .unwrap();
        }
        let latest = sessions.join("latest.jsonl");
        fs::write(
            &latest,
            "{\"type\":\"load_skill\",\"loaded_skill\":\"research\"}\n",
        )
        .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(latest)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(20)),
            )
            .unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();

        assert!(result.usage.iter().any(|usage| {
            usage.agent == AgentKind::Codex
                && usage.stage == UsageStage::Loaded
                && usage.quality == EvidenceQuality::Observed
        }));
        assert_eq!(
            result.coverage[0].files_discovered,
            MAX_SESSION_FILES_PER_ROOT + 2
        );
        assert_eq!(
            result.coverage[0].files_observed,
            MAX_SESSION_FILES_PER_ROOT
        );
        assert_eq!(result.coverage[0].files_skipped, 2);

        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn json_escaped_windows_reference_binds_observed_usage() {
        let windows_path = r"C:\Users\tester\.codex\skills\research\SKILL.md";
        let session_path = r"C:\Users\tester\.codex/skills/research\SKILL.md";
        let reference_lookup = vec![(
            "skill_windows".to_owned(),
            normalize_reference_text(windows_path),
        )];
        let matcher = AhoCorasickBuilder::new()
            .ascii_case_insensitive(true)
            .build(
                reference_lookup
                    .iter()
                    .map(|(_, reference)| reference.as_str()),
            )
            .unwrap();
        let line = serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "name": "exec",
                "input": format!(
                    "await tools.exec_command({{cmd: \"rg -n name {}\"}})",
                    session_path
                )
            }
        })
        .to_string();

        assert!(line.contains(r"C:\\Users\\tester"));
        assert!(
            session_record_observations(AgentKind::Codex, &line)
                .iter()
                .any(|observation| observation.signal == SessionSignal::Loaded)
        );
        assert_eq!(
            observed_reference_skill_ids(&line, Some(&matcher), &reference_lookup),
            BTreeSet::from(["skill_windows".to_owned()])
        );

        let nested_arguments = serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "arguments": serde_json::json!({"path": session_path}).to_string()
            }
        })
        .to_string();
        assert_eq!(
            observed_reference_skill_ids(&nested_arguments, Some(&matcher), &reference_lookup),
            BTreeSet::from(["skill_windows".to_owned()])
        );
    }

    #[test]
    fn ambiguous_skill_name_does_not_receive_observed_usage() {
        let home = temp_directory("ambiguous-usage-name");
        for (directory, body) in [("one", "first"), ("two", "second")] {
            let skill = home.join(".codex/skills").join(directory);
            fs::create_dir_all(&skill).unwrap();
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: research\n---\n{body}\n"),
            )
            .unwrap();
        }
        let sessions = home.join(".codex/sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("session.jsonl"),
            "{\"type\":\"match_skill\",\"matched_skill\":\"research\"}\n",
        )
        .unwrap();

        let result = scan(&ScanOptions::for_home(&home)).unwrap();

        assert_eq!(result.skills.len(), 2);
        assert!(
            result
                .usage
                .iter()
                .all(|usage| usage.stage == UsageStage::Exposed)
        );
        assert!(agents_with_usage(&result).is_empty());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn exposed_inference_is_not_counted_as_observed_agent_usage() {
        let mut result = ScanResult::default();
        result.usage.push(UsageEvidence {
            agent: AgentKind::Codex,
            skill_id: "skill_fixture".into(),
            stage: UsageStage::Exposed,
            quality: EvidenceQuality::Inferred,
            event_count: 1,
            first_seen_unix: None,
            last_seen_unix: None,
            month_start_unix: None,
            source_path_digest: "sha256:fixture".into(),
        });
        assert!(agents_with_usage(&result).is_empty());

        result.usage.push(UsageEvidence {
            agent: AgentKind::Codex,
            skill_id: "skill_fixture".into(),
            stage: UsageStage::Matched,
            quality: EvidenceQuality::Observed,
            event_count: 1,
            first_seen_unix: None,
            last_seen_unix: None,
            month_start_unix: None,
            source_path_digest: "sha256:fixture".into(),
        });
        assert_eq!(
            agents_with_usage(&result),
            BTreeSet::from([AgentKind::Codex])
        );
    }

    #[cfg(unix)]
    #[test]
    fn reports_links_that_escape_an_approved_root() {
        use std::os::unix::fs::symlink;
        let root = temp_directory("escaping-link");
        let outside = temp_directory("outside");
        fs::write(outside.join("SKILL.md"), "---\nname: outside\n---\n").unwrap();
        symlink(&outside, root.join("outside-link")).unwrap();
        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();
        assert_eq!(result.placements.len(), 1);
        assert_eq!(result.placements[0].link_status, LinkStatus::EscapesRoot);
        assert_eq!(result.placements[0].owned_by_agent, Some(true));
        assert_eq!(
            result.placements[0].mutation_scope,
            Some(MutationScope::UntrustedExternal)
        );
        assert!(!result.placements[0].governable);
        assert!(result.skills[0].summary.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("did not read unsafe Skill link"))
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn legacy_placement_authority_is_unknown_and_cannot_authorize_mutation() {
        let value = serde_json::json!({
            "id": "placement_legacy",
            "skill_id": "skill_legacy",
            "agent": "codex",
            "root": "/tmp/skills",
            "directory": "/tmp/skills/example",
            "entrypoint": "/tmp/skills/example/SKILL.md",
            "content_digest": "sha256:legacy",
            "link_target": null,
            "link_status": "not_link",
            "default_exposed": true,
            "governable": true,
            "executable_files": [],
            "declared_name_matches_directory": true
        });
        let placement: SkillPlacement = serde_json::from_value(value).unwrap();

        assert_eq!(placement.owned_by_agent, None);
        assert_eq!(placement.mutation_scope, None);
        assert!(!placement.is_mutable());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_indirect_link_escape_without_reading_outside_skill() {
        use std::os::unix::fs::symlink;
        let root = temp_directory("indirect-escaping-link");
        let outside = temp_directory("indirect-outside");
        let outside_skill = outside.join("pkg");
        fs::create_dir_all(&outside_skill).unwrap();
        fs::write(
            outside_skill.join("SKILL.md"),
            "---\nname: private-outside-skill\ndescription: must not be read\n---\nprivate marker",
        )
        .unwrap();

        symlink(&outside, root.join("alias")).unwrap();
        symlink(root.join("alias/pkg"), root.join("skill")).unwrap();

        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert_eq!(result.placements.len(), 2);
        let placement = result
            .placements
            .iter()
            .find(|placement| placement.directory == root.join("skill"))
            .expect("indirect linked placement");
        assert_eq!(placement.link_status, LinkStatus::EscapesRoot);
        let skill = result
            .skills
            .iter()
            .find(|skill| skill.id == placement.skill_id)
            .expect("placeholder skill record");
        assert_ne!(skill.name, "private-outside-skill");
        assert!(skill.metadata.description.is_none());
        assert!(skill.summary.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("did not read unsafe Skill link"))
        );

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unresolved_in_root_link_remains_broken_and_unread() {
        use std::os::unix::fs::symlink;
        let root = temp_directory("broken-in-root-link");
        symlink(root.join("missing/pkg"), root.join("skill")).unwrap();

        let mut options = ScanOptions::for_home(root.join("empty-home"));
        options.explicit_skill_roots.push(ExplicitSkillRoot {
            agent: AgentKind::Codex,
            path: root.clone(),
        });
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert_eq!(result.placements.len(), 1);
        assert_eq!(result.placements[0].link_status, LinkStatus::Broken);
        assert!(result.skills[0].summary.is_empty());

        fs::remove_dir_all(root).unwrap();
    }
}
