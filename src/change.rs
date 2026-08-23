//! Fail-closed filesystem changes for approved SkillRoster plans.
//!
//! The module deliberately has no "force" path. A plan is prepared against exact
//! fingerprints, and every apply/undo obtains the same process write lock.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct ChangeError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl ChangeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
        }
    }

    fn io(action: &str, path: &Path, source: io::Error) -> Self {
        Self::new(
            "filesystem_error",
            format!("{action} {}: {source}", path.display()),
        )
    }
}

impl fmt::Display for ChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ChangeError {}

pub type Result<T> = std::result::Result<T, ChangeError>;

#[derive(Clone, Debug)]
pub struct PrepareContext {
    /// Existing directories within which plan paths are allowed.
    pub approved_roots: Vec<PathBuf>,
    /// Existing private SkillRoster state directory. It stores the lock, journal,
    /// and recoverable artifacts, and must not be an Agent skill root.
    pub state_dir: PathBuf,
    /// Controls whether a trusted built-in workflow may emit filesystem work.
    /// Agent-authored stdin Plans must always use `GovernanceOnly`.
    pub(crate) operation_policy: OperationPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationPolicy {
    GovernanceOnly,
    BootstrapSetup,
    SourceUpdate,
    LibraryGovernance,
    #[cfg(test)]
    TestOnly,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    pub schema_version: u32,
    pub scan_id: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub operations: Vec<OperationInput>,
    #[serde(default)]
    pub roster_changes: Vec<RosterChange>,
    #[serde(default)]
    pub source_updates: Vec<SourceUpdateAction>,
    #[serde(default)]
    pub library_changes: Vec<LibraryChangeAction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryChangeAction {
    pub skill_id: String,
    pub canonical_placement_id: String,
    pub placement_ids: Vec<String>,
    pub requested_state: String,
    pub canonical_path: PathBuf,
    pub library_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceUpdateAction {
    pub skill_id: String,
    pub placement_id: String,
    pub choice: String,
    pub source: String,
    pub from_revision: String,
    pub to_revision: String,
    pub current_digest: String,
    pub expected_file_fingerprint: String,
    pub upstream_digest: String,
    pub baseline_trusted: bool,
    pub choice_reason: String,
    pub target: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterChange {
    pub agent: String,
    pub skill_id: String,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationInput {
    CreateDirectory {
        target: PathBuf,
        expected_fingerprint: String,
    },
    CreateSymlink {
        target: PathBuf,
        source: PathBuf,
        expected_fingerprint: String,
        expected_source_fingerprint: String,
    },
    WriteFile {
        target: PathBuf,
        content: String,
        expected_fingerprint: String,
    },
    ReplaceFile {
        target: PathBuf,
        content: String,
        expected_fingerprint: String,
    },
    RemoveSymlink {
        target: PathBuf,
        expected_fingerprint: String,
    },
    Copy {
        target: PathBuf,
        source: PathBuf,
        expected_fingerprint: String,
    },
    MoveRecoverable {
        target: PathBuf,
        source: PathBuf,
        expected_fingerprint: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    CreateDirectory {
        target: PathBuf,
        expected_fingerprint: String,
    },
    CreateSymlink {
        target: PathBuf,
        source: PathBuf,
        expected_fingerprint: String,
        expected_source_fingerprint: String,
    },
    WriteFile {
        target: PathBuf,
        content: String,
        expected_fingerprint: String,
    },
    ReplaceFile {
        target: PathBuf,
        content: String,
        expected_fingerprint: String,
    },
    RemoveSymlink {
        target: PathBuf,
        expected_fingerprint: String,
    },
    Copy {
        target: PathBuf,
        source: PathBuf,
        expected_fingerprint: String,
    },
    MoveRecoverable {
        target: PathBuf,
        source: PathBuf,
        expected_fingerprint: String,
    },
}

impl Operation {
    pub fn target(&self) -> &Path {
        match self {
            Self::CreateDirectory { target, .. }
            | Self::CreateSymlink { target, .. }
            | Self::WriteFile { target, .. }
            | Self::ReplaceFile { target, .. }
            | Self::RemoveSymlink { target, .. }
            | Self::Copy { target, .. }
            | Self::MoveRecoverable { target, .. } => target,
        }
    }

    pub fn source(&self) -> Option<&Path> {
        match self {
            Self::CreateSymlink { source, .. }
            | Self::Copy { source, .. }
            | Self::MoveRecoverable { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedPlan {
    pub id: String,
    pub scan_id: String,
    pub evidence_ids: Vec<String>,
    pub digest: String,
    pub operations: Vec<Operation>,
    pub roster_changes: Vec<RosterChange>,
    pub source_updates: Vec<SourceUpdateAction>,
    pub library_changes: Vec<LibraryChangeAction>,
    pub approved_roots: Vec<PathBuf>,
    pub state_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Applying,
    Applied,
    FailedRolledBack,
    RecoveryRequired,
    Undone,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeReceipt {
    pub id: String,
    pub plan_id: String,
    pub status: ReceiptStatus,
    pub changed_paths: Vec<PathBuf>,
    pub compensations: Vec<Compensation>,
    pub approved_roots: Vec<PathBuf>,
    pub state_dir: PathBuf,
    pub error: Option<String>,
    #[serde(default)]
    pub reverses_receipt_id: Option<String>,
    #[serde(default)]
    pub operation_results: Vec<JournalOperationResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalOperationResult {
    pub id: String,
    pub position: u32,
    pub action: String,
    pub target: PathBuf,
    pub status: String,
    pub before_fingerprint: Option<String>,
    pub after_fingerprint: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Compensation {
    /// Move a SkillRoster-created path into receipt-owned recovery storage.
    StashCreated {
        path: PathBuf,
        expected_fingerprint: String,
    },
    RestoreSymlink {
        path: PathBuf,
        target: PathBuf,
    },
    RenameBack {
        from: PathBuf,
        to: PathBuf,
        expected_fingerprint: String,
    },
    RestoreBackup {
        backup: PathBuf,
        original: PathBuf,
        created: PathBuf,
        expected_created: String,
    },
    RestoreReplacedFile {
        backup: PathBuf,
        target: PathBuf,
        expected_original: String,
        expected_replacement: String,
    },
}

#[derive(Clone, Debug)]
pub struct ApplyOutcome {
    pub receipt: ChangeReceipt,
    pub verification_passed: bool,
}

pub fn prepare(input: &str, ctx: &PrepareContext) -> Result<PreparedPlan> {
    let input: PlanInput = serde_json::from_str(input)
        .map_err(|error| ChangeError::new("invalid_plan_json", error.to_string()))?;
    if input.schema_version != 1 {
        return Err(ChangeError::new(
            "unsupported_plan_schema",
            format!(
                "plan schema_version must be 1, got {}",
                input.schema_version
            ),
        ));
    }
    if input.scan_id.trim().is_empty() {
        return Err(ChangeError::new(
            "invalid_plan",
            "scan_id must not be empty",
        ));
    }
    if input.operations.is_empty()
        && input.roster_changes.is_empty()
        && input.source_updates.is_empty()
        && input.library_changes.is_empty()
    {
        return Err(ChangeError::new(
            "invalid_plan",
            "a plan must contain a filesystem operation or Roster change",
        ));
    }
    match ctx.operation_policy {
        OperationPolicy::GovernanceOnly if !input.operations.is_empty() => {
            return Err(ChangeError::new(
                "operations_not_declarative",
                "Agent-authored Plans may request Roster state changes only; filesystem operations are derived by trusted SkillRoster workflows",
            ));
        }
        OperationPolicy::BootstrapSetup
            if input.operations.iter().any(|operation| {
                !matches!(
                    operation,
                    OperationInput::CreateDirectory { .. }
                        | OperationInput::WriteFile { .. }
                        | OperationInput::ReplaceFile { .. }
                )
            }) =>
        {
            return Err(ChangeError::new(
                "invalid_setup_operation",
                "bootstrap setup may only create directories and write or replace files in its fixed package",
            ));
        }
        OperationPolicy::SourceUpdate
            if input.operations.iter().any(|operation| {
                !matches!(
                    operation,
                    OperationInput::ReplaceFile { .. } | OperationInput::WriteFile { .. }
                )
            }) =>
        {
            return Err(ChangeError::new(
                "invalid_source_update_operation",
                "source updates may only replace the validated placement or create a non-overwriting sibling",
            ));
        }
        OperationPolicy::LibraryGovernance
            if input.operations.iter().any(|operation| {
                !matches!(
                    operation,
                    OperationInput::CreateDirectory { .. }
                        | OperationInput::MoveRecoverable { .. }
                        | OperationInput::CreateSymlink { .. }
                )
            }) =>
        {
            return Err(ChangeError::new(
                "invalid_library_operation",
                "Library governance may only create its root, move exact placements, and create links",
            ));
        }
        _ => {}
    }
    if matches!(
        ctx.operation_policy,
        OperationPolicy::GovernanceOnly
            | OperationPolicy::SourceUpdate
            | OperationPolicy::LibraryGovernance
    ) && input.evidence_ids.is_empty()
    {
        return Err(ChangeError::new(
            "missing_evidence",
            "Agent-authored Plans must reference Evidence from the latest Snapshot",
        ));
    }

    let state_dir = canonical_directory(&ctx.state_dir, "state_dir")?;
    let bootstrap_roots = if matches!(ctx.operation_policy, OperationPolicy::BootstrapSetup) {
        let roots = ctx
            .approved_roots
            .iter()
            .filter(|root| {
                !is_controlled_state_root(root, &ctx.state_dir)
                    && !is_controlled_state_root(root, &state_dir)
            })
            .cloned()
            .collect::<Vec<_>>();
        normalize_roots(&roots, &state_dir)?
            .into_iter()
            .filter(|root| !is_controlled_state_root(root, &state_dir))
            .collect()
    } else {
        Vec::new()
    };
    let roots = normalize_roots(&ctx.approved_roots, &state_dir)?;
    if roots.iter().any(|root| {
        (state_dir.starts_with(root) || root.starts_with(&state_dir))
            && !is_controlled_state_root(root, &state_dir)
    }) {
        return Err(ChangeError::new(
            "unsafe_state_directory",
            "state_dir must be separate from approved skill roots",
        ));
    }

    let mut targets = HashSet::new();
    let mut operations = Vec::with_capacity(input.operations.len());
    let mut projected_missing = HashSet::new();
    let mut projected_present = HashSet::new();
    let mut projected_fingerprints = HashMap::new();
    for raw in input.operations {
        let operation = normalize_operation(raw, &roots, &projected_present)?;
        if matches!(ctx.operation_policy, OperationPolicy::BootstrapSetup) {
            validate_bootstrap_operation_target(&operation, &bootstrap_roots)?;
        }
        let target = operation.target().to_path_buf();
        if !targets.insert(target.clone()) {
            return Err(ChangeError::new(
                "ambiguous_target",
                format!("multiple operations write {}", target.display()),
            ));
        }
        validate_projected_state(
            &operation,
            &roots,
            &projected_missing,
            &projected_present,
            &projected_fingerprints,
        )?;
        project_operation(
            &operation,
            &mut projected_missing,
            &mut projected_present,
            &mut projected_fingerprints,
        );
        operations.push(operation);
    }

    let digest_payload = serde_json::to_vec(&(
        input.scan_id.as_str(),
        &input.evidence_ids,
        &operations,
        &input.roster_changes,
        &input.source_updates,
        &input.library_changes,
    ))
    .map_err(|error| ChangeError::new("plan_encoding_failed", error.to_string()))?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(digest_payload)));
    Ok(PreparedPlan {
        id: format!("plan_{}", ulid::Ulid::new()),
        scan_id: input.scan_id,
        evidence_ids: input.evidence_ids,
        digest,
        operations,
        roster_changes: input.roster_changes,
        source_updates: input.source_updates,
        library_changes: input.library_changes,
        approved_roots: roots,
        state_dir,
    })
}

pub fn apply(plan: &PreparedPlan) -> Result<ApplyOutcome> {
    validate_prepared_plan(plan)?;
    let _lock = WriteLock::acquire(&plan.state_dir)?;
    apply_locked(plan)
}

pub(crate) fn apply_locked(plan: &PreparedPlan) -> Result<ApplyOutcome> {
    validate_prepared_plan(plan)?;
    reject_unresolved_recovery(&plan.state_dir)?;
    validate_operation_sequence(&plan.operations, &plan.approved_roots)?;

    let receipt_id = format!("receipt_{}", ulid::Ulid::new());
    let mut receipt = ChangeReceipt {
        id: receipt_id,
        plan_id: plan.id.clone(),
        status: ReceiptStatus::Applying,
        changed_paths: Vec::new(),
        compensations: Vec::new(),
        approved_roots: plan.approved_roots.clone(),
        state_dir: plan.state_dir.clone(),
        error: None,
        reverses_receipt_id: None,
        operation_results: Vec::new(),
    };
    persist_journal(&receipt)?;

    for (index, operation) in plan.operations.iter().enumerate() {
        receipt.operation_results.push(JournalOperationResult {
            id: format!("operation_{}", ulid::Ulid::new()),
            position: index as u32,
            action: operation_kind(operation).to_owned(),
            target: operation.target().to_path_buf(),
            status: "applying".to_owned(),
            before_fingerprint: fingerprint(operation.target()).ok(),
            after_fingerprint: None,
            error: None,
        });
        persist_journal(&receipt)?;
        if let Some(compensation) =
            stage_compensation(operation, &receipt.id, index, &plan.state_dir)?
        {
            receipt.compensations.push(compensation);
            persist_journal(&receipt)?;
        }
        match execute(operation, &receipt.id, index, &plan.approved_roots) {
            Ok(compensation) => {
                receipt.changed_paths.push(operation.target().to_path_buf());
                if let Some(compensation) = compensation {
                    receipt.compensations.push(compensation);
                }
                if let Some(result) = receipt.operation_results.last_mut() {
                    result.status = "applied".to_owned();
                    result.after_fingerprint = fingerprint(operation.target()).ok();
                }
                if let Err(journal_error) = persist_journal(&receipt) {
                    receipt.error = Some(journal_error.to_string());
                    let rollback = compensate_all(&receipt);
                    receipt.status = if rollback.is_ok() {
                        ReceiptStatus::FailedRolledBack
                    } else {
                        ReceiptStatus::RecoveryRequired
                    };
                    finalize_rollback_results(&mut receipt, rollback.is_ok());
                    if let Err(rollback_error) = rollback {
                        receipt.error = Some(format!(
                            "{journal_error}; compensation failed: {rollback_error}"
                        ));
                    }
                    // Best effort: if storage remains unavailable, the last durable
                    // journal stays Applying and therefore blocks future writes.
                    let _ = persist_journal(&receipt);
                    return Ok(ApplyOutcome {
                        receipt,
                        verification_passed: false,
                    });
                }
            }
            Err(error) => {
                if let Some(result) = receipt.operation_results.last_mut() {
                    result.status = "failed".to_owned();
                    result.after_fingerprint = fingerprint(operation.target()).ok();
                    result.error = Some(error.to_string());
                }
                receipt.error = Some(error.to_string());
                // Recursive copy can fail after creating a partial target. Record
                // the concrete artifact before compensating so a partial copy is
                // never reported as a clean rollback.
                if operation_creates_missing_target(operation)
                    && fs::symlink_metadata(operation.target()).is_ok()
                {
                    match fingerprint(operation.target()) {
                        Ok(expected_fingerprint) => {
                            receipt.compensations.push(Compensation::StashCreated {
                                path: operation.target().to_path_buf(),
                                expected_fingerprint,
                            });
                            receipt.changed_paths.push(operation.target().to_path_buf());
                            let _ = persist_journal(&receipt);
                        }
                        Err(fingerprint_error) => {
                            receipt.status = ReceiptStatus::RecoveryRequired;
                            receipt.error = Some(format!(
                                "{error}; partial target could not be fingerprinted: {fingerprint_error}"
                            ));
                            let _ = persist_journal(&receipt);
                            return Ok(ApplyOutcome {
                                receipt,
                                verification_passed: false,
                            });
                        }
                    }
                }
                let rollback = compensate_all(&receipt);
                receipt.status = if rollback.is_ok() {
                    ReceiptStatus::FailedRolledBack
                } else {
                    ReceiptStatus::RecoveryRequired
                };
                finalize_rollback_results(&mut receipt, rollback.is_ok());
                if let Err(rollback_error) = rollback {
                    receipt.error = Some(format!("{error}; compensation failed: {rollback_error}"));
                }
                persist_journal(&receipt)?;
                return Ok(ApplyOutcome {
                    receipt,
                    verification_passed: false,
                });
            }
        }
    }

    receipt.status = ReceiptStatus::Applied;
    if let Err(error) = persist_journal(&receipt) {
        receipt.status = ReceiptStatus::RecoveryRequired;
        receipt.error = Some(format!(
            "changes were applied but the final receipt could not be persisted: {error}"
        ));
        let _ = persist_journal(&receipt);
        return Ok(ApplyOutcome {
            receipt,
            verification_passed: false,
        });
    }
    Ok(ApplyOutcome {
        receipt,
        verification_passed: true,
    })
}

pub fn undo(receipt: &ChangeReceipt) -> Result<ApplyOutcome> {
    if receipt.status != ReceiptStatus::Applied {
        return Err(ChangeError::new(
            "receipt_not_undoable",
            "only an applied receipt can be undone",
        ));
    }
    let _lock = WriteLock::acquire(&receipt.state_dir)?;
    undo_locked(receipt)
}

pub(crate) fn undo_locked(receipt: &ChangeReceipt) -> Result<ApplyOutcome> {
    if receipt.status != ReceiptStatus::Applied {
        return Err(ChangeError::new(
            "receipt_not_undoable",
            "only an applied receipt can be undone",
        ));
    }
    reject_unresolved_recovery_except(&receipt.state_dir, &receipt.id)?;
    if reverse_receipt_exists(&receipt.state_dir, &receipt.id)? {
        return Err(ChangeError::new(
            "receipt_already_undone",
            format!("receipt {} has already been undone", receipt.id),
        ));
    }

    let mut undo_receipt = ChangeReceipt {
        id: format!("receipt_{}", ulid::Ulid::new()),
        plan_id: receipt.plan_id.clone(),
        status: ReceiptStatus::Applying,
        changed_paths: Vec::new(),
        compensations: Vec::new(),
        approved_roots: receipt.approved_roots.clone(),
        state_dir: receipt.state_dir.clone(),
        error: None,
        reverses_receipt_id: Some(receipt.id.clone()),
        operation_results: receipt
            .operation_results
            .iter()
            .map(|original| JournalOperationResult {
                id: format!("operation_{}", ulid::Ulid::new()),
                position: original.position,
                action: format!("undo_{}", original.action),
                target: original.target.clone(),
                status: "applying".to_owned(),
                before_fingerprint: fingerprint(&original.target).ok(),
                after_fingerprint: None,
                error: None,
            })
            .collect(),
    };
    persist_journal(&undo_receipt)?;
    match compensate_all(receipt) {
        Ok(()) => {
            for result in &mut undo_receipt.operation_results {
                result.status = "undone".to_owned();
                result.after_fingerprint = fingerprint(&result.target).ok();
            }
            undo_receipt.status = ReceiptStatus::Undone;
            undo_receipt.changed_paths = receipt.changed_paths.clone();
            persist_journal(&undo_receipt)?;
            Ok(ApplyOutcome {
                receipt: undo_receipt,
                verification_passed: true,
            })
        }
        Err(error) => {
            for result in &mut undo_receipt.operation_results {
                result.status = "failed".to_owned();
                result.after_fingerprint = fingerprint(&result.target).ok();
                result.error = Some(error.to_string());
            }
            undo_receipt.status = ReceiptStatus::RecoveryRequired;
            undo_receipt.error = Some(error.to_string());
            persist_journal(&undo_receipt)?;
            Ok(ApplyOutcome {
                receipt: undo_receipt,
                verification_passed: false,
            })
        }
    }
}

/// Compensates an Apply that cannot be committed at the governance metadata
/// layer. Unlike user-requested Undo this updates the original journal, so no
/// untracked reverse Receipt is invented before SQLite finalization.
pub fn rollback_apply(receipt: &ChangeReceipt) -> Result<ApplyOutcome> {
    if receipt.status != ReceiptStatus::Applied {
        return Err(ChangeError::new(
            "receipt_not_rollbackable",
            "only an applied receipt can be rolled back",
        ));
    }
    let _lock = WriteLock::acquire(&receipt.state_dir)?;
    rollback_apply_locked(receipt)
}

pub(crate) fn rollback_apply_locked(receipt: &ChangeReceipt) -> Result<ApplyOutcome> {
    if receipt.status != ReceiptStatus::Applied {
        return Err(ChangeError::new(
            "receipt_not_rollbackable",
            "only an applied receipt can be rolled back",
        ));
    }
    let mut rolled_back = receipt.clone();
    match compensate_all(receipt) {
        Ok(()) => {
            rolled_back.status = ReceiptStatus::FailedRolledBack;
            for result in &mut rolled_back.operation_results {
                result.status = "failed_rolled_back".to_owned();
                result.after_fingerprint = fingerprint(&result.target).ok();
            }
            persist_journal(&rolled_back)?;
            Ok(ApplyOutcome {
                receipt: rolled_back,
                verification_passed: true,
            })
        }
        Err(error) => {
            rolled_back.status = ReceiptStatus::RecoveryRequired;
            rolled_back.error = Some(error.to_string());
            for result in &mut rolled_back.operation_results {
                result.status = "recovery_required".to_owned();
                result.after_fingerprint = fingerprint(&result.target).ok();
                result.error = Some(error.to_string());
            }
            persist_journal(&rolled_back)?;
            Ok(ApplyOutcome {
                receipt: rolled_back,
                verification_passed: false,
            })
        }
    }
}

fn operation_kind(operation: &Operation) -> &'static str {
    match operation {
        Operation::CreateDirectory { .. } => "create_directory",
        Operation::CreateSymlink { .. } => "create_symlink",
        Operation::WriteFile { .. } => "write_file",
        Operation::ReplaceFile { .. } => "replace_file",
        Operation::RemoveSymlink { .. } => "remove_symlink",
        Operation::Copy { .. } => "copy",
        Operation::MoveRecoverable { .. } => "move_recoverable",
    }
}

fn finalize_rollback_results(receipt: &mut ChangeReceipt, recovered: bool) {
    for result in &mut receipt.operation_results {
        result.after_fingerprint = fingerprint(&result.target).ok();
        result.status = if recovered {
            if result.status == "failed" {
                "failed_rolled_back".to_owned()
            } else {
                "rolled_back".to_owned()
            }
        } else {
            "recovery_required".to_owned()
        };
    }
}

fn operation_creates_missing_target(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::CreateDirectory { .. }
            | Operation::CreateSymlink { .. }
            | Operation::WriteFile { .. }
            | Operation::Copy { .. }
            | Operation::MoveRecoverable { .. }
    )
}

fn normalize_operation(
    raw: OperationInput,
    roots: &[PathBuf],
    projected_present: &HashSet<PathBuf>,
) -> Result<Operation> {
    let normalized = match raw {
        OperationInput::CreateDirectory {
            target,
            expected_fingerprint,
        } => Operation::CreateDirectory {
            target: normalize_target(&target, roots)?,
            expected_fingerprint,
        },
        OperationInput::CreateSymlink {
            target,
            source,
            expected_fingerprint,
            expected_source_fingerprint,
        } => Operation::CreateSymlink {
            target: normalize_target(&target, roots)?,
            source: if projected_present.contains(&lexical_absolute(&source)?) {
                normalize_target(&source, roots)?
            } else {
                normalize_source(&source, roots)?
            },
            expected_fingerprint,
            expected_source_fingerprint,
        },
        OperationInput::WriteFile {
            target,
            content,
            expected_fingerprint,
        } => Operation::WriteFile {
            target: normalize_target(&target, roots)?,
            content,
            expected_fingerprint,
        },
        OperationInput::ReplaceFile {
            target,
            content,
            expected_fingerprint,
        } => Operation::ReplaceFile {
            target: normalize_target(&target, roots)?,
            content,
            expected_fingerprint,
        },
        OperationInput::RemoveSymlink {
            target,
            expected_fingerprint,
        } => Operation::RemoveSymlink {
            target: normalize_target(&target, roots)?,
            expected_fingerprint,
        },
        OperationInput::Copy {
            target,
            source,
            expected_fingerprint,
        } => Operation::Copy {
            target: normalize_target(&target, roots)?,
            source: normalize_source(&source, roots)?,
            expected_fingerprint,
        },
        OperationInput::MoveRecoverable {
            target,
            source,
            expected_fingerprint,
        } => Operation::MoveRecoverable {
            target: normalize_target(&target, roots)?,
            source: normalize_movable_source(&source, roots)?,
            expected_fingerprint,
        },
    };
    Ok(normalized)
}

fn validate_bootstrap_operation_target(operation: &Operation, roots: &[PathBuf]) -> Result<()> {
    let allowed = roots.iter().any(|root| {
        let Ok(relative) = operation.target().strip_prefix(root) else {
            return false;
        };
        match operation {
            Operation::CreateDirectory { .. } => {
                relative == Path::new("skillroster")
                    || relative == Path::new("skillroster/references")
            }
            Operation::WriteFile { .. } | Operation::ReplaceFile { .. } => {
                crate::bootstrap::is_managed_target(relative)
            }
            _ => false,
        }
    });
    if !allowed {
        return Err(ChangeError::new(
            "invalid_setup_target",
            format!(
                "bootstrap setup target {} is outside the fixed managed package",
                operation.target().display()
            ),
        ));
    }
    Ok(())
}

fn validate_current_state(operation: &Operation, roots: &[PathBuf]) -> Result<()> {
    normalize_target(operation.target(), roots)?;
    if let Some(source) = operation.source() {
        if matches!(operation, Operation::MoveRecoverable { .. }) {
            normalize_movable_source(source, roots)?;
        } else {
            normalize_source(source, roots)?;
        }
    }
    let (path, expected) = match operation {
        Operation::CreateDirectory {
            target,
            expected_fingerprint,
        }
        | Operation::RemoveSymlink {
            target,
            expected_fingerprint,
        }
        | Operation::WriteFile {
            target,
            expected_fingerprint,
            ..
        }
        | Operation::ReplaceFile {
            target,
            expected_fingerprint,
            ..
        } => (target, expected_fingerprint),
        Operation::CreateSymlink {
            target,
            expected_fingerprint,
            ..
        } => (target, expected_fingerprint),
        Operation::Copy {
            source,
            expected_fingerprint,
            ..
        }
        | Operation::MoveRecoverable {
            source,
            expected_fingerprint,
            ..
        } => (source, expected_fingerprint),
    };
    let actual = fingerprint(path)?;
    if &actual != expected {
        return Err(ChangeError::new(
            "plan_drifted",
            format!("{} expected {expected}, found {actual}", path.display()),
        ));
    }
    if let Operation::CreateSymlink {
        source,
        expected_source_fingerprint,
        ..
    } = operation
    {
        let actual_source = fingerprint(source)?;
        if &actual_source != expected_source_fingerprint {
            return Err(ChangeError::new(
                "plan_drifted",
                format!(
                    "{} expected {expected_source_fingerprint}, found {actual_source}",
                    source.display()
                ),
            ));
        }
    }
    match operation {
        Operation::CreateDirectory { target, .. }
        | Operation::CreateSymlink { target, .. }
        | Operation::WriteFile { target, .. } => require_missing(target)?,
        Operation::ReplaceFile { target, .. }
            if !fs::symlink_metadata(target)
                .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                .unwrap_or(false) =>
        {
            return Err(ChangeError::new(
                "not_a_regular_file",
                format!("{} is not a regular file", target.display()),
            ));
        }
        Operation::RemoveSymlink { target, .. }
            if fs::symlink_metadata(target)
                .map(|m| !m.file_type().is_symlink())
                .unwrap_or(true) =>
        {
            return Err(ChangeError::new(
                "not_a_symlink",
                format!("{} is not a symlink", target.display()),
            ));
        }
        Operation::Copy { target, .. } | Operation::MoveRecoverable { target, .. } => {
            require_missing(target)?
        }
        _ => {}
    }
    Ok(())
}

fn validate_operation_sequence(operations: &[Operation], roots: &[PathBuf]) -> Result<()> {
    let mut projected_missing = HashSet::new();
    let mut projected_present = HashSet::new();
    let mut projected_fingerprints = HashMap::new();
    for operation in operations {
        validate_projected_state(
            operation,
            roots,
            &projected_missing,
            &projected_present,
            &projected_fingerprints,
        )?;
        project_operation(
            operation,
            &mut projected_missing,
            &mut projected_present,
            &mut projected_fingerprints,
        );
    }
    Ok(())
}

fn validate_projected_state(
    operation: &Operation,
    roots: &[PathBuf],
    projected_missing: &HashSet<PathBuf>,
    projected_present: &HashSet<PathBuf>,
    projected_fingerprints: &HashMap<PathBuf, String>,
) -> Result<()> {
    if let Operation::CreateSymlink {
        target,
        source,
        expected_fingerprint,
        expected_source_fingerprint,
    } = operation
    {
        if projected_missing.contains(target) || projected_present.contains(source) {
            if projected_missing.contains(target) && expected_fingerprint != "missing" {
                return Err(ChangeError::new(
                    "invalid_projected_fingerprint",
                    format!(
                        "{} must expect missing after its planned move",
                        target.display()
                    ),
                ));
            }
            normalize_target(target, roots)?;
            if let Some(actual_source) = projected_fingerprints.get(source) {
                if actual_source != expected_source_fingerprint {
                    return Err(ChangeError::new(
                        "plan_drifted",
                        format!(
                            "{} expected {expected_source_fingerprint}, found {actual_source}",
                            source.display()
                        ),
                    ));
                }
            } else {
                normalize_source(source, roots)?;
                let actual_source = fingerprint(source)?;
                if &actual_source != expected_source_fingerprint {
                    return Err(ChangeError::new(
                        "plan_drifted",
                        format!(
                            "{} expected {expected_source_fingerprint}, found {actual_source}",
                            source.display()
                        ),
                    ));
                }
            }
            return Ok(());
        }
    }
    validate_current_state(operation, roots)
}

fn project_operation(
    operation: &Operation,
    projected_missing: &mut HashSet<PathBuf>,
    projected_present: &mut HashSet<PathBuf>,
    projected_fingerprints: &mut HashMap<PathBuf, String>,
) {
    match operation {
        Operation::MoveRecoverable {
            target,
            source,
            expected_fingerprint,
        } => {
            projected_missing.insert(source.clone());
            projected_present.insert(target.clone());
            projected_fingerprints.insert(target.clone(), expected_fingerprint.clone());
        }
        Operation::CreateDirectory { target, .. }
        | Operation::CreateSymlink { target, .. }
        | Operation::WriteFile { target, .. }
        | Operation::Copy { target, .. } => {
            projected_present.insert(target.clone());
        }
        Operation::ReplaceFile { target, .. } => {
            projected_present.insert(target.clone());
        }
        Operation::RemoveSymlink { target, .. } => {
            projected_missing.insert(target.clone());
        }
    }
}

fn stage_compensation(
    operation: &Operation,
    receipt_id: &str,
    index: usize,
    state_dir: &Path,
) -> Result<Option<Compensation>> {
    let Operation::ReplaceFile {
        target,
        content,
        expected_fingerprint,
    } = operation
    else {
        return Ok(None);
    };
    verify_fingerprint(target, expected_fingerprint)?;
    let directory = recovery_dir(state_dir, receipt_id);
    fs::create_dir_all(&directory)
        .map_err(|error| ChangeError::io("create recovery directory", &directory, error))?;
    let backup = directory.join(format!("replace-{index}.backup"));
    require_missing(&backup)?;
    fs::copy(target, &backup)
        .map_err(|error| ChangeError::io("persist replacement backup", target, error))?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&backup)
        .and_then(|file| file.sync_all())
        .map_err(|error| ChangeError::io("sync replacement backup", &backup, error))?;
    let backup_fingerprint = fingerprint(&backup)?;
    if backup_fingerprint != *expected_fingerprint {
        return Err(ChangeError::new(
            "backup_verification_failed",
            format!(
                "backup for {} does not match its precondition",
                target.display()
            ),
        ));
    }
    Ok(Some(Compensation::RestoreReplacedFile {
        backup,
        target: target.clone(),
        expected_original: expected_fingerprint.clone(),
        expected_replacement: file_content_fingerprint(content.as_bytes()),
    }))
}

fn replace_file(target: &Path, content: &str, receipt_id: &str, index: usize) -> Result<()> {
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ChangeError::new("unsafe_path", target.display().to_string()))?;
    let temp = target.with_file_name(format!(
        ".{file_name}.skillroster-replace-{receipt_id}-{index}"
    ));
    require_missing(&temp)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| ChangeError::io("create replacement", &temp, error))?;
    file.write_all(content.as_bytes())
        .map_err(|error| ChangeError::io("write replacement", &temp, error))?;
    file.sync_all()
        .map_err(|error| ChangeError::io("sync replacement", &temp, error))?;
    fs::remove_file(target)
        .map_err(|error| ChangeError::io("remove replaced file", target, error))?;
    fs::rename(&temp, target)
        .map_err(|error| ChangeError::io("publish replacement", target, error))?;
    Ok(())
}

fn file_content_fingerprint(content: &[u8]) -> String {
    format!("file:sha256:{}", hex::encode(Sha256::digest(content)))
}

fn execute(
    operation: &Operation,
    receipt_id: &str,
    index: usize,
    roots: &[PathBuf],
) -> Result<Option<Compensation>> {
    match operation {
        Operation::CreateDirectory { target, .. } => {
            fs::create_dir(target).map_err(|e| ChangeError::io("create directory", target, e))?;
            Ok(Some(Compensation::StashCreated {
                path: target.clone(),
                expected_fingerprint: fingerprint(target)?,
            }))
        }
        Operation::CreateSymlink { target, source, .. } => {
            create_symlink(source, target)?;
            Ok(Some(Compensation::StashCreated {
                path: target.clone(),
                expected_fingerprint: fingerprint(target)?,
            }))
        }
        Operation::WriteFile {
            target, content, ..
        } => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map_err(|e| ChangeError::io("create file", target, e))?;
            file.write_all(content.as_bytes())
                .map_err(|e| ChangeError::io("write file", target, e))?;
            file.sync_all()
                .map_err(|e| ChangeError::io("sync file", target, e))?;
            Ok(Some(Compensation::StashCreated {
                path: target.clone(),
                expected_fingerprint: fingerprint(target)?,
            }))
        }
        Operation::ReplaceFile {
            target, content, ..
        } => {
            replace_file(target, content, receipt_id, index)?;
            Ok(None)
        }
        Operation::RemoveSymlink { target, .. } => {
            let link_target =
                fs::read_link(target).map_err(|e| ChangeError::io("read symlink", target, e))?;
            remove_symlink(target)?;
            Ok(Some(Compensation::RestoreSymlink {
                path: target.clone(),
                target: link_target,
            }))
        }
        Operation::Copy { target, source, .. } => {
            copy_tree(source, target, roots)?;
            Ok(Some(Compensation::StashCreated {
                path: target.clone(),
                expected_fingerprint: fingerprint(target)?,
            }))
        }
        Operation::MoveRecoverable { target, source, .. } => {
            let expected = fingerprint(source)?;
            match fs::rename(source, target) {
                Ok(()) => {
                    return Ok(Some(Compensation::RenameBack {
                        from: target.clone(),
                        to: source.clone(),
                        expected_fingerprint: expected,
                    }));
                }
                Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {}
                Err(error) => return Err(ChangeError::io("move path", source, error)),
            }
            // Cross-device recovery must remain on the source filesystem, so the
            // original can be hidden atomically after the destination copy lands.
            let file_name = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| ChangeError::new("unsafe_path", source.display().to_string()))?;
            let backup = source.with_file_name(format!(
                ".{file_name}.skillroster-backup-{receipt_id}-{index}"
            ));
            require_missing(&backup)?;
            copy_tree(source, target, roots)?;
            if let Err(error) = fs::rename(source, &backup) {
                return Err(ChangeError::io("move source to recovery", source, error));
            }
            Ok(Some(Compensation::RestoreBackup {
                backup,
                original: source.clone(),
                created: target.clone(),
                expected_created: expected,
            }))
        }
    }
}

fn compensate_all(receipt: &ChangeReceipt) -> Result<()> {
    for (index, compensation) in receipt.compensations.iter().enumerate().rev() {
        compensate(
            compensation,
            &receipt.id,
            index,
            &receipt.approved_roots,
            &receipt.state_dir,
        )?;
    }
    Ok(())
}

fn compensate(
    compensation: &Compensation,
    receipt_id: &str,
    index: usize,
    roots: &[PathBuf],
    state_dir: &Path,
) -> Result<()> {
    match compensation {
        Compensation::StashCreated {
            path,
            expected_fingerprint,
        } => {
            verify_fingerprint(path, expected_fingerprint)?;
            normalize_target(path, roots)?;
            let stash = recovery_dir(state_dir, receipt_id).join(format!("undo-{index}"));
            fs::create_dir_all(stash.parent().expect("stash has parent"))
                .map_err(|e| ChangeError::io("create recovery directory", state_dir, e))?;
            fs::rename(path, &stash).map_err(|e| ChangeError::io("stash created path", path, e))?;
        }
        Compensation::RestoreSymlink { path, target } => {
            require_missing(path)?;
            normalize_target(path, roots)?;
            create_symlink(target, path)?;
        }
        Compensation::RenameBack {
            from,
            to,
            expected_fingerprint,
        } => {
            verify_fingerprint(from, expected_fingerprint)?;
            require_missing(to)?;
            normalize_target(from, roots)?;
            normalize_target(to, roots)?;
            fs::rename(from, to).map_err(|e| ChangeError::io("restore moved path", from, e))?;
        }
        Compensation::RestoreBackup {
            backup,
            original,
            created,
            expected_created,
        } => {
            verify_fingerprint(created, expected_created)?;
            require_missing(original)?;
            normalize_target(created, roots)?;
            normalize_target(original, roots)?;
            let stash = recovery_dir(state_dir, receipt_id).join(format!("undo-created-{index}"));
            fs::rename(created, &stash)
                .map_err(|e| ChangeError::io("stash copied destination", created, e))?;
            fs::rename(backup, original)
                .map_err(|e| ChangeError::io("restore source backup", backup, e))?;
        }
        Compensation::RestoreReplacedFile {
            backup,
            target,
            expected_original,
            expected_replacement,
        } => {
            let actual = fingerprint(target)?;
            if actual == *expected_original {
                fs::remove_file(backup)
                    .map_err(|error| ChangeError::io("remove unused backup", backup, error))?;
                return Ok(());
            }
            if actual != "missing" && actual != *expected_replacement {
                return Err(ChangeError::new(
                    "undo_drifted",
                    format!(
                        "{} expected {expected_replacement} or missing, found {actual}",
                        target.display()
                    ),
                ));
            }
            let restore = target.with_file_name(format!(
                ".{}.skillroster-restore-{receipt_id}-{index}",
                target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file")
            ));
            require_missing(&restore)?;
            fs::copy(backup, &restore)
                .map_err(|error| ChangeError::io("stage restored file", backup, error))?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&restore)
                .and_then(|file| file.sync_all())
                .map_err(|error| ChangeError::io("sync restored file", &restore, error))?;
            if actual != "missing" {
                fs::remove_file(target)
                    .map_err(|error| ChangeError::io("remove replacement", target, error))?;
            }
            fs::rename(&restore, target)
                .map_err(|error| ChangeError::io("restore replaced file", target, error))?;
            verify_fingerprint(target, expected_original)?;
            fs::remove_file(backup)
                .map_err(|error| ChangeError::io("remove consumed backup", backup, error))?;
        }
    }
    Ok(())
}

fn validate_prepared_plan(plan: &PreparedPlan) -> Result<()> {
    let payload = serde_json::to_vec(&(
        plan.scan_id.as_str(),
        &plan.evidence_ids,
        &plan.operations,
        &plan.roster_changes,
        &plan.source_updates,
        &plan.library_changes,
    ))
    .map_err(|e| ChangeError::new("plan_encoding_failed", e.to_string()))?;
    let actual = format!("sha256:{}", hex::encode(Sha256::digest(payload)));
    if actual != plan.digest {
        return Err(ChangeError::new(
            "plan_tampered",
            "prepared plan digest does not match its operations",
        ));
    }
    if plan.operations.is_empty()
        && plan.roster_changes.is_empty()
        && plan.source_updates.is_empty()
        && plan.library_changes.is_empty()
    {
        return Err(ChangeError::new(
            "invalid_plan",
            "prepared plan has no operations or Roster changes",
        ));
    }
    Ok(())
}

pub fn fingerprint(path: &Path) -> Result<String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok("missing".into()),
        Err(error) => return Err(ChangeError::io("inspect", path, error)),
    };
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|e| ChangeError::io("read symlink", path, e))?;
        return Ok(format!(
            "symlink:sha256:{}",
            hex::encode(Sha256::digest(target.as_os_str().as_encoded_bytes()))
        ));
    }
    if metadata.is_file() {
        let mut file = File::open(path).map_err(|e| ChangeError::io("open", path, e))?;
        let mut hash = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|e| ChangeError::io("read", path, e))?;
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
        return Ok(format!("file:sha256:{}", hex::encode(hash.finalize())));
    }
    if metadata.is_dir() {
        let mut entries = fs::read_dir(path)
            .map_err(|e| ChangeError::io("read directory", path, e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| ChangeError::io("read directory", path, e))?;
        entries.sort_by_key(|entry| entry.file_name());
        let mut hash = Sha256::new();
        for entry in entries {
            hash.update(entry.file_name().as_encoded_bytes());
            hash.update(fingerprint(&entry.path())?.as_bytes());
        }
        return Ok(format!("directory:sha256:{}", hex::encode(hash.finalize())));
    }
    Err(ChangeError::new(
        "unsupported_file_type",
        format!("{} is not a file, directory, or symlink", path.display()),
    ))
}

