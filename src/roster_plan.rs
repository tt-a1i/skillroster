use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::change::{self, LibraryChangeAction, RosterChange};
use crate::harness::AgentKind;
use crate::scan::{LinkStatus, RootKind, RootStatus, ScanResult, SkillPlacement};

pub struct DerivedRosterPlan {
    pub operations: Vec<Value>,
    pub implicit_library_changes: Vec<LibraryChangeAction>,
    pub impact: Value,
}

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

const JSON_BLOCKER_LIMIT: usize = 10;

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
) -> RosterPlanBlocked {
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
    let bounded_roots = source_root_paths
        .iter()
        .take(JSON_BLOCKER_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    let blocked_changes = exclusions
        .iter()
        .take(JSON_BLOCKER_LIMIT)
        .map(|exclusion| {
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
        })
        .collect::<Vec<_>>();
    let mut relevant_ids = vec![finding_id.to_string()];
    relevant_ids.extend(
        exclusions
            .iter()
            .map(|exclusion| exclusion.skill_id.clone())
            .take(JSON_BLOCKER_LIMIT),
    );
    let blocked_change_count = exclusions.len();
    RosterPlanBlocked {
        message: format!(
            "Finding {finding_id} is blocked by {blocked_change_count} Roster changes without owned exact content at core_budget {core_budget}; confirm the reported source roots, rescan, and use the new Finding"
        ),
        relevant_ids,
        paths: bounded_roots.clone(),
        details: json!({
            "reason": "trusted_canonical_sources_required",
            "decision": "confirm_trusted_source_roots",
            "automatic_change_supported": false,
            "requested_core_budget": core_budget,
            "blocked_change_count": blocked_change_count,
            "blocked_changes": blocked_changes,
            "blocked_changes_truncated": blocked_change_count > JSON_BLOCKER_LIMIT,
            "source_root_count": source_root_paths.len(),
            "source_roots": bounded_roots,
            "source_roots_truncated": source_root_paths.len() > JSON_BLOCKER_LIMIT,
            "after_confirmation": {
                "repeatable_option": "--source-root",
                "source_roots": bounded_roots,
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
            "detail": {
                "mode": if blocked_change_count > JSON_BLOCKER_LIMIT
                    || source_root_paths.len() > JSON_BLOCKER_LIMIT
                {
                    "summary"
                } else {
                    "complete"
                },
                "contains": ["blocked_changes", "source_roots"]
            }
        }),
    }
}

/// Collapse observed escaping-link targets to the fewest `--source-root` directories.
pub fn minimum_reviewed_source_roots(targets: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let unique = targets
        .into_iter()
        .filter(|path| path.is_absolute())
        .collect::<BTreeSet<_>>();
    let mut by_parent = BTreeMap::<PathBuf, BTreeSet<PathBuf>>::new();
    for target in unique {
        match target.parent() {
            Some(parent) if parent.parent().is_some() => {
                by_parent
                    .entry(parent.to_path_buf())
                    .or_default()
                    .insert(target);
            }
            _ => {
                by_parent.entry(target.clone()).or_default().insert(target);
            }
        }
    }
    let mut roots = by_parent
        .into_iter()
        .map(|(parent, members)| {
            if members.len() > 1 {
                parent
            } else {
                members.into_iter().next().expect("group is non-empty")
            }
        })
        .collect::<Vec<_>>();
    roots.sort();
    let mut kept = Vec::new();
    for root in roots {
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
            .any(|placement| {
                placement.link_target.as_ref().is_some_and(|target| {
                    removable.iter().any(|removed| {
                        target == &removed.directory || target.starts_with(&removed.directory)
                    })
                })
            });
        let reason = if non_agent_source_dependency {
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
            bail!("Skill {skill_id} is provider-managed and read-only");
        }
        let desired = requests
            .iter()
            .map(|request| Ok((agent(&request.agent)?, request.state.as_str())))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let removal = placements
            .iter()
            .copied()
            .filter(|placement| {
                placement
                    .agent
                    .and_then(|agent| desired.get(&agent))
                    .is_some_and(|state| *state != "core")
            })
            .collect::<Vec<_>>();
        let mut source = verified_real_source(&placements, &removal, &skill.content_digest);
        let mut source_fingerprint = source
            .as_ref()
            .map(|path| change::fingerprint(path))
            .transpose()?;
        let mut migrated_id = None;

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
            let canonical_fingerprint = change::fingerprint(&canonical.directory)?;
            operations.push(json!({
                "kind": "move_recoverable",
                "source": canonical.directory,
                "target": library_path,
                "expected_fingerprint": canonical_fingerprint
            }));
            *operation_groups.entry("host_canonical").or_default() += 1;
            source = Some(library_path.clone());
            source_fingerprint = Some(canonical_fingerprint);
            migrated_id = Some(canonical.id.as_str());
            implicit_library_changes.push(LibraryChangeAction {
                skill_id: skill_id.to_string(),
                canonical_placement_id: canonical.id.clone(),
                placement_ids: placements
                    .iter()
                    .map(|placement| placement.id.clone())
                    .collect(),
                requested_state: "hosted".into(),
                canonical_path: canonical.directory.clone(),
                library_path: Some(library_path),
            });
        }

        let dependent_links = placements
            .iter()
            .copied()
            .filter(|placement| !removal.iter().any(|removed| removed.id == placement.id))
            .filter(|placement| {
                placement.link_target.as_ref().is_some_and(|target| {
                    removal.iter().any(|removed| {
                        target == &removed.directory || target.starts_with(&removed.directory)
                    })
                })
            })
            .collect::<Vec<_>>();
        if dependent_links
            .iter()
            .any(|placement| placement.agent.is_none())
        {
            bail!(
                "Skill {skill_id} has a non-Agent source link that depends on a placement scheduled for removal"
            );
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

        for placement in &removal {
            before_exposure += usize::from(placement.default_exposed);
            affected_placements.insert(placement.id.clone());
            if migrated_id == Some(placement.id.as_str()) {
                continue;
            }
            needs_backup_root |= !backup_root.exists();
            let backup = backup_root.join(format!("{nonce}-{}", placement.id));
            operations.push(json!({
                "kind": "move_recoverable",
                "source": placement.directory,
                "target": backup,
                "expected_fingerprint": change::fingerprint(&placement.directory)?
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

fn verified_real_source(
    placements: &[&SkillPlacement],
    excluded: &[&SkillPlacement],
    digest: &str,
) -> Option<PathBuf> {
    placements
        .iter()
        .copied()
        .filter(|placement| !excluded.iter().any(|item| item.id == placement.id))
        .find(|placement| is_real_exact(placement, digest))
        .map(|placement| placement.directory.clone())
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

        assert!(error.to_string().contains("provider-managed and read-only"));
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
    fn reviewed_source_roots_collapse_sibling_packages_and_keep_singletons() {
        let shared = PathBuf::from("/opt/reviewed/alpha");
        let sibling = PathBuf::from("/opt/reviewed/beta");
        let unique = PathBuf::from("/elsewhere/one-off");
        let roots = minimum_reviewed_source_roots([shared, sibling, unique.clone()]);
        assert_eq!(roots, vec![unique, PathBuf::from("/opt/reviewed")]);
    }

    #[test]
    fn source_confirmation_block_is_typed_bounded_and_actionable() {
        let exclusions = vec![
            RosterChangeExclusion {
                agent: "codex".into(),
                skill_id: "skill_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                name: "alpha".into(),
                reason: "no_owned_exact_content_to_preserve",
                observed_source_target: Some(PathBuf::from("/opt/reviewed/alpha")),
            },
            RosterChangeExclusion {
                agent: "codex".into(),
                skill_id: "skill_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                name: "beta".into(),
                reason: "no_owned_exact_content_to_preserve",
                observed_source_target: Some(PathBuf::from("/opt/reviewed/beta")),
            },
        ];
        let blocked = source_confirmation_block("finding_fixture", 10, &exclusions);
        assert_eq!(blocked.paths, vec!["/opt/reviewed"]);
        assert!(blocked.relevant_ids.contains(&"finding_fixture".into()));
        assert_eq!(blocked.details["decision"], "confirm_trusted_source_roots");
        assert_eq!(blocked.details["requested_core_budget"], 10);
        assert_eq!(blocked.details["blocked_change_count"], 2);
        assert_eq!(blocked.details["blocked_changes_truncated"], false);
        assert_eq!(blocked.details["source_roots"], json!(["/opt/reviewed"]));
        assert_eq!(blocked.details["blocked_changes"][0]["name"], "alpha");
        assert_eq!(
            blocked.details["blocked_changes"][0]["observed_source_target"],
            "/opt/reviewed/alpha"
        );
        assert_eq!(blocked.details["files_changed"], false);
        assert_eq!(blocked.details["detail"]["mode"], "complete");
        assert!(!blocked.message.contains("session"));
    }
}
