use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

const SOURCE_CONFIRMATION_JSON_LIMIT: usize = 10;
const SOURCE_CONFIRMATION_SCHEMA_VERSION: u32 = 1;

use crate::change::{self, LibraryChangeAction, RosterChange};
use crate::harness::AgentKind;
use crate::model::RosterState;
use crate::scan::{LinkStatus, RootKind, RootStatus, ScanResult, SkillPlacement};

#[derive(Debug)]
pub struct DerivedRosterPlan {
    pub operations: Vec<Value>,
    pub implicit_library_changes: Vec<LibraryChangeAction>,
    pub impact: Value,
}

#[derive(Debug)]
pub struct RosterPhysicalConflict {
    pub skill_id: String,
    pub agents: Vec<String>,
}

impl std::fmt::Display for RosterPhysicalConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Skill {} has incompatible Core and non-Core requests across one shared physical placement for Agents {}; separate the shared Agent roots or request a consistent exposure state",
            self.skill_id,
            self.agents.join(", ")
        )
    }
}

impl std::error::Error for RosterPhysicalConflict {}

#[derive(Debug)]
pub struct RosterOperationConflict {
    pub identity_role: &'static str,
    pub operation_kinds: Vec<String>,
    pub path: PathBuf,
}

impl std::fmt::Display for RosterOperationConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Roster Plan contains conflicting {} operations ({}) for physical path {}; resolve overlapping physical ownership or the same-name variant Finding before retrying",
            self.identity_role,
            self.operation_kinds.join(", "),
            self.path.display()
        )
    }
}

impl std::error::Error for RosterOperationConflict {}

#[derive(Debug)]
pub enum RosterSafetyBlocker {
    ProviderManaged {
        skill_id: String,
        placement_ids: Vec<String>,
    },
    DependentSource {
        skill_id: String,
        placement_ids: Vec<String>,
    },
}

impl std::fmt::Display for RosterSafetyBlocker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProviderManaged { skill_id, .. } => write!(
                formatter,
                "Skill {skill_id} includes a provider-managed placement that is read-only"
            ),
            Self::DependentSource { skill_id, .. } => write!(
                formatter,
                "Skill {skill_id} has a non-Agent source link that depends on a placement scheduled for removal"
            ),
        }
    }
}

impl std::error::Error for RosterSafetyBlocker {}

#[derive(Clone, Debug)]
pub struct RosterChangeExclusion {
    pub agent: String,
    pub skill_id: String,
    pub name: String,
    pub reason: &'static str,
    pub observed_source_target: Option<PathBuf>,
}

pub struct SupportedRosterChanges {
    pub changes: Vec<RosterChange>,
    pub exclusions: Vec<RosterChangeExclusion>,
}

#[derive(Debug)]
pub struct RosterPlanBlocked {
    pub message: String,
    pub relevant_ids: Vec<String>,
    pub paths: Vec<String>,
    pub details: Value,
}

impl std::fmt::Display for RosterPlanBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RosterPlanBlocked {}

/// Fail closed with bounded, typed source-confirmation evidence for the requested budget.
pub fn source_confirmation_block(
    finding_id: &str,
    core_budget: usize,
    exclusions: &[RosterChangeExclusion],
    state_dir: &Path,
) -> Result<RosterPlanBlocked> {
    let mut exclusions = exclusions.to_vec();
    exclusions.sort_by(|left, right| {
        (&left.agent, &left.name, &left.skill_id).cmp(&(&right.agent, &right.name, &right.skill_id))
    });
    let source_roots = minimum_reviewed_source_roots(
        exclusions
            .iter()
            .filter_map(|exclusion| exclusion.observed_source_target.clone()),
    );
    let source_root_paths = source_roots
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let blocked_changes = exclusions
        .iter()
        .map(blocked_change_json)
        .collect::<Vec<_>>();
    let skill_ids = exclusions
        .iter()
        .map(|exclusion| exclusion.skill_id.clone())
        .collect::<Vec<_>>();
    let blocked_change_count = exclusions.len();
    let source_root_count = source_root_paths.len();
    let changes_truncated = blocked_change_count > SOURCE_CONFIRMATION_JSON_LIMIT;
    let roots_truncated = source_root_count > SOURCE_CONFIRMATION_JSON_LIMIT;
    let bounded_changes = blocked_changes
        .iter()
        .take(SOURCE_CONFIRMATION_JSON_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    let bounded_roots = source_root_paths
        .iter()
        .take(SOURCE_CONFIRMATION_JSON_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    let mut relevant_ids = vec![finding_id.to_string()];
    relevant_ids.extend(
        skill_ids
            .iter()
            .take(SOURCE_CONFIRMATION_JSON_LIMIT)
            .cloned(),
    );
    let mut details = json!({
        "reason": "trusted_canonical_sources_required",
        "decision": "confirm_trusted_source_roots",
        "automatic_change_supported": false,
        "requested_core_budget": core_budget,
        "blocked_change_count": blocked_change_count,
        "blocked_changes": bounded_changes,
        "blocked_changes_truncated": changes_truncated,
        "source_root_count": source_root_count,
        "source_roots": bounded_roots.clone(),
        "source_roots_truncated": roots_truncated,
        "after_confirmation": {
            "repeatable_option": "--source-root",
            "source_roots": bounded_roots.clone(),
            "argv_template": [
                "skillroster",
                "--source-root",
                "<confirmed-canonical-source-directory>",
                "scan",
                "--json"
            ],
            "next": "rescan with only the reported reviewed source roots and retry the same Plan request"
        },
        "files_changed": false,
        "agent_files_changed": false,
        "library_files_changed": false,
        "state_files_changed": changes_truncated || roots_truncated,
        "detail_artifact_created": changes_truncated || roots_truncated
    });
    if changes_truncated || roots_truncated {
        details["detail"] = json!({
            "path": write_source_confirmation_detail(
                state_dir,
                json!({
                    "schema_version": SOURCE_CONFIRMATION_SCHEMA_VERSION,
                    "reason": "trusted_canonical_sources_required",
                    "decision": "confirm_trusted_source_roots",
                    "requested_core_budget": core_budget,
                    "blocked_change_count": blocked_change_count,
                    "blocked_changes": blocked_changes,
                    "skill_ids": skill_ids,
                    "source_root_count": source_root_count,
                    "source_roots": source_root_paths,
                    "after_confirmation": {
                        "repeatable_option": "--source-root",
                        "source_roots": source_root_paths,
                        "argv": scan_with_source_roots_argv(&source_root_paths)
                    }
                }),
            )?
        });
    }
    Ok(RosterPlanBlocked {
        message: format!(
            "Finding {finding_id} is blocked by {blocked_change_count} Roster changes without owned exact content at core_budget {core_budget}; confirm the reported source roots, rescan, and use the new Finding"
        ),
        relevant_ids,
        paths: bounded_roots,
        details,
    })
}

fn blocked_change_json(exclusion: &RosterChangeExclusion) -> Value {
    let mut item = json!({
        "agent": exclusion.agent,
        "skill_id": exclusion.skill_id,
        "name": exclusion.name,
        "reason": exclusion.reason,
        "state": "unchanged"
    });
    if let Some(target) = &exclusion.observed_source_target {
        item["observed_source_target"] = json!(target);
    }
    item
}

fn scan_with_source_roots_argv(source_roots: &[String]) -> Vec<String> {
    let mut argv = vec!["skillroster".into()];
    for root in source_roots {
        argv.push("--source-root".into());
        argv.push(root.clone());
    }
    argv.push("scan".into());
    argv.push("--json".into());
    argv
}

fn write_source_confirmation_detail(state_dir: &Path, complete: Value) -> Result<PathBuf> {
    let directory = state_dir.join("source-confirmation");
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!(
                "refusing invalid source-confirmation directory: {}",
                directory.display()
            );
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&directory)
                .with_context(|| format!("cannot create {}", directory.display()))?;
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", directory.display()));
        }
    }
    let id = ulid::Ulid::new();
    let path = directory.join(format!("{id}.json"));
    let temporary_path = directory.join(format!(".{id}.tmp"));
    let bytes = serde_json::to_vec(&complete)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary_path)
        .with_context(|| format!("cannot create {}", temporary_path.display()))?;
    let write_result = (|| -> Result<()> {
        file.write_all(&bytes)
            .with_context(|| format!("cannot write {}", temporary_path.display()))?;
        file.sync_all()
            .with_context(|| format!("cannot sync {}", temporary_path.display()))?;
        drop(file);
        std::fs::rename(&temporary_path, &path).with_context(|| {
            format!(
                "cannot publish source-confirmation detail {}",
                path.display()
            )
        })?;
        Ok(())
    })();
    if write_result.is_err() {
        match std::fs::remove_file(&temporary_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }
    write_result?;
    Ok(path)
}

