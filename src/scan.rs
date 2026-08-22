use crate::harness::{AgentKind, SessionSignal, classify_session_record, known_agent_roots};
use aho_corasick::AhoCorasickBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const MAX_SKILL_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SKILL_PACKAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SESSION_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SESSION_FILES_PER_AGENT: usize = 250;
const MAX_SESSION_BYTES_PER_AGENT: u64 = 4 * 1024 * 1024;
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
    pub files_observed: usize,
    pub files_skipped: usize,
    pub denominator_reliable: bool,
    pub bytes_observed: u64,
    pub lines_observed: usize,
    pub truncated: bool,
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
    entrypoint: PathBuf,
    link_target: Option<PathBuf>,
    link_status: LinkStatus,
}

pub fn scan(options: &ScanOptions) -> io::Result<ScanResult> {
    let mut result = ScanResult::default();
    let known = known_agent_roots(&options.home);
    let shared_roots = [
        options.home.join(".agents_skills"),
        options.home.join(".skillroster/library"),
    ];
    let approved_roots = known
        .iter()
        .flat_map(|roots| roots.skill_roots.iter().cloned())
        .chain(shared_roots.iter().cloned())
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
                Some(roots.agent),
                root,
                false,
                &approved_roots,
                options.max_depth,
                &mut result,
                &mut candidates,
            );
        }
    }
    for root in shared_roots.iter().filter(|path| path.exists()) {
        observe_skill_root(
            None,
            root,
            false,
            &approved_roots,
            options.max_depth,
            &mut result,
            &mut candidates,
        );
    }
    for root in &options.explicit_source_roots {
        observe_skill_root(
            None,
            root,
            true,
            &approved_roots,
            options.max_depth,
            &mut result,
            &mut candidates,
        );
    }
    for root in &options.explicit_skill_roots {
        observe_skill_root(
            Some(root.agent),
            &root.path,
            true,
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
    agent: Option<AgentKind>,
    root: &Path,
    explicit: bool,
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
        agent,
        kind: RootKind::Skills,
        path: root.to_path_buf(),
        status,
        explicit,
        detail: None,
    });
    if status != RootStatus::Included {
        return;
    }

    if let Err(error) =
        discover_entrypoints(agent, root, approved_roots, root, 0, max_depth, candidates)
    {
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
    agent: Option<AgentKind>,
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
                agent,
                root: placement_root.to_path_buf(),
                entrypoint: path,
                link_target: target,
                link_status,
            });
        } else if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            discover_entrypoints(
                agent,
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
                    agent,
                    root: placement_root.to_path_buf(),
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
            executable_files,
            declared_name_matches_directory,
        });
    }
    result.skills = skills.into_values().collect();
}

pub fn parse_skill_markdown(markdown: &str) -> SkillMetadata {
    let mut metadata = SkillMetadata::default();
    let Some(frontmatter) = markdown.strip_prefix("---\n") else {
        return metadata;
    };
    let Some((frontmatter, _)) = frontmatter.split_once("\n---") else {
        return metadata;
    };
    let mut list_key: Option<&str> = None;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("- ") {
            if list_key == Some("triggers") {
                metadata.triggers.push(unquote(value));
            }
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        list_key = Some(key);
        if value.is_empty() {
            continue;
        }
        match key {
            "name" => metadata.name = Some(unquote(value)),
            "description" => metadata.description = Some(unquote(value)),
            "source" | "repository" => metadata.source = Some(unquote(value)),
            "version" => metadata.version = Some(unquote(value)),
            "revision" | "rev" | "commit" => metadata.revision = Some(unquote(value)),
            "triggers" => {
                metadata.triggers.extend(
                    value
                        .trim_matches(['[', ']'])
                        .split(',')
                        .map(unquote)
                        .filter(|value| !value.is_empty()),
                );
            }
            _ => {}
        }
    }
    metadata
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches(['\'', '"']).trim().to_string()
}