fn copy_tree(source: &Path, target: &Path, roots: &[PathBuf]) -> Result<()> {
    normalize_source(source, roots)?;
    normalize_target(target, roots)?;
    require_missing(target)?;
    let metadata =
        fs::symlink_metadata(source).map_err(|e| ChangeError::io("inspect source", source, e))?;
    if metadata.file_type().is_symlink() {
        return Err(ChangeError::new(
            "unsupported_copy_symlink",
            format!("refusing to recursively copy symlink {}", source.display()),
        ));
    }
    if metadata.is_file() {
        fs::copy(source, target).map_err(|e| ChangeError::io("copy file", source, e))?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(ChangeError::new(
            "unsupported_file_type",
            source.display().to_string(),
        ));
    }
    fs::create_dir(target).map_err(|e| ChangeError::io("create copied directory", target, e))?;
    for entry in
        fs::read_dir(source).map_err(|e| ChangeError::io("read source directory", source, e))?
    {
        let entry = entry.map_err(|e| ChangeError::io("read source directory", source, e))?;
        copy_tree(&entry.path(), &target.join(entry.file_name()), roots)?;
    }
    Ok(())
}

fn normalize_roots(roots: &[PathBuf], state_dir: &Path) -> Result<Vec<PathBuf>> {
    if roots.is_empty() {
        return Err(ChangeError::new(
            "missing_approved_roots",
            "at least one approved root is required",
        ));
    }
    let mut result = Vec::new();
    for root in roots {
        if root.exists() {
            result.push(canonical_directory(root, "approved root")?);
        } else {
            let mut root = lexical_absolute(root)?;
            if let Some(parent) = root.parent() {
                if let Ok(canonical_parent) = fs::canonicalize(parent) {
                    if canonical_parent == state_dir {
                        if let Some(name) = root.file_name() {
                            root = state_dir.join(name);
                        }
                    }
                }
            }
            if !is_controlled_state_root(&root, state_dir) {
                return Err(ChangeError::new(
                    "invalid_directory",
                    format!("approved root {} does not exist", root.display()),
                ));
            }
            result.push(root);
        }
    }
    result.sort();
    result.dedup();
    Ok(result)
}

