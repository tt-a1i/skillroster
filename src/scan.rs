use crate::harness::{AgentKind, SessionSignal, known_agent_roots, session_record_observations};
use aho_corasick::{AhoCorasick, AhoCorasickBuilder};
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
const MAX_SESSION_DISCOVERY_FILES_PER_AGENT: usize = 10_000;
const MAX_SESSION_FILES_PER_AGENT: usize = 250;
const MAX_SESSION_BYTES_PER_AGENT: u64 = 4 * 1024 * 1024;
const MAX_SESSION_BYTES_PER_FILE: u64 = 512 * 1024;
const MAX_SESSION_LINES_PER_AGENT: usize = 20_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanOptions {
    pub home: PathBuf,
    pub explicit_skill_roots: Vec<ExplicitSkillRoot>,
    pub explicit_source_roots: Vec<PathBuf>,
    pub excluded_session_agents: BTreeSet<AgentKind>,
    pub include_session_evidence: bool,
    pub max_depth: usize,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillPlacement {
    pub id: String,
    pub skill_id: String,
    pub agent: Option<AgentKind>,
    pub root: PathBuf,
    pub directory: PathBuf,
    pub entrypoint: PathBuf,
    pub content_digest: String,
    pub link_target: Option<PathBuf>,
    pub link_status: LinkStatus,
    pub default_exposed: bool,
    /// Whether SkillRoster may include this placement in a mutating governance Plan.
    /// External provider caches are observed and searchable, but never governable.
    #[serde(default = "default_true")]
    pub governable: bool,
    /// External provider identity when the placement comes from an enabled plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default)]
    pub executable_files: Vec<PathBuf>,
    #[serde(default)]
    pub declared_name_matches_directory: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScannedSkill {
    pub id: String,
    pub name: String,
    pub metadata: SkillMetadata,
    pub content_digest: String,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageEvidence {
    pub agent: AgentKind,
    pub skill_id: String,
    pub stage: UsageStage,
    pub quality: EvidenceQuality,
    pub event_count: u64,
    pub first_seen_unix: Option<u64>,
    pub last_seen_unix: Option<u64>,
    /// A stable digest of the source path, never session content.
    pub source_path_digest: String,
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
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanResult {
    pub roots: Vec<RootObservation>,
    pub skills: Vec<ScannedSkill>,
    pub placements: Vec<SkillPlacement>,
    pub usage: Vec<UsageEvidence>,
    pub coverage: Vec<SessionCoverage>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
struct EntryCandidate {
    agent: Option<AgentKind>,
    root: PathBuf,
    governable: bool,
    provider: Option<String>,
    entrypoint: PathBuf,
    link_target: Option<PathBuf>,
    link_status: LinkStatus,
}

#[derive(Clone, Debug)]
struct SkillRootPolicy {
    agent: Option<AgentKind>,
    explicit: bool,
    governable: bool,
    detail: Option<String>,
    provider: Option<String>,
}

#[derive(Clone, Debug)]
struct CodexPluginSkillRoot {
    plugin_id: String,
    path: PathBuf,
}

#[derive(Default)]
struct PluginConfigState {
    sections: usize,
    enabled_values: Vec<bool>,
}

fn default_true() -> bool {
    true
}

fn codex_enabled_plugin_skill_roots(
    home: &Path,
    warnings: &mut Vec<String>,
) -> Vec<CodexPluginSkillRoot> {
    let config_path = home.join(".codex/config.toml");
    let config = match fs::read_to_string(&config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            warnings.push(format!(
                "could not read Codex plugin configuration {}: {error}",
                config_path.display()
            ));
            return Vec::new();
        }
    };
    let enabled_plugins = parse_enabled_codex_plugins(&config, warnings);
    let cache_root = home.join(".codex/plugins/cache");
    let mut roots = Vec::new();
    for (plugin, marketplace) in enabled_plugins {
        let plugin_id = format!("{plugin}@{marketplace}");
        let unresolved_plugin_cache = cache_root.join(&marketplace).join(&plugin);
        let Some(plugin_cache) = contained_directory(&cache_root, &unresolved_plugin_cache) else {
            warnings.push(format!(
                "enabled Codex plugin {plugin_id} has no contained local cache; skipped"
            ));
            continue;
        };
        let latest = plugin_cache.join("latest/skills");
        if let Some(path) = contained_directory(&plugin_cache, &latest) {
            roots.push(CodexPluginSkillRoot { plugin_id, path });
            continue;
        }
        let mut candidates = match fs::read_dir(&plugin_cache) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name() != "latest")
                .filter_map(|entry| {
                    contained_directory(&plugin_cache, &entry.path().join("skills"))
                })
                .collect::<Vec<_>>(),
            Err(error) => {
                warnings.push(format!(
                    "could not inspect enabled Codex plugin cache {}: {error}",
                    plugin_cache.display()
                ));
                continue;
            }
        };
        candidates.sort();
        candidates.dedup();
        match candidates.as_slice() {
            [path] => roots.push(CodexPluginSkillRoot {
                plugin_id,
                path: path.clone(),
            }),
            [] => {}
            _ => warnings.push(format!(
                "enabled Codex plugin {plugin_id} has multiple cached Skill versions and no unique latest version; skipped"
            )),
        }
    }
    roots.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    roots
}

fn parse_enabled_codex_plugins(config: &str, warnings: &mut Vec<String>) -> Vec<(String, String)> {
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

    let mut enabled = Vec::new();
    for (plugin_id, state) in sections {
        if state.sections != 1 || state.enabled_values.len() != 1 {
            warnings.push(format!(
                "Codex plugin {plugin_id} has ambiguous configuration; skipped"
            ));
            continue;
        }
        if !state.enabled_values[0] {
            continue;
        }
        let Some((plugin, marketplace)) = plugin_id.split_once('@') else {
            warnings.push(format!(
                "Codex plugin identifier {plugin_id} is invalid; skipped"
            ));
            continue;
        };
        if plugin.is_empty()
            || marketplace.is_empty()
            || marketplace.contains('@')
            || !safe_path_component(plugin)
            || !safe_path_component(marketplace)
        {
            warnings.push(format!(
                "Codex plugin identifier {plugin_id} is unsafe; skipped"
            ));
            continue;
        }
        enabled.push((plugin.to_owned(), marketplace.to_owned()));
    }
    enabled.sort();
    enabled
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
    let mut result = ScanResult::default();
    let known = known_agent_roots(&options.home);
    let plugin_roots = codex_enabled_plugin_skill_roots(&options.home, &mut result.warnings);
    let shared_roots = [
        options.home.join(".agents_skills"),
        options.home.join(".skillroster/library"),
    ];
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
        .collect::<Vec<_>>();
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
    for root in &options.explicit_source_roots {
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
                detail: Some(format!(
                    "Codex enabled plugin source {}; observed and searchable; not Roster-managed",
                    root.plugin_id
                )),
                provider: Some(root.plugin_id.clone()),
            },
            &approved_roots,
            options.max_depth,
            &mut result,
            &mut candidates,
        );
    }

    materialize_candidates(candidates, &mut result);

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
    });
    if status != RootStatus::Included {
        return;
    }

    if let Err(error) = discover_entrypoints(
        &policy,
        root,
        approved_roots,
        root,
        0,
        max_depth,
        candidates,
    ) {
        result.warnings.push(format!(
            "could not completely inspect skill root {}: {error}",
            root.display()
        ));
        if let Some(observation) = result.roots.last_mut() {
            observation.status = RootStatus::Inaccessible;
            observation.detail = Some(error.to_string());
        }
    }
}