fn summarize_markdown(markdown: &str, limit: usize) -> String {
    let body = if let Some(frontmatter) = markdown.strip_prefix("---\n") {
        frontmatter
            .split_once("\n---")
            .map(|(_, body)| body)
            .unwrap_or(markdown)
    } else {
        markdown
    };
    let summary = body
        .lines()
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
        files_observed: 0,
        files_skipped: 0,
        denominator_reliable: false,
        bytes_observed: 0,
        lines_observed: 0,
        truncated: false,
        first_seen_unix: None,
        last_seen_unix: None,
    };
    let skill_lookup: Vec<(String, String)> = result
        .skills
        .iter()
        .map(|skill| (skill.id.clone(), skill.name.to_lowercase()))
        .collect();
    let patterns = skill_lookup
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>();
    let matcher = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .build(&patterns)
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
        match collect_session_files(root, 0, 6, MAX_SESSION_FILES_PER_AGENT + 1, &mut files) {
            Ok(true) => {
                coverage.files_skipped += 1;
                coverage.truncated = true;
            }
            Ok(false) => {}
            Err(_) => {
                coverage.files_skipped += 1;
                continue;
            }
        }
        files.sort_by_key(|path| {
            std::cmp::Reverse(
                fs::metadata(path)
                    .ok()
                    .and_then(|metadata| modified_unix(&metadata))
                    .unwrap_or_default(),
            )
        });
        files.truncate(MAX_SESSION_FILES_PER_AGENT);
        for file in files {
            let metadata = match fs::metadata(&file) {
                Ok(metadata) if metadata.len() <= MAX_SESSION_FILE_BYTES => metadata,
                _ => {
                    coverage.files_skipped += 1;
                    continue;
                }
            };
            if bytes_observed.saturating_add(metadata.len()) > MAX_SESSION_BYTES_PER_AGENT
                || lines_observed >= MAX_SESSION_LINES_PER_AGENT
            {
                coverage.files_skipped += 1;
                coverage.truncated = true;
                break;
            }
            bytes_observed = bytes_observed.saturating_add(metadata.len());
            let timestamp = modified_unix(&metadata);
            let reader = match File::open(&file).map(BufReader::new) {
                Ok(reader) => reader,
                Err(_) => {
                    coverage.files_skipped += 1;
                    continue;
                }
            };
            coverage.files_observed += 1;
            update_window(
                &mut coverage.first_seen_unix,
                &mut coverage.last_seen_unix,
                timestamp,
            );
            let source_path_digest = stable_digest(file.to_string_lossy().as_bytes());
            for line in reader.lines().map_while(Result::ok) {
                if lines_observed >= MAX_SESSION_LINES_PER_AGENT {
                    coverage.files_skipped += 1;
                    coverage.truncated = true;
                    break;
                }
                lines_observed += 1;
                let lower = line.to_lowercase();
                let Some(matcher) = &matcher else { continue };
                for matched in matcher.find_iter(&lower) {
                    let (skill_id, skill_name) = &skill_lookup[matched.pattern().as_usize()];
                    let (stage, quality) = classify_usage_line(agent, &line, skill_name);
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
        }
    }
    coverage.bytes_observed = bytes_observed;
    coverage.lines_observed = lines_observed;
    coverage.denominator_reliable = coverage.roots_present > 0 && coverage.files_skipped == 0;
    result.usage.extend(events.into_values());
    result.coverage.push(coverage);
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

fn classify_usage_line(
    agent: AgentKind,
    line: &str,
    matched_skill_name: &str,
) -> (UsageStage, EvidenceQuality) {
    match classify_session_record(agent, line) {
        Some(_)
            if agent == AgentKind::Codex
                && !codex_line_explicitly_references_skill(line, matched_skill_name)
                && !line_has_explicit_skill_field(line, matched_skill_name) =>
        {
            (UsageStage::Exposed, EvidenceQuality::Inferred)
        }
        Some(SessionSignal::Outcome) => (UsageStage::Outcome, EvidenceQuality::Observed),
        Some(SessionSignal::Applied) => (UsageStage::Applied, EvidenceQuality::Observed),
        Some(SessionSignal::Loaded) => (UsageStage::Loaded, EvidenceQuality::Observed),
        Some(SessionSignal::Matched) => (UsageStage::Matched, EvidenceQuality::Observed),
        None => (UsageStage::Exposed, EvidenceQuality::Inferred),
    }
}

fn line_has_explicit_skill_field(line: &str, skill_name: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    let payload = object.get("payload").and_then(serde_json::Value::as_object);
    [
        "skill_id",
        "skill_name",
        "selected_skill",
        "matched_skill",
        "loaded_skill",
        "applied_skill",
        "invoked_skill",
        "outcome_skill",
    ]
    .into_iter()
    .any(|field| {
        [
            object.get(field),
            payload.and_then(|payload| payload.get(field)),
        ]
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .any(|value| value.eq_ignore_ascii_case(skill_name))
    })
}

fn codex_line_explicitly_references_skill(line: &str, skill_name: &str) -> bool {
    let line = line.to_ascii_lowercase();
    let skill_name = skill_name.to_ascii_lowercase();
    [
        format!("/{skill_name}/skill.md"),
        format!("\\{skill_name}\\skill.md"),
        format!("${skill_name}]("),
    ]
    .iter()
    .any(|reference| line.contains(reference))
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
    scan.usage.iter().map(|usage| usage.agent).collect()
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
    fn parses_relevant_frontmatter_without_copying_body() {
        let metadata = parse_skill_markdown(
            "---\nname: research\ndescription: Find facts\nsource: example/repo\nrevision: abc\ntriggers: [search, verify]\n---\nSecret prompt body",
        );
        assert_eq!(metadata.name.as_deref(), Some("research"));
        assert_eq!(metadata.revision.as_deref(), Some("abc"));
        assert_eq!(metadata.triggers, ["search", "verify"]);
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
        let skill = home.join(".codex/skills/research");
        let sessions = home.join(".codex/sessions");
        fs::create_dir_all(&skill).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: research\ndescription: Primary-source research\n---\n",
        )
        .unwrap();
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
                    "base_instructions": "research: /skills/research/SKILL.md"
                }
            }),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": "Plan this with [$research](/skills/research/SKILL.md)"
                    }]
                }
            }),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "name": "exec",
                    "input": "await tools.exec_command({cmd: \"sed -n '1,80p' /skills/research/SKILL.md\"})"
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
            .find(|skill| skill.name == "research")
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
        assert!(stages.contains(&(UsageStage::Matched, EvidenceQuality::Observed)));
        assert!(stages.contains(&(UsageStage::Loaded, EvidenceQuality::Observed)));
        assert!(
            result
                .usage
                .iter()
                .filter(|usage| usage.skill_id == *generic_skill_id)
                .all(|usage| usage.stage == UsageStage::Exposed)
        );
        fs::remove_dir_all(home).unwrap();
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