fn is_controlled_state_root(root: &Path, state_dir: &Path) -> bool {
    root == state_dir.join("library") || root == state_dir.join("plan-backups")
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .map_err(|e| ChangeError::io(&format!("canonicalize {label}"), path, e))?;
    if !canonical.is_dir() {
        return Err(ChangeError::new(
            "invalid_directory",
            format!("{label} {} is not a directory", path.display()),
        ));
    }
    Ok(canonical)
}

fn lexical_absolute(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(ChangeError::new(
            "unsafe_path",
            format!("{} is not absolute", path.display()),
        ));
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                out.push(component.as_os_str())
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return Err(ChangeError::new("unsafe_path", path.display().to_string()));
                }
            }
        }
    }
    Ok(out)
}

fn normalize_target(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    let path = lexical_absolute(path)?;
    if roots.contains(&path) {
        return Ok(path);
    }
    if roots
        .iter()
        .any(|root| !root.exists() && path.starts_with(root))
    {
        return Ok(path);
    }
    let parent = path
        .parent()
        .ok_or_else(|| ChangeError::new("unsafe_path", path.display().to_string()))?;
    let ancestor = nearest_existing_ancestor(parent)?;
    let resolved = fs::canonicalize(&ancestor)
        .map_err(|e| ChangeError::io("canonicalize ancestor", &ancestor, e))?;
    if !roots.iter().any(|root| resolved.starts_with(root)) {
        let mut cursor = ancestor.parent();
        let mut crossed_from_root = false;
        while let Some(candidate) = cursor {
            if let Ok(canonical) = fs::canonicalize(candidate) {
                if roots.iter().any(|root| canonical.starts_with(root)) {
                    crossed_from_root = true;
                    break;
                }
            }
            cursor = candidate.parent();
        }
        let code = if crossed_from_root {
            "path_escapes_through_symlink"
        } else {
            "path_outside_approved_roots"
        };
        return Err(ChangeError::new(code, path.display().to_string()));
    }
    let suffix = path
        .strip_prefix(&ancestor)
        .map_err(|_| ChangeError::new("unsafe_path", path.display().to_string()))?;
    Ok(resolved.join(suffix))
}