fn discover_entrypoints(
    policy: &SkillRootPolicy,
    placement_root: &Path,
    approved_roots: &[PathBuf],
    directory: &Path,
    depth: usize,
    max_depth: usize,
    output: &mut Vec<EntryCandidate>,
) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let file_name = entry.file_name();
        if file_name == "SKILL.md" {
            let (target, link_status) = inspect_link(approved_roots, &path, &metadata);
            output.push(EntryCandidate {
                agent: policy.agent,
                root: placement_root.to_path_buf(),
                governable: policy.governable,
                provider: policy.provider.clone(),
                entrypoint: path,
                link_target: target,
                link_status,
            });
        } else if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            discover_entrypoints(
                policy,
                placement_root,
                approved_roots,
                &path,
                depth + 1,
                max_depth,
                output,
            )?;
        } else if metadata.file_type().is_symlink() {
            // A linked Skill directory is a placement too, but it is never
            // traversed recursively before the boundary has been evaluated.
            let linked_entrypoint = path.join("SKILL.md");
            let (target, status) = inspect_link(approved_roots, &path, &metadata);
            if linked_entrypoint.exists() || status != LinkStatus::Valid {
                output.push(EntryCandidate {
                    agent: policy.agent,
                    root: placement_root.to_path_buf(),
                    governable: policy.governable,
                    provider: policy.provider.clone(),
                    entrypoint: linked_entrypoint,
                    link_target: target,
                    link_status: status,
                });
            }
        }
    }
    Ok(())
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
    let target = if raw_target.is_absolute() {
        raw_target
    } else {
        path.parent().unwrap_or(Path::new(".")).join(raw_target)
    };
    let normalized_target = lexical_normalize(&target);
    if !approved_roots
        .iter()
        .map(|root| lexical_normalize(root))
        .any(|root| normalized_target.starts_with(root))
    {
        return (Some(normalized_target), LinkStatus::EscapesRoot);
    }

    // Lexical containment is only an early rejection. An in-root target can
    // still escape through a symlink in one of its parent components, so the
    // final resolved path must be contained by a resolved approved root too.
    // A target that cannot be fully resolved is never safe to read.
    let resolved_target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(_) => return (Some(normalized_target), LinkStatus::Broken),
    };
    let within_resolved_root = approved_roots.iter().any(|root| {
        fs::canonicalize(root)
            .map(|root| resolved_target.starts_with(root))
            .unwrap_or(false)
    });
    if !within_resolved_root {
        return (Some(normalized_target), LinkStatus::EscapesRoot);
    }
    if fs::metadata(path).is_err() {
        return (Some(normalized_target), LinkStatus::Broken);
    }
    (Some(normalized_target), LinkStatus::Valid)
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