#[cfg(test)]
pub fn test_absolute_path(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" });
    path.extend(
        relative
            .split('/')
            .filter(|component| !component.is_empty()),
    );
    path
}

/// Keep the narrowest observed `--source-root` set without synthesizing broader trust.
pub fn minimum_reviewed_source_roots(targets: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let unique = targets
        .into_iter()
        .filter(|path| path.is_absolute() && path.parent().is_some())
        .collect::<BTreeSet<_>>();
    let mut kept = Vec::new();
    for root in unique {
        if kept
            .iter()
            .any(|existing: &PathBuf| root.starts_with(existing))
        {
            continue;
        }
        kept.push(root);
    }
    kept
}

fn safe_observed_source_target(removable: &[&SkillPlacement]) -> Option<PathBuf> {
    removable
        .iter()
        .filter(|placement| placement.link_status == LinkStatus::EscapesRoot)
        .filter_map(|placement| placement.link_target.as_ref())
        .filter(|target| target.is_absolute())
        .min()
        .cloned()
}

/// Keep a semantic bulk recommendation useful without weakening raw Plan safety.
/// A demotion is excluded when no exact owned placement can remain or be migrated,
/// or when a non-Agent source link depends on a placement that would be removed.
pub fn exclude_unpreservable_demotions(
    scan: &ScanResult,
    changes: Vec<RosterChange>,
) -> Result<SupportedRosterChanges> {
    let mut by_skill = BTreeMap::<&str, Vec<&RosterChange>>::new();
    for change in &changes {
        agent(&change.agent)?;
        by_skill.entry(&change.skill_id).or_default().push(change);
    }
    let mut excluded_pairs = BTreeSet::new();
    let mut exclusions = Vec::new();
    for (skill_id, requests) in by_skill {
        let placements = scan
            .placements
            .iter()
            .filter(|placement| placement.skill_id == skill_id)
            .collect::<Vec<_>>();
        let demoted_agents = requests
            .iter()
            .filter(|request| request.state != "core")
            .map(|request| agent(&request.agent))
            .collect::<Result<BTreeSet<_>>>()?;
        if demoted_agents.is_empty() {
            continue;
        }
        let removable = placements
            .iter()
            .copied()
            .filter(|placement| {
                placement
                    .agent
                    .is_some_and(|agent| demoted_agents.contains(&agent))
            })
            .collect::<Vec<_>>();
        let provider_managed_removal =
            provider_placements_on_physical_removal(&placements, &removable)
                .next()
                .is_some();
        let skill = scan
            .skills
            .iter()
            .find(|skill| skill.id == skill_id)
            .ok_or_else(|| anyhow!("Skill {skill_id} is not in the latest Snapshot"))?;
        let exact_digest = skill.content_digest.as_str();
        let name = skill.name.as_str();
        let retained_owned = placements
            .iter()
            .copied()
            .filter(|placement| !removable.iter().any(|item| item.id == placement.id))
            .any(|placement| is_real_exact(placement, exact_digest));
        let migratable_owned = removable
            .iter()
            .copied()
            .any(|placement| is_real_exact(placement, exact_digest));
        let non_agent_source_dependency = placements
            .iter()
            .copied()
            .filter(|placement| placement.agent.is_none())
            .any(|placement| depends_on_physical_removal(placement, &removable));
        let reason = if provider_managed_removal {
            "provider_managed_placement_is_read_only"
        } else if non_agent_source_dependency {
            "non_agent_source_link_depends_on_removal"
        } else if retained_owned || migratable_owned {
            continue;
        } else {
            "no_owned_exact_content_to_preserve"
        };
        for request in requests
            .into_iter()
            .filter(|request| request.state != "core")
        {
            excluded_pairs.insert((request.agent.clone(), request.skill_id.clone()));
            exclusions.push(RosterChangeExclusion {
                agent: request.agent.clone(),
                skill_id: request.skill_id.clone(),
                name: name.to_owned(),
                reason,
                observed_source_target: safe_observed_source_target(&removable),
            });
        }
    }
    let changes = changes
        .into_iter()
        .filter(|change| !excluded_pairs.contains(&(change.agent.clone(), change.skill_id.clone())))
        .collect();
    Ok(SupportedRosterChanges {
        changes,
        exclusions,
    })
}