fn normalize_source(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    let path = lexical_absolute(path)?;
    let canonical =
        fs::canonicalize(&path).map_err(|e| ChangeError::io("canonicalize source", &path, e))?;
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(ChangeError::new(
            "path_outside_approved_roots",
            path.display().to_string(),
        ));
    }
    Ok(canonical)
}

/// Validates the directory entry being moved without following its final
/// symlink. Moving a link out of an Agent root must move the link itself, not
/// the canonical Skill directory it points at.
fn normalize_movable_source(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    let path = normalize_target(path, roots)?;
    fs::symlink_metadata(&path)
        .map_err(|error| ChangeError::io("inspect move source", &path, error))?;
    Ok(path)
}

fn nearest_existing_ancestor(path: &Path) -> Result<PathBuf> {
    let mut current = path;
    loop {
        if fs::symlink_metadata(current).is_ok() {
            return Ok(current.to_path_buf());
        }
        current = current
            .parent()
            .ok_or_else(|| ChangeError::new("unsafe_path", path.display().to_string()))?;
    }
}

fn require_missing(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(ChangeError::new(
            "target_exists",
            path.display().to_string(),
        ));
    }
    Ok(())
}

fn verify_fingerprint(path: &Path, expected: &str) -> Result<()> {
    let actual = fingerprint(path)?;
    if actual != expected {
        return Err(ChangeError::new(
            "undo_drifted",
            format!("{} expected {expected}, found {actual}", path.display()),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn create_symlink(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target)
        .map_err(|e| ChangeError::io("create symlink", target, e))
}

#[cfg(windows)]
fn create_symlink(source: &Path, target: &Path) -> Result<()> {
    let metadata =
        fs::metadata(source).map_err(|e| ChangeError::io("inspect symlink source", source, e))?;
    let result = if metadata.is_dir() {
        std::os::windows::fs::symlink_dir(source, target)
    } else {
        std::os::windows::fs::symlink_file(source, target)
    };
    result.map_err(|e| ChangeError::io("create symlink", target, e))
}

#[cfg(unix)]
fn remove_symlink(path: &Path) -> Result<()> {
    fs::remove_file(path).map_err(|error| ChangeError::io("remove symlink", path, error))
}

#[cfg(windows)]
fn remove_symlink(path: &Path) -> Result<()> {
    let result = match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir(path),
        _ => fs::remove_file(path),
    };
    result.map_err(|error| ChangeError::io("remove symlink", path, error))
}

fn recovery_dir(state_dir: &Path, receipt_id: &str) -> PathBuf {
    state_dir.join("recovery").join(receipt_id)
}

fn persist_journal(receipt: &ChangeReceipt) -> Result<()> {
    let directory = receipt.state_dir.join("receipts");
    fs::create_dir_all(&directory)
        .map_err(|e| ChangeError::io("create receipt directory", &directory, e))?;
    let path = directory.join(format!("{}.json", receipt.id));
    let temp = directory.join(format!(".{}.tmp", receipt.id));
    let bytes = serde_json::to_vec_pretty(receipt)
        .map_err(|e| ChangeError::new("receipt_encoding_failed", e.to_string()))?;
    let mut file =
        File::create(&temp).map_err(|e| ChangeError::io("create receipt journal", &temp, e))?;
    file.write_all(&bytes)
        .map_err(|e| ChangeError::io("write receipt journal", &temp, e))?;
    file.sync_all()
        .map_err(|e| ChangeError::io("sync receipt journal", &temp, e))?;
    fs::rename(&temp, &path).map_err(|e| ChangeError::io("publish receipt journal", &path, e))?;
    Ok(())
}

pub(crate) fn persist_journal_state(receipt: &ChangeReceipt) -> Result<()> {
    persist_journal(receipt)
}

/// Loads the durable filesystem journal without changing it. Invalid entries are
/// surfaced as errors because silently skipping one could permit an unsafe write.
pub fn journals(state_dir: &Path) -> Result<Vec<ChangeReceipt>> {
    let directory = state_dir.join("receipts");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ChangeError::io("read receipts", &directory, error)),
    };
    let expected_state_dir = fs::canonicalize(state_dir)
        .map_err(|error| ChangeError::io("canonicalize state directory", state_dir, error))?;
    let mut receipts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ChangeError::io("read receipt", &directory, error))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| ChangeError::io("inspect receipt", &path, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ChangeError::new(
                "invalid_receipt_journal",
                format!("receipt journal {} is not a regular file", path.display()),
            ));
        }
        let bytes =
            fs::read(&path).map_err(|error| ChangeError::io("read receipt", &path, error))?;
        let receipt: ChangeReceipt = serde_json::from_slice(&bytes)
            .map_err(|error| ChangeError::new("invalid_receipt_journal", error.to_string()))?;
        let receipt_state_dir = fs::canonicalize(&receipt.state_dir).map_err(|error| {
            ChangeError::io(
                "canonicalize journal state directory",
                &receipt.state_dir,
                error,
            )
        })?;
        if receipt_state_dir != expected_state_dir {
            return Err(ChangeError::new(
                "invalid_receipt_journal",
                format!(
                    "journal {} belongs to a different state directory",
                    receipt.id
                ),
            ));
        }
        receipts.push(receipt);
    }
    receipts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(receipts)
}