fn materialize_candidates(candidates: Vec<EntryCandidate>, result: &mut ScanResult) {
    let mut skills = BTreeMap::<String, ScannedSkill>::new();
    for candidate in candidates {
        let directory = candidate
            .entrypoint
            .parent()
            .unwrap_or(&candidate.root)
            .to_path_buf();
        let safe_to_read = !matches!(
            candidate.link_status,
            LinkStatus::Broken | LinkStatus::EscapesRoot
        );
        let (content, modified_at) = if safe_to_read {
            match read_bounded(&candidate.entrypoint, MAX_SKILL_FILE_BYTES) {
                Ok(value) => value,
                Err(error) => {
                    result.warnings.push(format!(
                        "could not read Skill entrypoint {}: {error}",
                        candidate.entrypoint.display()
                    ));
                    (String::new(), None)
                }
            }
        } else {
            result.warnings.push(format!(
                "did not read unsafe Skill link {}",
                candidate.entrypoint.display()
            ));
            (String::new(), None)
        };
        let metadata = parse_skill_markdown(&content);
        let digest = if safe_to_read {
            match digest_skill_directory(&directory) {
                Ok(digest) => digest,
                Err(error) => {
                    result.warnings.push(format!(
                        "could not completely fingerprint Skill package {}: {error}",
                        directory.display()
                    ));
                    stable_digest(content.as_bytes())
                }
            }
        } else {
            stable_digest(candidate.entrypoint.to_string_lossy().as_bytes())
        };
        let identity_basis = match (&metadata.source, &metadata.version, &metadata.revision) {
            (Some(source), Some(version), _) => format!("source:{source}@{version}"),
            (Some(source), _, Some(revision)) => format!("source:{source}@{revision}"),
            _ if safe_to_read && !content.is_empty() => format!("content:{digest}"),
            _ => format!("unreadable-link:{}", candidate.entrypoint.display()),
        };
        let skill_id = format!("skill_{}", stable_digest(identity_basis.as_bytes()));
        let name = metadata
            .name
            .clone()
            .or_else(|| {
                directory
                    .file_name()
                    .map(|name| name.to_string_lossy().into())
            })
            .unwrap_or_else(|| "unnamed".into());
        let normalized_text = normalize_search_text(&content);
        let executable_files = if safe_to_read {
            executable_files(&directory).unwrap_or_default()
        } else {
            Vec::new()
        };
        let declared_name_matches_directory = metadata.name.as_ref().and_then(|declared| {
            directory.file_name().map(|directory_name| {
                declared.eq_ignore_ascii_case(&directory_name.to_string_lossy())
            })
        });
        skills
            .entry(skill_id.clone())
            .or_insert_with(|| ScannedSkill {
                id: skill_id.clone(),
                name,
                metadata,
                content_digest: digest.clone(),
                digest_algorithm: "sha256-v1".into(),
                summary: summarize_markdown(&content, 320),
                normalized_text,
                modified_at_unix: modified_at,
            });
        let placement_basis = format!(
            "{}\0{}\0{}",
            candidate.agent.map(AgentKind::id).unwrap_or("explicit"),
            candidate.root.display(),
            candidate.entrypoint.display()
        );
        result.placements.push(SkillPlacement {
            id: format!("placement_{}", stable_digest(placement_basis.as_bytes())),
            skill_id,
            agent: candidate.agent,
            root: candidate.root,
            directory,
            entrypoint: candidate.entrypoint,
            content_digest: digest,
            link_target: candidate.link_target,
            link_status: candidate.link_status,
            default_exposed: candidate.agent.is_some(),
            governable: candidate.governable,
            provider: candidate.provider,
            executable_files,
            declared_name_matches_directory,
        });
    }
    result.skills = skills.into_values().collect();
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
            index += 1;
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            index += 1;
            continue;
        };
        let key = key.trim();
        let value = value.trim();
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
                metadata.triggers.extend(
                    value
                        .trim_matches(['[', ']'])
                        .split(',')
                        .map(unquote)
                        .filter(|value| !value.is_empty()),
                );
            }
            _ => set_metadata_scalar(&mut metadata, key, unquote(value)),
        }
        index += 1;
    }
    metadata
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