pub fn derive(
    scan: &ScanResult,
    state_dir: &Path,
    changes: &[RosterChange],
) -> Result<DerivedRosterPlan> {
    let mut by_skill = BTreeMap::<&str, Vec<&RosterChange>>::new();
    let mut pairs = HashSet::new();
    for change in changes {
        let agent = agent(&change.agent)?;
        if !pairs.insert((change.skill_id.as_str(), agent)) {
            bail!(
                "duplicate Roster request for Agent {} and Skill {}",
                change.agent,
                change.skill_id
            );
        }
        by_skill.entry(&change.skill_id).or_default().push(change);
    }

    let library_root = state_dir.join("library");
    let backup_root = state_dir.join("plan-backups");
    let nonce = ulid::Ulid::new().to_string();
    let mut operations = Vec::new();
    let mut implicit_library_changes = Vec::new();
    let mut needs_library_root = false;
    let mut needs_backup_root = false;
    let mut before_exposure = 0_usize;
    let mut after_exposure = 0_usize;
    let mut affected_agents = BTreeSet::new();
    let mut affected_skills = BTreeSet::new();
    let mut affected_placements = BTreeSet::new();
    let mut operation_groups = BTreeMap::<&str, usize>::new();
    let mut exclusions = Vec::new();
    let mut retargeted_placements = BTreeSet::new();

    for (skill_id, requests) in by_skill {
        let skill = scan
            .skills
            .iter()
            .find(|skill| skill.id == skill_id)
            .ok_or_else(|| anyhow!("Skill {skill_id} is not in the latest Snapshot"))?;
        let placements = scan
            .placements
            .iter()
            .filter(|placement| placement.skill_id == skill_id)
            .collect::<Vec<_>>();
        if placements.is_empty() {
            bail!("Skill {skill_id} has no verified placement");
        }
        if !placements.iter().any(|placement| placement.governable) {
            return Err(RosterSafetyBlocker::ProviderManaged {
                skill_id: skill_id.to_owned(),
                placement_ids: placements
                    .iter()
                    .map(|placement| placement.id.clone())
                    .collect(),
            }
            .into());
        }
        let desired = requests
            .iter()
            .map(|request| Ok((agent(&request.agent)?, roster_state(&request.state)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        ensure_physical_exposure_compatible(skill_id, &placements, &desired)?;
        let removal = placements
            .iter()
            .copied()
            .filter(|placement| {
                placement
                    .agent
                    .and_then(|agent| desired.get(&agent))
                    .is_some_and(|state| state != &RosterState::Core)
            })
            .collect::<Vec<_>>();
        for placement in &removal {
            placement.validated_physical_directory()?;
        }
        let provider_placement_ids = provider_placements_on_physical_removal(&placements, &removal)
            .map(|placement| placement.id.clone())
            .collect::<Vec<_>>();
        if !provider_placement_ids.is_empty() {
            return Err(RosterSafetyBlocker::ProviderManaged {
                skill_id: skill_id.to_owned(),
                placement_ids: provider_placement_ids,
            }
            .into());
        }
        let mut source = verified_real_source(&placements, &removal, &skill.content_digest);
        let mut source_fingerprint = source
            .as_ref()
            .map(|path| change::fingerprint(path))
            .transpose()?;
        let mut migrated_source = None;

        if source.is_none() && !removal.is_empty() {
            let canonical = removal
                .iter()
                .copied()
                .find(|placement| is_real_exact(placement, &skill.content_digest))
                .ok_or_else(|| {
                    anyhow!(
                        "Skill {skill_id} has no owned exact-digest content to preserve before exposure removal"
                    )
                })?;
            let library_path = library_root.join(safe_name(&skill.name)?);
            if library_path.exists() {
                bail!("Library target {} already exists", library_path.display());
            }
            needs_library_root |= !library_root.exists();
            let canonical_source = physical_mutation_path(canonical);
            let canonical_fingerprint = change::fingerprint(&canonical_source)?;
            operations.push(json!({
                "kind": "move_recoverable",
                "source": canonical_source,
                "target": library_path,
                "expected_fingerprint": canonical_fingerprint
            }));
            *operation_groups.entry("host_canonical").or_default() += 1;
            source = Some(library_path.clone());
            source_fingerprint = Some(canonical_fingerprint);
            migrated_source = Some(physical_mutation_path(canonical));
            implicit_library_changes.push(LibraryChangeAction {
                skill_id: skill_id.to_string(),
                canonical_placement_id: canonical.id.clone(),
                placement_ids: placements
                    .iter()
                    .map(|placement| placement.id.clone())
                    .collect(),
                requested_state: "hosted".into(),
                canonical_path: physical_mutation_path(canonical),
                library_path: Some(library_path),
            });
        }

        let dependent_links = placements
            .iter()
            .copied()
            .filter(|placement| !removal.iter().any(|removed| removed.id == placement.id))
            .filter(|placement| depends_on_physical_removal(placement, &removal))
            .collect::<Vec<_>>();
        if dependent_links
            .iter()
            .any(|placement| placement.agent.is_none())
        {
            return Err(RosterSafetyBlocker::DependentSource {
                skill_id: skill_id.to_owned(),
                placement_ids: dependent_links
                    .iter()
                    .filter(|placement| placement.agent.is_none())
                    .map(|placement| placement.id.clone())
                    .collect(),
            }
            .into());
        }
        if !dependent_links.is_empty() {
            let source = source
                .clone()
                .ok_or_else(|| anyhow!("Skill {skill_id} has no verified canonical source"))?;
            let source_fingerprint = source_fingerprint
                .clone()
                .ok_or_else(|| anyhow!("Skill {skill_id} source fingerprint is unavailable"))?;
            for placement in dependent_links {
                needs_backup_root |= !backup_root.exists();
                before_exposure += usize::from(placement.default_exposed);
                after_exposure += usize::from(placement.default_exposed);
                affected_placements.insert(placement.id.clone());
                if let Some(agent) = placement.agent {
                    affected_agents.insert(agent.id().to_owned());
                }
                retargeted_placements.insert(placement.id.clone());
                operations.push(json!({
                    "kind": "move_recoverable",
                    "source": placement.directory,
                    "target": backup_root.join(format!("{nonce}-{}", placement.id)),
                    "expected_fingerprint": change::fingerprint(&placement.directory)?
                }));
                *operation_groups
                    .entry("preserve_dependent_link")
                    .or_default() += 1;
                operations.push(json!({
                    "kind": "create_symlink",
                    "source": source,
                    "target": placement.directory,
                    "expected_fingerprint": "missing",
                    "expected_source_fingerprint": source_fingerprint
                }));
                *operation_groups
                    .entry("retarget_dependent_link")
                    .or_default() += 1;
            }
        }

        let mut physical_removals = BTreeMap::<PathBuf, &SkillPlacement>::new();
        for placement in &removal {
            before_exposure += usize::from(placement.default_exposed);
            affected_placements.insert(placement.id.clone());
            physical_removals
                .entry(physical_mutation_path(placement))
                .or_insert(placement);
        }
        for (physical_source, placement) in physical_removals {
            if migrated_source.as_ref() == Some(&physical_source) {
                continue;
            }
            needs_backup_root |= !backup_root.exists();
            let backup = backup_root.join(format!("{nonce}-{}", placement.id));
            let expected_fingerprint = change::fingerprint(&physical_source)?;
            operations.push(json!({
                "kind": "move_recoverable",
                "source": physical_source,
                "target": backup,
                "expected_fingerprint": expected_fingerprint
            }));
            *operation_groups.entry("remove_exposure").or_default() += 1;
        }

        for request in requests.iter().filter(|request| request.state == "core") {
            let requested_agent = agent(&request.agent)?;
            let existing = placements
                .iter()
                .copied()
                .filter(|placement| placement.agent == Some(requested_agent))
                .collect::<Vec<_>>();
            before_exposure += existing
                .iter()
                .filter(|placement| !retargeted_placements.contains(&placement.id))
                .filter(|placement| placement.default_exposed)
                .count();
            if existing
                .iter()
                .any(|placement| retargeted_placements.contains(&placement.id))
            {
                exclusions.push(json!({
                    "agent": request.agent,
                    "skill_id": skill_id,
                    "reason": "retargeted_from_removed_canonical"
                }));
                continue;
            }
            if existing.iter().any(|placement| {
                placement.default_exposed
                    && placement.link_status != LinkStatus::Broken
                    && placement.content_digest == skill.content_digest
                    && !placement.link_target.as_ref().is_some_and(|target| {
                        removal.iter().any(|removed| target == &removed.directory)
                    })
            }) {
                after_exposure += 1;
                exclusions.push(json!({
                    "agent": request.agent,
                    "skill_id": skill_id,
                    "reason": "already_exposed_exact_content"
                }));
                continue;
            }
            let source = source
                .clone()
                .or_else(|| verified_real_source(&placements, &[], &skill.content_digest))
                .ok_or_else(|| anyhow!("Skill {skill_id} has no verified canonical source"))?;
            let source_fingerprint = source_fingerprint
                .clone()
                .or_else(|| change::fingerprint(&source).ok())
                .ok_or_else(|| anyhow!("Skill {skill_id} source fingerprint is unavailable"))?;
            for placement in existing {
                needs_backup_root |= !backup_root.exists();
                affected_placements.insert(placement.id.clone());
                operations.push(json!({
                    "kind": "move_recoverable",
                    "source": placement.directory,
                    "target": backup_root.join(format!("{nonce}-{}", placement.id)),
                    "expected_fingerprint": change::fingerprint(&placement.directory)?
                }));
                *operation_groups
                    .entry("replace_invalid_exposure")
                    .or_default() += 1;
            }
            let target_root = agent_root(scan, requested_agent)?;
            let target = target_root.join(safe_name(&skill.name)?);
            operations.push(json!({
                "kind": "create_symlink",
                "source": source,
                "target": target,
                "expected_fingerprint": "missing",
                "expected_source_fingerprint": source_fingerprint
            }));
            *operation_groups.entry("add_core_exposure").or_default() += 1;
            after_exposure += 1;
        }

        for request in requests {
            affected_agents.insert(request.agent.clone());
            affected_skills.insert(skill_id.to_string());
        }
    }

    if needs_backup_root {
        operations.insert(
            0,
            json!({
                "kind": "create_directory",
                "target": backup_root,
                "expected_fingerprint": "missing"
            }),
        );
        *operation_groups.entry("create_backup_root").or_default() += 1;
    }
    if needs_library_root {
        operations.insert(
            0,
            json!({
                "kind": "create_directory",
                "target": library_root,
                "expected_fingerprint": "missing"
            }),
        );
        *operation_groups.entry("create_library").or_default() += 1;
    }
    ensure_unique_operation_paths(&operations)?;

    for placement in &scan.placements {
        if placement.default_exposed
            && !affected_placements.contains(&placement.id)
            && !changes.iter().any(|change| {
                placement.skill_id == change.skill_id
                    && placement
                        .agent
                        .is_some_and(|agent| agent.id() == change.agent)
            })
        {
            before_exposure += 1;
            after_exposure += 1;
        }
    }

    Ok(DerivedRosterPlan {
        operations,
        implicit_library_changes,
        impact: json!({
            "before_default_exposure": before_exposure,
            "after_default_exposure": after_exposure,
            "exposure_reduction": before_exposure.saturating_sub(after_exposure),
            "exposure_reduction_percent": if before_exposure == 0 { 0.0 } else {
                (before_exposure.saturating_sub(after_exposure) as f64 * 100.0)
                    / before_exposure as f64
            },
            "affected_agents": affected_agents,
            "affected_skill_ids": affected_skills,
            "affected_placement_ids": affected_placements,
            "operation_groups": operation_groups,
            "exclusions": exclusions,
            "blocked_preconditions": [],
            "recovery_state": "clear"
        }),
    })
}

fn ensure_physical_exposure_compatible(
    skill_id: &str,
    placements: &[&SkillPlacement],
    desired: &BTreeMap<AgentKind, RosterState>,
) -> Result<()> {
    let mut groups = BTreeMap::<PathBuf, Vec<&SkillPlacement>>::new();
    for placement in placements
        .iter()
        .copied()
        .filter(|placement| placement.default_exposed)
    {
        groups
            .entry(physical_mutation_path(placement))
            .or_default()
            .push(placement);
    }
    for placements in groups.into_values().filter(|group| group.len() > 1) {
        let has_demotion = placements.iter().any(|placement| {
            placement
                .agent
                .and_then(|agent| desired.get(&agent))
                .is_some_and(|state| state != &RosterState::Core)
        });
        let has_retained_exposure = placements.iter().any(|placement| {
            placement.agent.is_some_and(|agent| {
                desired
                    .get(&agent)
                    .is_none_or(|state| state == &RosterState::Core)
            })
        });
        if has_demotion && has_retained_exposure {
            let agents = placements
                .iter()
                .filter_map(|placement| placement.agent.map(AgentKind::id))
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            return Err(RosterPhysicalConflict {
                skill_id: skill_id.to_owned(),
                agents,
            }
            .into());
        }
    }
    Ok(())
}

fn physical_mutation_path(placement: &SkillPlacement) -> PathBuf {
    if is_symlink(&placement.directory) {
        return physical_entry_path(&placement.directory);
    }
    placement.physical_directory_or_logical().to_path_buf()
}

fn depends_on_physical_removal(placement: &SkillPlacement, removal: &[&SkillPlacement]) -> bool {
    placement.link_target.is_some()
        && removal.iter().any(|removed| {
            let dependency = placement.physical_directory_or_logical();
            let removed = removed.physical_directory_or_logical();
            dependency == removed || dependency.starts_with(removed)
        })
}

fn provider_placements_on_physical_removal<'a>(
    placements: &'a [&SkillPlacement],
    removal: &[&SkillPlacement],
) -> impl Iterator<Item = &'a SkillPlacement> {
    let removal_paths = removal
        .iter()
        .map(|placement| physical_mutation_path(placement))
        .collect::<BTreeSet<_>>();
    placements.iter().copied().filter(move |placement| {
        !placement.governable && removal_paths.contains(&physical_mutation_path(placement))
    })
}

fn ensure_unique_operation_paths(operations: &[Value]) -> Result<()> {
    let mut move_sources = BTreeSet::new();
    let mut targets = BTreeMap::<PathBuf, &str>::new();
    for operation in operations {
        if operation["kind"] == "move_recoverable" {
            let source = operation["source"]
                .as_str()
                .ok_or_else(|| anyhow!("Roster move operation has no source"))?;
            let source = physical_operation_path(Path::new(source));
            if !move_sources.insert(source.clone()) {
                return Err(RosterOperationConflict {
                    identity_role: "source",
                    operation_kinds: vec!["move_recoverable".into(), "move_recoverable".into()],
                    path: source,
                }
                .into());
            }
        }
        let Some(target) = operation["target"].as_str() else {
            continue;
        };
        let target = physical_operation_path(Path::new(target));
        let kind = operation["kind"].as_str().unwrap_or("unknown");
        if let Some(first_kind) = targets.insert(target.clone(), kind) {
            return Err(RosterOperationConflict {
                identity_role: "target",
                operation_kinds: vec![first_kind.into(), kind.into()],
                path: target,
            }
            .into());
        }
    }
    Ok(())
}

fn physical_operation_path(path: &Path) -> PathBuf {
    if is_symlink(path) {
        return physical_entry_path(path);
    }
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved;
    }
    physical_entry_path(path)
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

fn physical_entry_path(path: &Path) -> PathBuf {
    let Some(parent) = path.parent() else {
        return path.to_path_buf();
    };
    let Some(name) = path.file_name() else {
        return path.to_path_buf();
    };
    std::fs::canonicalize(parent)
        .unwrap_or_else(|_| parent.to_path_buf())
        .join(name)
}

fn verified_real_source(
    placements: &[&SkillPlacement],
    excluded: &[&SkillPlacement],
    digest: &str,
) -> Option<PathBuf> {
    let excluded_sources = excluded
        .iter()
        .map(|placement| physical_mutation_path(placement))
        .collect::<BTreeSet<_>>();
    placements
        .iter()
        .copied()
        .filter(|placement| !excluded_sources.contains(&physical_mutation_path(placement)))
        .find(|placement| is_real_exact(placement, digest))
        .map(physical_mutation_path)
}

fn is_real_exact(placement: &SkillPlacement, digest: &str) -> bool {
    placement.governable
        && placement.content_digest == digest
        && placement.link_target.is_none()
        && std::fs::symlink_metadata(&placement.directory)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn agent_root(scan: &ScanResult, agent: AgentKind) -> Result<PathBuf> {
    scan.roots
        .iter()
        .find(|root| {
            root.agent == Some(agent)
                && root.kind == RootKind::Skills
                && root.status == RootStatus::Included
        })
        .map(|root| root.path.clone())
        .ok_or_else(|| anyhow!("Agent {} has no included Skill root", agent.id()))
}

fn agent(value: &str) -> Result<AgentKind> {
    AgentKind::ALL
        .into_iter()
        .find(|agent| agent.id() == value)
        .ok_or_else(|| anyhow!("unsupported Agent in Roster change: {value}"))
}

fn roster_state(value: &str) -> Result<RosterState> {
    match value {
        "core" => Ok(RosterState::Core),
        "on_demand" => Ok(RosterState::OnDemand),
        "explicit_only" => Ok(RosterState::ExplicitOnly),
        "archived" => Ok(RosterState::Archived),
        _ => bail!("unsupported Roster state: {value}"),
    }
}

fn safe_name(name: &str) -> Result<String> {
    let safe = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if safe.trim_matches('-').is_empty() {
        bail!("Skill name cannot form a safe placement directory");
    }
    Ok(safe)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::scan::{ScanOptions, scan};

    #[cfg(unix)]
    fn shared_agent_root_fixture() -> (TempDir, ScanResult, PathBuf) {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let shared_root = home.join(".agents_skills");
        let shared_skill = shared_root.join("shared");
        fs::create_dir_all(&shared_skill).unwrap();
        fs::write(
            shared_skill.join("SKILL.md"),
            "---\nname: shared\n---\nfixture\n",
        )
        .unwrap();
        for (parent, logical_root) in [
            (home.join(".codex"), home.join(".codex/skills")),
            (home.join(".claude"), home.join(".claude/skills")),
            (home.join(".pi/agent"), home.join(".pi/agent/skills")),
        ] {
            fs::create_dir_all(parent).unwrap();
            symlink(&shared_root, logical_root).unwrap();
        }
        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let snapshot = scan(&options).unwrap();
        assert_eq!(
            snapshot
                .placements
                .iter()
                .filter(|placement| placement.default_exposed)
                .count(),
            3
        );
        (temp, snapshot, shared_skill)
    }

    #[cfg(unix)]
    #[test]
    fn shared_physical_demotion_emits_one_recoverable_move() {
        let (temp, snapshot, shared_skill) = shared_agent_root_fixture();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let skill_id = snapshot.skills[0].id.clone();
        let changes = ["codex", "claude-code", "pi"]
            .into_iter()
            .map(|agent| RosterChange {
                agent: agent.into(),
                skill_id: skill_id.clone(),
                state: "on_demand".into(),
            })
            .collect::<Vec<_>>();

        let plan = derive(&snapshot, &state, &changes).unwrap();
        let moves = plan
            .operations
            .iter()
            .filter(|operation| operation["kind"] == "move_recoverable")
            .collect::<Vec<_>>();

        assert_eq!(moves.len(), 1);
        assert_eq!(
            moves[0]["source"],
            json!(fs::canonicalize(shared_skill).unwrap())
        );
        assert_eq!(plan.impact["before_default_exposure"], 3);
        assert_eq!(plan.impact["after_default_exposure"], 0);
        assert_eq!(
            plan.impact["affected_placement_ids"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[cfg(unix)]
    #[test]
    fn shared_physical_core_and_demotion_fail_closed() {
        let (temp, snapshot, shared_skill) = shared_agent_root_fixture();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let skill_id = snapshot.skills[0].id.clone();

        let error = match derive(
            &snapshot,
            &state,
            &[
                RosterChange {
                    agent: "codex".into(),
                    skill_id: skill_id.clone(),
                    state: "core".into(),
                },
                RosterChange {
                    agent: "claude-code".into(),
                    skill_id,
                    state: "on_demand".into(),
                },
            ],
        ) {
            Ok(_) => panic!("conflicting shared physical states unexpectedly produced a Plan"),
            Err(error) => error,
        };

        assert!(error.downcast_ref::<RosterPhysicalConflict>().is_some());
        assert!(shared_skill.join("SKILL.md").is_file());
        assert!(fs::read_dir(&state).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn shared_physical_root_drift_requires_a_new_scan() {
        use std::os::unix::fs::symlink;

        let (temp, snapshot, shared_skill) = shared_agent_root_fixture();
        let home = shared_skill.parent().unwrap().parent().unwrap();
        let replacement_root = home.join(".replacement_skills");
        let replacement_skill = replacement_root.join("shared");
        fs::create_dir_all(&replacement_skill).unwrap();
        fs::write(
            replacement_skill.join("SKILL.md"),
            "---\nname: shared\n---\nreplacement\n",
        )
        .unwrap();
        for root in [
            home.join(".codex/skills"),
            home.join(".claude/skills"),
            home.join(".pi/agent/skills"),
        ] {
            fs::remove_file(&root).unwrap();
            symlink(&replacement_root, root).unwrap();
        }
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let skill_id = snapshot.skills[0].id.clone();
        let changes = ["codex", "claude-code", "pi"]
            .into_iter()
            .map(|agent| RosterChange {
                agent: agent.into(),
                skill_id: skill_id.clone(),
                state: "on_demand".into(),
            })
            .collect::<Vec<_>>();

        let error = derive(&snapshot, &state, &changes).unwrap_err();

        assert!(error.to_string().contains("physical source drifted"));
        assert!(error.to_string().contains("run skillroster scan"));
        assert!(shared_skill.join("SKILL.md").is_file());
        assert!(replacement_skill.join("SKILL.md").is_file());
        assert!(fs::read_dir(&state).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn shared_physical_demotion_blocks_a_dependent_source_link() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let shared_root = home.join(".agents_skills");
        let shared_skill = shared_root.join("shared");
        fs::create_dir_all(&shared_skill).unwrap();
        fs::write(
            shared_skill.join("SKILL.md"),
            "---\nname: shared\n---\nfixture\n",
        )
        .unwrap();
        for (parent, root) in [
            (home.join(".codex"), home.join(".codex/skills")),
            (home.join(".claude"), home.join(".claude/skills")),
            (home.join(".pi/agent"), home.join(".pi/agent/skills")),
        ] {
            fs::create_dir_all(parent).unwrap();
            symlink(&shared_root, root).unwrap();
        }
        let source_root = temp.path().join("sources");
        fs::create_dir(&source_root).unwrap();
        symlink(&shared_skill, source_root.join("shared")).unwrap();
        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        options.explicit_source_roots.push(source_root.clone());
        let snapshot = scan(&options).unwrap();
        let skill_id = snapshot.skills[0].id.clone();
        let changes = ["codex", "claude-code", "pi"]
            .into_iter()
            .map(|agent| RosterChange {
                agent: agent.into(),
                skill_id: skill_id.clone(),
                state: "on_demand".into(),
            })
            .collect::<Vec<_>>();
        let supported = exclude_unpreservable_demotions(&snapshot, changes.clone()).unwrap();
        assert!(supported.changes.is_empty());
        assert!(
            supported
                .exclusions
                .iter()
                .all(|item| item.reason == "non_agent_source_link_depends_on_removal")
        );
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();

        let error = derive(&snapshot, &state, &changes).unwrap_err();

        assert!(error.to_string().contains("non-Agent source link"));
        assert!(shared_skill.join("SKILL.md").is_file());
        assert!(source_root.join("shared/SKILL.md").is_file());
        assert!(fs::read_dir(&state).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn mixed_provider_managed_shared_placement_is_read_only() {
        let (temp, mut snapshot, shared_skill) = shared_agent_root_fixture();
        let mut provider = snapshot
            .placements
            .iter()
            .find(|placement| placement.agent == Some(AgentKind::Codex))
            .unwrap()
            .clone();
        provider.id = "placement_provider_fixture".into();
        provider.agent = None;
        provider.governable = false;
        provider.provider = Some("fixture-provider".into());
        snapshot.placements.push(provider);
        let skill_id = snapshot.skills[0].id.clone();
        let changes = ["codex", "claude-code", "pi"]
            .into_iter()
            .map(|agent| RosterChange {
                agent: agent.into(),
                skill_id: skill_id.clone(),
                state: "on_demand".into(),
            })
            .collect::<Vec<_>>();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();

        let error = derive(&snapshot, &state, &changes).unwrap_err();

        assert!(error.to_string().contains("provider-managed placement"));
        assert!(shared_skill.join("SKILL.md").is_file());
        assert!(fs::read_dir(&state).unwrap().next().is_none());
    }

    #[test]
    fn same_name_variants_cannot_share_one_library_target() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        for (root, body) in [
            (home.join(".codex/skills/shared"), "codex variant"),
            (home.join(".hermes/skills/shared"), "hermes variant"),
        ] {
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join("SKILL.md"),
                format!("---\nname: shared\n---\n{body}\n"),
            )
            .unwrap();
        }
        fs::create_dir(&state).unwrap();
        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let snapshot = scan(&options).unwrap();
        let changes = snapshot
            .placements
            .iter()
            .filter_map(|placement| {
                placement.agent.map(|agent| RosterChange {
                    agent: agent.id().into(),
                    skill_id: placement.skill_id.clone(),
                    state: "on_demand".into(),
                })
            })
            .collect::<Vec<_>>();

        let error = match derive(&snapshot, &state, &changes) {
            Ok(_) => panic!("same-name variants unexpectedly shared one Library target"),
            Err(error) => error,
        };

        assert!(error.downcast_ref::<RosterOperationConflict>().is_some());
        assert!(error.to_string().contains("same-name variant Finding"));
        assert!(fs::read_dir(&state).unwrap().next().is_none());
    }

    #[test]
    fn large_roster_proposal_removes_more_than_half_of_default_exposure() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let root = home.join(".codex/skills");
        fs::create_dir_all(&root).unwrap();
        for index in 0..120 {
            let directory = root.join(format!("skill-{index:03}"));
            fs::create_dir(&directory).unwrap();
            fs::write(
                directory.join("SKILL.md"),
                format!("---\nname: skill-{index:03}\n---\nfixture\n"),
            )
            .unwrap();
        }
        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let snapshot = scan(&options).unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let changes = snapshot
            .skills
            .iter()
            .map(|skill| RosterChange {
                agent: "codex".into(),
                skill_id: skill.id.clone(),
                state: "on_demand".into(),
            })
            .collect::<Vec<_>>();
        let plan = derive(&snapshot, &state, &changes).unwrap();
        assert_eq!(plan.impact["before_default_exposure"], 120);
        assert_eq!(plan.impact["after_default_exposure"], 0);
        assert!(plan.impact["exposure_reduction_percent"].as_f64().unwrap() >= 50.0);
        assert_eq!(plan.implicit_library_changes.len(), 120);
    }

    #[cfg(unix)]
    #[test]
    fn semantic_bulk_changes_exclude_an_unowned_escaping_link() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let root = home.join(".codex/skills");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            outside.join("SKILL.md"),
            "---\nname: external\n---\nfixture\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, root.join("external")).unwrap();
        let snapshot = scan(&ScanOptions::for_home(&home)).unwrap();
        let skill_id = snapshot.placements[0].skill_id.clone();

        let supported = exclude_unpreservable_demotions(
            &snapshot,
            vec![RosterChange {
                agent: "codex".into(),
                skill_id: skill_id.clone(),
                state: "on_demand".into(),
            }],
        )
        .unwrap();

        assert!(supported.changes.is_empty());
        assert_eq!(supported.exclusions.len(), 1);
        assert_eq!(supported.exclusions[0].skill_id, skill_id);
        assert_eq!(supported.exclusions[0].name, "external");
        assert_eq!(
            supported.exclusions[0].reason,
            "no_owned_exact_content_to_preserve"
        );
        assert_eq!(
            supported.exclusions[0].observed_source_target.as_deref(),
            Some(outside.as_path())
        );
    }

    #[cfg(unix)]
    #[test]
    fn retained_escaping_link_without_mutation_does_not_require_physical_validation() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let root = home.join(".codex/skills");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            outside.join("SKILL.md"),
            "---\nname: external\n---\nfixture\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, root.join("external")).unwrap();
        let snapshot = scan(&ScanOptions::for_home(&home)).unwrap();
        assert!(snapshot.placements[0].physical_directory.is_none());
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();

        let plan = derive(
            &snapshot,
            &state,
            &[RosterChange {
                agent: "codex".into(),
                skill_id: snapshot.skills[0].id.clone(),
                state: "core".into(),
            }],
        )
        .unwrap();

        assert!(plan.operations.is_empty());
        assert!(fs::read_dir(&state).unwrap().next().is_none());
        assert!(outside.join("SKILL.md").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn semantic_bulk_changes_exclude_a_non_agent_link_to_a_removed_placement() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let codex = home.join(".codex/skills/shared");
        let source_root = temp.path().join("sources");
        let dependent_source = source_root.join("shared");
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir_all(&source_root).unwrap();
        fs::write(codex.join("SKILL.md"), "---\nname: shared\n---\nfixture\n").unwrap();
        std::os::unix::fs::symlink(&codex, &dependent_source).unwrap();
        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        options.explicit_source_roots.push(source_root);
        let snapshot = scan(&options).unwrap();
        let skill_id = snapshot
            .placements
            .iter()
            .find(|placement| placement.agent == Some(AgentKind::Codex))
            .unwrap()
            .skill_id
            .clone();

        let supported = exclude_unpreservable_demotions(
            &snapshot,
            vec![RosterChange {
                agent: "codex".into(),
                skill_id: skill_id.clone(),
                state: "on_demand".into(),
            }],
        )
        .unwrap();

        assert!(supported.changes.is_empty());
        assert_eq!(supported.exclusions.len(), 1);
        assert_eq!(supported.exclusions[0].skill_id, skill_id);
        assert_eq!(
            supported.exclusions[0].reason,
            "non_agent_source_link_depends_on_removal"
        );
        assert!(supported.exclusions[0].observed_source_target.is_none());
    }

    #[test]
    fn core_request_derives_link_from_verified_canonical_content() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let codex_root = home.join(".codex/skills");
        let canonical = home.join(".claude/skills/shared");
        fs::create_dir_all(&codex_root).unwrap();
        fs::create_dir_all(&canonical).unwrap();
        fs::write(canonical.join("SKILL.md"), "---\nname: shared\n---\nbody\n").unwrap();
        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let snapshot = scan(&options).unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let plan = derive(
            &snapshot,
            &state,
            &[RosterChange {
                agent: "codex".into(),
                skill_id: snapshot.skills[0].id.clone(),
                state: "core".into(),
            }],
        )
        .unwrap();
        assert!(plan.operations.iter().any(|operation| {
            operation["kind"] == "create_symlink"
                && operation["target"] == codex_root.join("shared").to_string_lossy().as_ref()
        }));
    }

    #[test]
    fn core_request_refuses_a_provider_only_skill_source() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let codex_root = home.join(".codex/skills");
        let plugin_skill = home
            .join(".codex/plugins/cache/openai-bundled/browser/1.0.0/skills")
            .join("control-browser");
        fs::create_dir_all(&codex_root).unwrap();
        fs::create_dir_all(&plugin_skill).unwrap();
        fs::write(
            plugin_skill.join("SKILL.md"),
            "---\nname: control-browser\n---\nprovider body\n",
        )
        .unwrap();
        fs::write(
            home.join(".codex/config.toml"),
            "[plugins.\"browser@openai-bundled\"]\nenabled = true\n",
        )
        .unwrap();
        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let snapshot = scan(&options).unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();

        let error = match derive(
            &snapshot,
            &state,
            &[RosterChange {
                agent: "codex".into(),
                skill_id: snapshot.skills[0].id.clone(),
                state: "core".into(),
            }],
        ) {
            Ok(_) => panic!("provider-only Skill unexpectedly produced a Roster Plan"),
            Err(error) => error,
        };

        let blocker = error.downcast_ref::<RosterSafetyBlocker>().unwrap();
        assert!(matches!(
            blocker,
            RosterSafetyBlocker::ProviderManaged { .. }
        ));
        assert!(!codex_root.join("control-browser").exists());
    }

    #[cfg(unix)]
    #[test]
    fn out_of_scope_agent_link_is_retargeted_when_its_canonical_directory_moves() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let codex = home.join(".codex/skills/shared");
        let claude_root = home.join(".claude/skills");
        let claude = claude_root.join("shared");
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir_all(&claude_root).unwrap();
        fs::write(codex.join("SKILL.md"), "---\nname: shared\n---\nfixture\n").unwrap();
        std::os::unix::fs::symlink(&codex, &claude).unwrap();
        let snapshot = scan(&ScanOptions::for_home(&home)).unwrap();
        let skill_id = snapshot.skills[0].id.clone();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let state = fs::canonicalize(state).unwrap();

        let plan = derive(
            &snapshot,
            &state,
            &[RosterChange {
                agent: "codex".into(),
                skill_id,
                state: "on_demand".into(),
            }],
        )
        .unwrap();

        assert!(plan.operations.iter().any(|operation| {
            operation["kind"] == "move_recoverable" && operation["source"] == json!(claude)
        }));
        assert_eq!(plan.impact["before_default_exposure"], 2);
        assert_eq!(plan.impact["after_default_exposure"], 1);

        let mut approved_roots = snapshot
            .roots
            .iter()
            .filter(|root| root.kind == RootKind::Skills && root.status == RootStatus::Included)
            .map(|root| root.path.clone())
            .collect::<Vec<_>>();
        approved_roots.push(state.join("library"));
        approved_roots.push(state.join("plan-backups"));
        let prepared = change::prepare(
            &json!({
                "schema_version": 1,
                "scan_id": "scan_fixture",
                "evidence_ids": ["evidence_fixture"],
                "operations": plan.operations,
                "roster_changes": [{
                    "agent": "codex",
                    "skill_id": snapshot.skills[0].id,
                    "state": "on_demand"
                }],
                "library_changes": plan.implicit_library_changes
            })
            .to_string(),
            &change::PrepareContext {
                approved_roots,
                state_dir: state.clone(),
                operation_policy: change::OperationPolicy::LibraryGovernance,
            },
        )
        .unwrap();
        let applied = change::apply(&prepared).unwrap();
        assert!(applied.verification_passed);
        assert!(!codex.exists());
        assert_eq!(
            fs::canonicalize(&claude).unwrap(),
            fs::canonicalize(state.join("library/shared")).unwrap()
        );

        let undone = change::undo(&applied.receipt).unwrap();
        assert!(undone.verification_passed);
        assert!(codex.join("SKILL.md").is_file());
        assert_eq!(
            fs::canonicalize(&claude).unwrap(),
            fs::canonicalize(codex).unwrap()
        );
        assert!(plan.operations.iter().any(|operation| {
            operation["kind"] == "create_symlink"
                && operation["target"] == json!(claude)
                && operation["source"] == json!(state.join("library/shared"))
        }));
    }

    #[test]
    fn reviewed_source_roots_keep_siblings_exact_and_only_dedupe_observed_ancestors() {
        let shared = test_absolute_path("opt/reviewed/alpha");
        let sibling = test_absolute_path("opt/reviewed/beta");
        let unique = test_absolute_path("elsewhere/one-off");
        let observed_ancestor = test_absolute_path("explicit/root");
        let observed_descendant = observed_ancestor.join("nested");
        let roots = minimum_reviewed_source_roots([
            shared.clone(),
            sibling.clone(),
            unique.clone(),
            observed_ancestor.clone(),
            observed_descendant,
        ]);
        assert_eq!(roots, vec![unique, observed_ancestor, shared, sibling]);
    }

    #[test]
    fn source_confirmation_block_is_typed_bounded_and_actionable() {
        let state = TempDir::new().unwrap();
        let exclusions = vec![
            RosterChangeExclusion {
                agent: "codex".into(),
                skill_id: "skill_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                name: "alpha".into(),
                reason: "no_owned_exact_content_to_preserve",
                observed_source_target: Some(test_absolute_path("opt/reviewed/alpha")),
            },
            RosterChangeExclusion {
                agent: "codex".into(),
                skill_id: "skill_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                name: "beta".into(),
                reason: "no_owned_exact_content_to_preserve",
                observed_source_target: Some(test_absolute_path("opt/reviewed/beta")),
            },
        ];
        let blocked =
            source_confirmation_block("finding_fixture", 10, &exclusions, state.path()).unwrap();
        let reviewed = [
            test_absolute_path("opt/reviewed/alpha"),
            test_absolute_path("opt/reviewed/beta"),
        ];
        assert_eq!(
            blocked.paths,
            reviewed
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
        );
        assert!(blocked.relevant_ids.contains(&"finding_fixture".into()));
        assert_eq!(blocked.details["decision"], "confirm_trusted_source_roots");
        assert_eq!(blocked.details["requested_core_budget"], 10);
        assert_eq!(blocked.details["blocked_change_count"], 2);
        assert_eq!(blocked.details["blocked_changes_truncated"], false);
        assert_eq!(blocked.details["source_roots"], json!(reviewed));
        assert_eq!(blocked.details["blocked_changes"][0]["name"], "alpha");
        assert_eq!(
            blocked.details["blocked_changes"][0]["observed_source_target"],
            json!(test_absolute_path("opt/reviewed/alpha"))
        );
        assert_eq!(blocked.details["files_changed"], false);
        assert!(blocked.details.get("detail").is_none());
        assert!(!state.path().join("source-confirmation").exists());
        assert!(!blocked.message.contains("session"));
    }

    #[test]
    fn source_confirmation_block_writes_omitted_identities_to_a_detail_file() {
        let state = TempDir::new().unwrap();
        let exclusions = (0..11)
            .map(|index| RosterChangeExclusion {
                agent: "codex".into(),
                skill_id: format!("skill_{index:032}"),
                name: format!("skill-{index:02}"),
                reason: "no_owned_exact_content_to_preserve",
                observed_source_target: Some(test_absolute_path(&format!(
                    "opt/root-{index:02}/pkg"
                ))),
            })
            .collect::<Vec<_>>();
        let blocked =
            source_confirmation_block("finding_fixture", 10, &exclusions, state.path()).unwrap();
        let changes = blocked.details["blocked_changes"].as_array().unwrap();
        assert_eq!(changes.len(), 10);
        assert_eq!(blocked.details["blocked_change_count"], 11);
        assert_eq!(blocked.details["blocked_changes_truncated"], true);
        assert_eq!(blocked.details["source_roots_truncated"], true);
        assert_eq!(blocked.details["state_files_changed"], true);
        assert_eq!(blocked.details["detail_artifact_created"], true);
        assert_eq!(blocked.relevant_ids.len(), 11);
        let expected_roots = (0..11)
            .map(|index| test_absolute_path(&format!("opt/root-{index:02}/pkg")))
            .collect::<Vec<_>>();
        let bounded_roots = expected_roots.iter().take(10).cloned().collect::<Vec<_>>();
        assert_eq!(
            blocked.paths,
            bounded_roots
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(blocked.details["source_roots"], json!(bounded_roots));
        assert_eq!(
            blocked.details["after_confirmation"]["source_roots"],
            json!(bounded_roots)
        );
        let detail_path = blocked.details["detail"]["path"].as_str().unwrap();
        let detail_name = Path::new(detail_path)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap();
        assert!(ulid::Ulid::from_string(detail_name).is_ok());
        assert_eq!(
            fs::read_dir(state.path().join("source-confirmation"))
                .unwrap()
                .count(),
            1,
            "atomic publication must not leave a temporary artifact"
        );
        let complete: Value = serde_json::from_slice(&fs::read(detail_path).unwrap()).unwrap();
        assert_eq!(complete["schema_version"], 1);
        assert_eq!(complete["blocked_changes"].as_array().unwrap().len(), 11);
        assert_eq!(complete["source_roots"], json!(expected_roots));
        assert_eq!(
            complete["after_confirmation"]["source_roots"],
            json!(expected_roots)
        );
        let argv = complete["after_confirmation"]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        for root in &expected_roots {
            let root = root.display().to_string();
            assert!(
                argv.windows(2)
                    .any(|pair| pair[0] == "--source-root" && pair[1] == root),
                "missing --source-root {root} in {argv:?}"
            );
        }
        for exclusion in &exclusions {
            assert!(
                complete["blocked_changes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| {
                        item["skill_id"] == exclusion.skill_id && item["name"] == exclusion.name
                    }),
                "missing {}",
                exclusion.skill_id
            );
        }
        assert!(!changes.iter().any(|item| item["name"] == "skill-10"));
    }

    #[cfg(unix)]
    #[test]
    fn source_confirmation_block_refuses_a_symlinked_detail_directory() {
        let state = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), state.path().join("source-confirmation"))
            .unwrap();
        let exclusions = (0..11)
            .map(|index| RosterChangeExclusion {
                agent: "codex".into(),
                skill_id: format!("skill_{index:032}"),
                name: format!("skill-{index:02}"),
                reason: "no_owned_exact_content_to_preserve",
                observed_source_target: Some(test_absolute_path(&format!(
                    "opt/root-{index:02}/pkg"
                ))),
            })
            .collect::<Vec<_>>();

        assert!(
            source_confirmation_block("finding_fixture", 10, &exclusions, state.path()).is_err()
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn reviewed_source_roots_drop_filesystem_roots() {
        let keep = PathBuf::from("/opt/reviewed/alpha");
        assert_eq!(
            minimum_reviewed_source_roots([PathBuf::from("/"), keep.clone()]),
            vec![keep]
        );
        assert!(minimum_reviewed_source_roots([PathBuf::from("/")]).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn reviewed_source_roots_drop_windows_prefix_roots() {
        let drive = PathBuf::from(r"C:\");
        let keep = PathBuf::from(r"C:\reviewed\alpha");
        assert_eq!(
            minimum_reviewed_source_roots([drive.clone(), keep.clone()]),
            vec![keep]
        );
        assert!(minimum_reviewed_source_roots([drive]).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn source_confirmation_block_omits_filesystem_root_guidance() {
        let exclusions = vec![RosterChangeExclusion {
            agent: "codex".into(),
            skill_id: "skill_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            name: "rootish".into(),
            reason: "no_owned_exact_content_to_preserve",
            observed_source_target: Some(PathBuf::from("/")),
        }];
        let state = TempDir::new().unwrap();
        let blocked =
            source_confirmation_block("finding_fixture", 10, &exclusions, state.path()).unwrap();
        assert!(blocked.paths.is_empty());
        assert_eq!(blocked.details["source_roots"], json!([]));
        assert_eq!(blocked.details["blocked_changes"][0]["name"], "rootish");
        assert_eq!(
            blocked.details["blocked_changes"].as_array().unwrap().len(),
            1
        );
    }
}
