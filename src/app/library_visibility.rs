use super::*;

/// An implicit Roster move relocates its exact package, not every separate copy
/// with the same Skill identity. Explicit Library consolidation retains its
/// wider, independently validated source selection.
pub(super) fn validate(scan: &ScanResult, plan: &PreparedPlan) -> Result<()> {
    let mut relocated;
    let scope = if plan.roster_changes.is_empty() {
        scan
    } else {
        relocated = scan.clone();
        relocated.placements.retain(|placement| {
            plan.operations.iter().any(|operation| match operation {
                Operation::MoveRecoverable { source, .. } => {
                    placement.directory.starts_with(source)
                        || placement
                            .physical_directory
                            .as_ref()
                            .is_some_and(|path| path.starts_with(source))
                }
                Operation::CreateSymlink { target, .. } => placement.directory == *target,
                _ => false,
            })
        });
        &relocated
    };
    for change in &plan.library_changes {
        validate_library_native_visibility(
            scope,
            &change.skill_id,
            change
                .library_path
                .as_deref()
                .unwrap_or(&change.canonical_path),
            &change.canonical_path,
        )?;
    }
    Ok(())
}