fn digest_skill_directory(directory: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    collect_skill_files(directory, directory, 0, 8, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut digest = Sha256::new();
    let mut total_bytes = 0_u64;
    for (relative_path, path, file_type) in files {
        digest.update(relative_path.to_string_lossy().as_bytes());
        digest.update([0]);
        if file_type.is_symlink() {
            let target = fs::read_link(path)?;
            digest.update(b"symlink\0");
            digest.update(target.to_string_lossy().as_bytes());
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
            let mut file = File::open(path)?;
            io::copy(&mut file, &mut DigestWriter(&mut digest))?;
        }
        digest.update([0xff]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect_skill_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    max_depth: usize,
    output: &mut Vec<(PathBuf, PathBuf, fs::FileType)>,
) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
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
            collect_skill_files(root, &path, depth + 1, max_depth, output)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            let relative = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_path_buf();
            output.push((relative, path, file_type));
        }
    }
    Ok(())
}

struct DigestWriter<'a>(&'a mut Sha256);

impl io::Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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
    };
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
            skill_ids_by_reference
                .entry(normalize_reference_text(&entrypoint.to_string_lossy()))
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
    let mut events = BTreeMap::<(String, UsageStage, String), UsageEvidence>::new();
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
            RootStatus::Missing => coverage.roots_missing += 1,
            RootStatus::Inaccessible => coverage.roots_inaccessible += 1,
            RootStatus::Included | RootStatus::Excluded => {}
        }
        result.roots.push(RootObservation {
            agent: Some(agent),
            kind: RootKind::Sessions,
            path: root.clone(),
            status,
            explicit: false,
            detail: None,
        });
        if status != RootStatus::Included {
            continue;
        }
        let mut files = Vec::new();
        match collect_session_files(
            root,
            0,
            6,
            MAX_SESSION_DISCOVERY_FILES_PER_AGENT + 1,
            &mut files,
        ) {
            Ok(true) => {
                coverage.discovery_truncated = true;
                coverage.truncated = true;
            }
            Ok(false) => {}
            Err(_) => {
                coverage.files_skipped += 1;
                continue;
            }
        }
        let discovered_count = files.len();
        coverage.files_discovered = coverage.files_discovered.saturating_add(discovered_count);
        files.sort_by_cached_key(|path| {
            (std::cmp::Reverse(session_file_modified(path)), path.clone())
        });
        files.truncate(MAX_SESSION_DISCOVERY_FILES_PER_AGENT);
        if files.len() > MAX_SESSION_FILES_PER_AGENT {
            coverage.files_skipped = coverage
                .files_skipped
                .saturating_add(discovered_count - MAX_SESSION_FILES_PER_AGENT);
            coverage.truncated = true;
        }
        files.truncate(MAX_SESSION_FILES_PER_AGENT);
        for (index, file) in files.iter().enumerate() {
            let remaining_bytes = MAX_SESSION_BYTES_PER_AGENT.saturating_sub(bytes_observed);
            if remaining_bytes == 0 || lines_observed >= MAX_SESSION_LINES_PER_AGENT {
                coverage.files_skipped = coverage
                    .files_skipped
                    .saturating_add(files.len().saturating_sub(index));
                coverage.truncated = true;
                break;
            }
            let metadata = match fs::metadata(file) {
                Ok(metadata) => metadata,
                _ => {
                    coverage.files_skipped += 1;
                    continue;
                }
            };
            let is_json = file.extension().and_then(|extension| extension.to_str()) == Some("json");
            let sample_limit = remaining_bytes.min(MAX_SESSION_BYTES_PER_FILE);
            let (sample, sample_bytes) =
                match read_session_tail(file, metadata.len(), sample_limit, !is_json) {
                    Ok(sample) => sample,
                    Err(_) => {
                        coverage.files_skipped += 1;
                        continue;
                    }
                };
            let mut file_partially_observed = sample_bytes < metadata.len();
            if file_partially_observed {
                coverage.truncated = true;
            }
            if sample_bytes == 0 && metadata.len() != 0 {
                coverage.files_skipped += 1;
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
            let source_path_digest = stable_digest(file.to_string_lossy().as_bytes());
            let sample = String::from_utf8_lossy(&sample);
            let complete_json = is_json
                && sample_bytes == metadata.len()
                && serde_json::from_str::<serde_json::Value>(&sample).is_ok();
            let records = if complete_json {
                let physical_lines = sample.lines().count().max(1);
                vec![(sample.into_owned(), physical_lines)]
            } else if is_json {
                extract_complete_json_objects(&sample)
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
                    break;
                }
                lines_observed += physical_lines;
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
                            let key = (skill_id.clone(), stage, source_path_digest.clone());
                            let event = events.entry(key).or_insert_with(|| UsageEvidence {
                                agent,
                                skill_id,
                                stage,
                                quality,
                                event_count: 0,
                                first_seen_unix: timestamp,
                                last_seen_unix: timestamp,
                                source_path_digest: source_path_digest.clone(),
                            });
                            event.event_count += 1;
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
                    let key = (skill_id.clone(), stage, source_path_digest.clone());
                    let event = events.entry(key).or_insert_with(|| UsageEvidence {
                        agent,
                        skill_id: skill_id.clone(),
                        stage,
                        quality,
                        event_count: 0,
                        first_seen_unix: timestamp,
                        last_seen_unix: timestamp,
                        source_path_digest: source_path_digest.clone(),
                    });
                    event.event_count += 1;
                }
            }
            if file_partially_observed {
                coverage.files_partially_observed += 1;
            }
        }
    }
    coverage.bytes_observed = bytes_observed;
    coverage.lines_observed = lines_observed;
    coverage.denominator_reliable = coverage.roots_present > 0
        && coverage.files_skipped == 0
        && coverage.files_partially_observed == 0
        && !coverage.discovery_truncated;
    result.usage.extend(events.into_values());
    result.coverage.push(coverage);
}