fn reject_unresolved_recovery(state_dir: &Path) -> Result<()> {
    reject_unresolved_recovery_except(state_dir, "")
}

fn reverse_receipt_exists(state_dir: &Path, original_receipt_id: &str) -> Result<bool> {
    let directory = state_dir.join("receipts");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(ChangeError::io("read receipts", &directory, error)),
    };
    for entry in entries {
        let entry = entry.map_err(|error| ChangeError::io("read receipt", &directory, error))?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(entry.path())
            .map_err(|error| ChangeError::io("read receipt", &entry.path(), error))?;
        let existing: ChangeReceipt = serde_json::from_slice(&bytes)
            .map_err(|error| ChangeError::new("invalid_receipt_journal", error.to_string()))?;
        if existing.status == ReceiptStatus::Undone
            && existing.reverses_receipt_id.as_deref() == Some(original_receipt_id)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn reject_unresolved_recovery_except(state_dir: &Path, except: &str) -> Result<()> {
    let directory = state_dir.join("receipts");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(ChangeError::io("read receipts", &directory, e)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| ChangeError::io("read receipts", &directory, e))?;
        if entry.path().extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(entry.path())
            .map_err(|e| ChangeError::io("read receipt", &entry.path(), e))?;
        let existing: ChangeReceipt = serde_json::from_slice(&bytes)
            .map_err(|e| ChangeError::new("invalid_receipt_journal", e.to_string()))?;
        if existing.id != except
            && matches!(
                existing.status,
                ReceiptStatus::Applying | ReceiptStatus::RecoveryRequired
            )
        {
            return Err(ChangeError::new(
                "recovery_required",
                format!(
                    "receipt {} must be recovered before another write",
                    existing.id
                ),
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct StateLock(File);

impl StateLock {
    pub(crate) fn acquire_shared(state_dir: &Path) -> Result<Self> {
        Self::acquire(state_dir, false)
    }

    pub(crate) fn acquire_exclusive(state_dir: &Path) -> Result<Self> {
        Self::acquire(state_dir, true)
    }

    fn acquire(state_dir: &Path, exclusive: bool) -> Result<Self> {
        let path = state_dir.join("write.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| ChangeError::io("open state lock", &path, e))?;
        let result = if exclusive {
            FileExt::try_lock_exclusive(&file)
        } else {
            FileExt::try_lock_shared(&file)
        };
        result.map_err(|error| {
            let mut result = ChangeError::new(
                "write_locked",
                format!(
                    "another SkillRoster command is using local state guarded by {}: {error}",
                    path.display()
                ),
            );
            result.retryable = true;
            result
        })?;
        Ok(Self(file))
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

pub(crate) struct WriteLock {
    _guard: StateLock,
}

impl WriteLock {
    pub(crate) fn acquire(state_dir: &Path) -> Result<Self> {
        StateLock::acquire_exclusive(state_dir).map(|guard| Self { _guard: guard })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let state = temp.path().join("state");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&state).unwrap();
        (temp, root, state)
    }

    #[test]
    fn state_lock_allows_concurrent_readers_but_excludes_writers() {
        let (_temp, _root, state) = fixture();
        let first_reader = StateLock::acquire_shared(&state).unwrap();
        let second_reader = StateLock::acquire_shared(&state).unwrap();
        let blocked_writer = StateLock::acquire_exclusive(&state).unwrap_err();
        assert_eq!(blocked_writer.code, "write_locked");
        drop(second_reader);
        drop(first_reader);
        let writer = StateLock::acquire_exclusive(&state).unwrap();
        let blocked_reader = StateLock::acquire_shared(&state).unwrap_err();
        assert_eq!(blocked_reader.code, "write_locked");
        drop(writer);
    }

    #[test]
    fn prepare_requires_the_supported_request_schema() {
        let (_temp, root, state) = fixture();
        let context = PrepareContext {
            approved_roots: vec![root],
            state_dir: state,
            operation_policy: OperationPolicy::GovernanceOnly,
        };
        let missing = prepare(r#"{"scan_id":"scan_1","roster_changes":[]}"#, &context).unwrap_err();
        assert_eq!(missing.code, "invalid_plan_json");

        let unsupported = prepare(
            r#"{"schema_version":2,"scan_id":"scan_1","roster_changes":[]}"#,
            &context,
        )
        .unwrap_err();
        assert_eq!(unsupported.code, "unsupported_plan_schema");
    }

    #[test]
    fn prepare_refuses_target_through_escaping_symlink() {
        let (_temp, root, state) = fixture();
        let outside = state.join("outside");
        fs::create_dir(&outside).unwrap();
        create_symlink(&outside, &root.join("escape")).unwrap();
        let input = serde_json::json!({"schema_version":1,"scan_id":"scan_1","operations":[{"kind":"create_directory","target":root.join("escape/new"),"expected_fingerprint":"missing"}]}).to_string();
        let error = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "path_escapes_through_symlink");
    }

    #[test]
    fn governance_plan_refuses_filesystem_operations() {
        let (_temp, root, state) = fixture();
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "write_file",
                "target": root.join("arbitrary.txt"),
                "content": "arbitrary",
                "expected_fingerprint": "missing"
            }]
        })
        .to_string();
        let error = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::GovernanceOnly,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "operations_not_declarative");
    }

    #[test]
    fn bootstrap_setup_accepts_only_fixed_package_targets() {
        let (_temp, root, state) = fixture();
        let library = state.join("library");
        let plan_backups = state.join("plan-backups");
        fs::create_dir(&library).unwrap();
        fs::create_dir(&plan_backups).unwrap();
        let context = PrepareContext {
            approved_roots: vec![root.clone(), library.clone(), plan_backups.clone()],
            state_dir: state,
            operation_policy: OperationPolicy::BootstrapSetup,
        };
        let allowed = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "write_file",
                "target": root.join("skillroster/references/../references/routing.md"),
                "content": "routing",
                "expected_fingerprint": "missing"
            }]
        })
        .to_string();
        assert!(prepare(&allowed, &context).is_ok());

        for target in [
            root.join("skillroster/notes.md"),
            root.join("other/SKILL.md"),
            library.join("skillroster/SKILL.md"),
            plan_backups.join("skillroster/SKILL.md"),
        ] {
            let refused = serde_json::json!({
                "schema_version": 1,
                "scan_id": "scan_1",
                "operations": [{
                    "kind": "write_file",
                    "target": target,
                    "content": "unexpected",
                    "expected_fingerprint": "missing"
                }]
            })
            .to_string();
            assert_eq!(
                prepare(&refused, &context).unwrap_err().code,
                "invalid_setup_target"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn bootstrap_setup_rejects_agent_root_aliases_to_controlled_state_roots() {
        use std::os::unix::fs::symlink;

        let (_temp, root, state) = fixture();
        let library = state.join("library");
        let plan_backups = state.join("plan-backups");
        fs::create_dir(&library).unwrap();
        fs::create_dir(&plan_backups).unwrap();
        let library_alias = root.join("library-alias");
        let backups_alias = root.join("backups-alias");
        symlink(&library, &library_alias).unwrap();
        symlink(&plan_backups, &backups_alias).unwrap();
        let context = PrepareContext {
            approved_roots: vec![library_alias.clone(), backups_alias.clone()],
            state_dir: state,
            operation_policy: OperationPolicy::BootstrapSetup,
        };

        for target in [
            library_alias.join("skillroster/SKILL.md"),
            backups_alias.join("skillroster/SKILL.md"),
        ] {
            let input = serde_json::json!({
                "schema_version": 1,
                "scan_id": "scan_1",
                "operations": [{
                    "kind": "write_file",
                    "target": target,
                    "content": "unexpected",
                    "expected_fingerprint": "missing"
                }]
            })
            .to_string();
            assert_eq!(
                prepare(&input, &context).unwrap_err().code,
                "invalid_setup_target"
            );
        }
    }

    #[test]
    fn apply_refuses_a_drifted_plan() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        fs::write(&source, "before").unwrap();
        let input = serde_json::json!({"schema_version":1,"scan_id":"scan_1","operations":[{"kind":"copy","source":source,"target":root.join("copy"),"expected_fingerprint":fingerprint(&source).unwrap()}]}).to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        fs::write(&source, "after").unwrap();
        assert_eq!(apply(&plan).unwrap_err().code, "plan_drifted");
    }

    #[test]
    fn apply_and_undo_round_trip_created_symlink() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        let target = root.join("linked");
        let source_fingerprint = fingerprint(&source).unwrap();
        let input = serde_json::json!({"schema_version":1,"scan_id":"scan_1","operations":[{"kind":"create_symlink","source":source,"target":target,"expected_fingerprint":"missing","expected_source_fingerprint":source_fingerprint}]}).to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let applied = apply(&plan).unwrap();
        assert_eq!(applied.receipt.status, ReceiptStatus::Applied);
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let undone = undo(&applied.receipt).unwrap();
        assert_eq!(undone.receipt.status, ReceiptStatus::Undone);
        assert!(!target.exists());
        let repeated = undo(&applied.receipt).unwrap_err();
        assert_eq!(repeated.code, "receipt_already_undone");
    }

    #[test]
    fn undo_refuses_drift_in_a_created_file() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        fs::write(&source, "original").unwrap();
        let target = root.join("copy");
        let input = serde_json::json!({"schema_version":1,"scan_id":"scan_1","operations":[{"kind":"copy","source":source,"target":target,"expected_fingerprint":fingerprint(&source).unwrap()}]}).to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let applied = apply(&plan).unwrap();
        fs::write(&target, "user edit").unwrap();
        let undone = undo(&applied.receipt).unwrap();
        assert_eq!(undone.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_to_string(target).unwrap(), "user edit");
    }

    #[test]
    fn write_file_round_trips_without_overwriting_existing_content() {
        let (_temp, root, state) = fixture();
        let target = root.join("SKILL.md");
        let input = serde_json::json!({"schema_version":1,"scan_id":"scan_1","operations":[{"kind":"write_file","target":target,"content":"---\nname: test\n---\n","expected_fingerprint":"missing"}]}).to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let applied = apply(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "---\nname: test\n---\n"
        );
        let undone = undo(&applied.receipt).unwrap();
        assert_eq!(undone.receipt.status, ReceiptStatus::Undone);
        assert!(!target.exists());
    }

    #[test]
    fn move_recoverable_moves_a_symlink_entry_not_its_canonical_directory() {
        let (_temp, root, state) = fixture();
        let canonical = root.join("canonical");
        fs::create_dir(&canonical).unwrap();
        fs::write(canonical.join("SKILL.md"), "---\nname: linked\n---\n").unwrap();
        let link = root.join("linked");
        create_symlink(&canonical, &link).unwrap();
        let backup = root.join("linked-backup");
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "move_recoverable",
                "source": link,
                "target": backup,
                "expected_fingerprint": fingerprint(&link).unwrap()
            }]
        })
        .to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let normalized_source = plan.operations[0].source().unwrap();
        assert_eq!(normalized_source.file_name().unwrap(), "linked");
        assert_ne!(normalized_source, fs::canonicalize(&link).unwrap());

        let applied = apply(&plan).unwrap();
        assert!(canonical.join("SKILL.md").is_file());
        assert!(fs::symlink_metadata(&link).is_err());
        assert!(
            fs::symlink_metadata(&backup)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        let undone = undo(&applied.receipt).unwrap();
        assert_eq!(undone.receipt.status, ReceiptStatus::Undone);
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(canonical.join("SKILL.md").is_file());
    }

    #[test]
    fn journals_refuse_symlinks_before_reading_their_targets() {
        let (temp, _root, state) = fixture();
        let outside = temp.path().join("outside.json");
        fs::write(&outside, r#"{"private":"must not be read as a journal"}"#).unwrap();
        let receipts = state.join("receipts");
        fs::create_dir(&receipts).unwrap();
        create_symlink(&outside, &receipts.join("receipt_escape.json")).unwrap();

        let error = journals(&state).unwrap_err();
        assert_eq!(error.code, "invalid_receipt_journal");
        assert!(!error.message.contains("must not be read"));
    }

    #[test]
    fn receipt_records_every_filesystem_operation_without_file_content() {
        let (_temp, root, state) = fixture();
        let replace = root.join("replace.txt");
        fs::write(&replace, "private-before").unwrap();
        let removed_link = root.join("removed-link");
        let link_source = root.join("link-source");
        fs::create_dir(&link_source).unwrap();
        create_symlink(&link_source, &removed_link).unwrap();
        let copy_source = root.join("copy-source.txt");
        fs::write(&copy_source, "private-copy").unwrap();
        let move_source = root.join("move-source");
        fs::create_dir(&move_source).unwrap();
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id":"scan_1",
            "operations":[
                {"kind":"create_directory","target":root.join("created"),"expected_fingerprint":"missing"},
                {"kind":"create_symlink","source":link_source,"target":root.join("created-link"),"expected_fingerprint":"missing","expected_source_fingerprint":fingerprint(&link_source).unwrap()},
                {"kind":"write_file","target":root.join("written.txt"),"content":"private-written","expected_fingerprint":"missing"},
                {"kind":"replace_file","target":replace,"content":"private-after","expected_fingerprint":fingerprint(&replace).unwrap()},
                {"kind":"remove_symlink","target":removed_link,"expected_fingerprint":fingerprint(&removed_link).unwrap()},
                {"kind":"copy","source":copy_source,"target":root.join("copied.txt"),"expected_fingerprint":fingerprint(&copy_source).unwrap()},
                {"kind":"move_recoverable","source":move_source,"target":root.join("moved"),"expected_fingerprint":fingerprint(&move_source).unwrap()}
            ]
        }).to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let applied = apply(&plan).unwrap();
        assert_eq!(applied.receipt.operation_results.len(), 7);
        assert!(applied.receipt.operation_results.iter().all(|result| {
            result.status == "applied"
                && result.before_fingerprint.is_some()
                && result.after_fingerprint.is_some()
        }));
        let encoded = serde_json::to_string(&applied.receipt.operation_results).unwrap();
        assert!(!encoded.contains("private-written"));
        assert!(!encoded.contains("private-copy"));
        assert!(!encoded.contains("private-before"));
    }

    #[test]
    fn failed_step_is_journaled_and_partial_copy_is_rolled_back() {
        let (_temp, root, state) = fixture();
        let unsafe_source = root.join("unsafe-source");
        fs::create_dir(&unsafe_source).unwrap();
        create_symlink(&root, &unsafe_source.join("nested-link")).unwrap();
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id":"scan_1",
            "operations":[
                {"kind":"create_directory","target":root.join("first"),"expected_fingerprint":"missing"},
                {"kind":"copy","source":unsafe_source,"target":root.join("partial-copy"),"expected_fingerprint":fingerprint(&unsafe_source).unwrap()}
            ]
        }).to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root.clone()],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let outcome = apply(&plan).unwrap();
        assert_eq!(outcome.receipt.status, ReceiptStatus::FailedRolledBack);
        assert_eq!(outcome.receipt.operation_results[0].status, "rolled_back");
        assert_eq!(
            outcome.receipt.operation_results[1].status,
            "failed_rolled_back"
        );
        assert!(outcome.receipt.operation_results[1].error.is_some());
        assert!(!root.join("first").exists());
        assert!(!root.join("partial-copy").exists());
    }
}
