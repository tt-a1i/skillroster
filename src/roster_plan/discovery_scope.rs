use super::*;
use crate::scan::{RootObservation, SkillDiscoveryGap, SkillDiscoveryGapReason};

fn blocked(root: &RootObservation) -> anyhow::Error {
    RosterDiscoveryIncomplete {
        agent: root.agent.map(AgentKind::id).unwrap_or("shared").into(),
        path: root.path.clone(),
        detail: root.detail.clone(),
    }
    .into()
}

pub(super) fn preserved_gaps<'a>(
    scan: &'a ScanResult,
    root: &RootObservation,
) -> Result<&'a [SkillDiscoveryGap]> {
    let gaps = scan
        .skill_discovery_gaps
        .get(&root.path)
        .filter(|gaps| !gaps.is_empty())
        .ok_or_else(|| blocked(root))?;
    for gap in gaps {
        if !matches!(
            gap.reason,
            SkillDiscoveryGapReason::UnapprovedBranch
                | SkillDiscoveryGapReason::UnreadableBranch
                | SkillDiscoveryGapReason::CyclicBranch
        ) || gap.path == root.path
            || !gap.path.starts_with(&root.path)
            || gap.physical_path.as_ref().is_none_or(|physical| {
                !physical.is_absolute()
                    || std::fs::canonicalize(&gap.path).ok().as_ref() != Some(physical)
            })
        {
            return Err(blocked(root));
        }
    }
    Ok(gaps)
}

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

pub(super) fn validate_operations(scan: &ScanResult, operations: &[OperationInput]) -> Result<()> {
    let paths = operations
        .iter()
        .flat_map(|operation| match operation {
            OperationInput::CreateSymlink { source, target, .. }
            | OperationInput::Copy { source, target, .. }
            | OperationInput::MoveRecoverable { source, target, .. } => {
                vec![source.as_path(), target.as_path()]
            }
            OperationInput::CreateDirectory { target, .. }
            | OperationInput::WriteFile { target, .. }
            | OperationInput::ReplaceFile { target, .. }
            | OperationInput::RemoveSymlink { target, .. } => vec![target.as_path()],
        })
        .collect::<Vec<_>>();
    validate_paths(scan, paths)
}

pub(super) fn validate_paths(scan: &ScanResult, paths: Vec<&Path>) -> Result<()> {
    let paths = paths
        .into_iter()
        .flat_map(|path| {
            let mut aliases = vec![path.to_path_buf(), physical_operation_path(path)];
            if let Ok(physical) = std::fs::canonicalize(path) {
                aliases.push(physical);
            }
            aliases
        })
        .collect::<Vec<_>>();
    for placement in scan.placements.iter().filter(|placement| {
        scan.placement_native_visibility(placement)
            == Some(&crate::claude_visibility::CatalogVisibility::Unknown)
    }) {
        let resolved = std::fs::canonicalize(&placement.directory).ok();
        if paths.iter().any(|path| {
            overlaps(path, &placement.directory)
                || resolved
                    .as_ref()
                    .is_some_and(|physical| overlaps(path, physical))
                || placement
                    .physical_directory
                    .as_ref()
                    .is_some_and(|physical| overlaps(path, physical))
        }) {
            return Err(NativeVisibilityConflict {
                agent: placement
                    .agent
                    .expect("native visibility belongs to an Agent"),
                skill_id: placement.skill_id.clone(),
                mode: crate::claude_visibility::CatalogVisibility::Unknown,
            }
            .into());
        }
    }
    for root in scan.roots.iter().filter(|root| {
        root.kind == RootKind::Skills
            && root.status == RootStatus::Included
            && !root.discovery_complete
    }) {
        let gaps = match preserved_gaps(scan, root) {
            Ok(gaps) => gaps,
            Err(error) => {
                let physical_root = physical_operation_path(&root.path);
                if paths
                    .iter()
                    .any(|path| overlaps(path, &root.path) || overlaps(path, &physical_root))
                {
                    return Err(error);
                }
                continue;
            }
        };
        for gap in gaps {
            if paths.iter().any(|path| {
                overlaps(path, &gap.path)
                    || gap
                        .physical_path
                        .as_ref()
                        .is_some_and(|physical| overlaps(path, physical))
            }) {
                return Err(blocked(root));
            }
        }
        for placement in scan
            .placements
            .iter()
            .filter(|placement| placement.root == root.path)
        {
            if placement.fingerprint_completeness != FingerprintCompleteness::Complete
                && paths.iter().any(|path| {
                    overlaps(path, &placement.directory)
                        || placement
                            .physical_directory
                            .as_ref()
                            .is_some_and(|physical| overlaps(path, physical))
                })
            {
                return Err(blocked(root));
            }
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn preserved_gap_paths_bind_aliases_and_cannot_cover_mutation_or_missing_boundaries() {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let root = home.join(".hermes/skills");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let branch = root.join("external");
        std::os::unix::fs::symlink(&outside, &branch).unwrap();
        let mut options = crate::scan::ScanOptions::for_home(home);
        options.include_session_evidence = false;
        let scan = crate::scan::scan(&options).unwrap();
        let observed = scan
            .roots
            .iter()
            .find(|observed| observed.path == root)
            .unwrap();
        assert!(!observed.discovery_complete);
        let gaps = preserved_gaps(&scan, observed).unwrap();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].reason, SkillDiscoveryGapReason::UnapprovedBranch);
        assert!(validate_paths(&scan, vec![&root.join("mutable")]).is_ok());
        let alias = root.join("new-alias");
        std::os::unix::fs::symlink(&outside, &alias).unwrap();
        for path in [&branch, &outside, &alias.join("new-core"), &root] {
            assert!(
                validate_paths(&scan, vec![path]).is_err(),
                "{}",
                path.display()
            );
        }
        let mut legacy = scan.clone();
        legacy.skill_discovery_gaps.clear();
        assert!(preserved_gaps(&legacy, observed).is_err());
        for reason in [
            SkillDiscoveryGapReason::MissingBranch,
            SkillDiscoveryGapReason::DriftedBranch,
        ] {
            let mut blocked = scan.clone();
            blocked.skill_discovery_gaps.get_mut(&root).unwrap()[0].reason = reason;
            assert!(preserved_gaps(&blocked, observed).is_err());
        }
        std::fs::remove_file(&branch).unwrap();
        assert!(preserved_gaps(&scan, observed).is_err());
    }
}
