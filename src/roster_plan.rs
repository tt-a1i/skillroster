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
                .filter(|placement| placement.default_exposed)
                .count();
            if existing.iter().any(|placement| {
                placement.default_exposed
                    && placement.link_status != LinkStatus::Broken
                    && placement.content_digest == skill.content_digest
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
    placement.content_digest == digest
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
}