fn session_file_modified(path: &Path) -> u64 {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| modified_unix(&metadata))
        .unwrap_or_default()
}

fn read_session_tail(
    path: &Path,
    file_bytes: u64,
    maximum: u64,
    align_to_line: bool,
) -> io::Result<(Vec<u8>, u64)> {
    let mut file = File::open(path)?;
    let offset = file_bytes.saturating_sub(maximum);
    file.seek(SeekFrom::Start(offset))?;
    let mut sample = Vec::with_capacity(maximum as usize);
    file.take(maximum).read_to_end(&mut sample)?;
    let bytes_read = sample.len() as u64;
    if offset != 0 && align_to_line {
        if let Some(newline) = sample.iter().position(|byte| *byte == b'\n') {
            sample.drain(..=newline);
        } else {
            sample.clear();
        }
    }
    Ok((sample, bytes_read))
}

fn extract_complete_json_objects(sample: &str) -> Vec<(String, usize)> {
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
    for (start, end) in candidates {
        if end <= covered_until {
            continue;
        }
        let candidate_bytes = (end - start) as u64;
        if parse_bytes.saturating_add(candidate_bytes) > 8 * MAX_SESSION_BYTES_PER_FILE {
            continue;
        }
        parse_bytes += candidate_bytes;
        if serde_json::from_slice::<serde_json::Value>(&bytes[start..end]).is_ok() {
            selected.push((start, end));
            covered_until = covered_until.max(end);
        }
    }
    selected.sort_by_key(|(start, _)| *start);
    if selected.len() > MAX_SESSION_LINES_PER_AGENT {
        selected.drain(..selected.len() - MAX_SESSION_LINES_PER_AGENT);
    }
    selected
        .into_iter()
        .map(|(start, end)| {
            let value = sample[start..end].to_owned();
            let lines = value.lines().count().max(1);
            (value, lines)
        })
        .collect()
}

fn collect_session_files(
    directory: &Path,
    depth: usize,
    max_depth: usize,
    maximum: usize,
    output: &mut Vec<PathBuf>,
) -> io::Result<bool> {
    if depth > max_depth || output.len() >= maximum {
        return Ok(output.len() >= maximum);
    }
    for entry in fs::read_dir(directory)? {
        if output.len() >= maximum {
            return Ok(true);
        }
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            if collect_session_files(&path, depth + 1, max_depth, maximum, output)? {
                return Ok(true);
            }
        } else if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("json" | "jsonl" | "log")
        ) {
            output.push(path);
        }
    }
    Ok(false)
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

fn executable_files(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_skill_files(directory, directory, 0, 8, &mut files)?;
    let mut executable = files
        .into_iter()
        .filter_map(|(_, path, file_type)| {
            if file_type.is_symlink() || !file_type.is_file() {
                return None;
            }
            let extension_is_script = path
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
            #[cfg(unix)]
            let mode_is_executable = {
                use std::os::unix::fs::PermissionsExt;
                fs::metadata(&path)
                    .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            };
            #[cfg(not(unix))]
            let mode_is_executable = false;
            (extension_is_script || mode_is_executable).then_some(path)
        })
        .collect::<Vec<_>>();
    executable.sort();
    executable.dedup();
    Ok(executable)
}

pub(crate) fn inspect_skill_identity(entrypoint: &Path) -> io::Result<(String, String)> {
    let directory = entrypoint
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Skill has no directory"))?;
    let (content, _) = read_bounded(entrypoint, MAX_SKILL_FILE_BYTES)?;
    let metadata = parse_skill_markdown(&content);
    let digest = digest_skill_directory(directory)?;
    let identity_basis = match (&metadata.source, &metadata.version, &metadata.revision) {
        (Some(source), Some(version), _) => format!("source:{source}@{version}"),
        (Some(source), _, Some(revision)) => format!("source:{source}@{revision}"),
        _ => format!("content:{digest}"),
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
        let plugins = parse_enabled_codex_plugins(
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
            plugins,
            vec![("browser".to_owned(), "openai-bundled".to_owned())]
        );
        assert!(warnings.iter().any(|warning| warning.contains("unsafe")));
        assert!(warnings.iter().any(|warning| warning.contains("ambiguous")));
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
        assert!(result.roots.iter().any(|seen| seen.explicit));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn explicit_source_root_approves_links_without_adding_agent_exposure() {
        let root = temp_directory("source-root");
        let home = root.join("home");
        let source_root = root.join("sources");
        let source_skill = source_root.join("external");
        fs::create_dir_all(&source_skill).unwrap();
        fs::write(
            source_skill.join("SKILL.md"),
            "---\nname: external\ndescription: External source Skill\n---\n",
        )
        .unwrap();
        let codex_root = home.join(".codex/skills");
        fs::create_dir_all(&codex_root).unwrap();
        std::os::unix::fs::symlink(&source_skill, codex_root.join("external")).unwrap();

        let mut options = ScanOptions::for_home(&home);
        options.explicit_source_roots.push(source_root.clone());
        options.include_session_evidence = false;
        let result = scan(&options).unwrap();

        assert!(result.warnings.is_empty());
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.placements.len(), 2);
        assert_eq!(
            result
                .placements
                .iter()
                .filter(|placement| placement.default_exposed)
                .count(),
            1
        );
        assert!(
            result
                .roots
                .iter()
                .any(|seen| { seen.path == source_root && seen.explicit && seen.agent.is_none() })
        );
        fs::remove_dir_all(root).unwrap();
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
        for index in 0..=MAX_SESSION_FILES_PER_AGENT {
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
            MAX_SESSION_FILES_PER_AGENT + 2
        );
        assert_eq!(
            result.coverage[0].files_observed,
            MAX_SESSION_FILES_PER_AGENT
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
