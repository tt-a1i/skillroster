//! Fail-closed filesystem changes for approved SkillRoster plans.
//!
//! The module deliberately has no "force" path. A plan is prepared against exact
//! fingerprints, and every apply/undo obtains the same process write lock.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::anchored_fs::AnchoredFs;
use crate::durable_fs::{DirectorySync, SystemDirectorySync};
use crate::package_fingerprint::{
    MAX_SKILL_PACKAGE_DEPTH, PackageHashBuilder, ignored_package_entry_name,
    normalized_relative_package_path,
};

#[cfg(test)]
thread_local! {
    static BEFORE_MUTATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static BEFORE_EXECUTE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static BEFORE_REMOVE_SYMLINK_RENAME_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static BEFORE_SEQUENCE_VALIDATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn run_before_mutation_hook() {
    BEFORE_MUTATION_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_before_mutation_hook() {}

#[cfg(test)]
fn run_before_execute_hook() {
    BEFORE_EXECUTE_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_before_execute_hook() {}

#[cfg(test)]
fn run_before_remove_symlink_rename_hook() {
    BEFORE_REMOVE_SYMLINK_RENAME_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_before_remove_symlink_rename_hook() {}

#[cfg(test)]
fn run_before_sequence_validation_hook() {
    BEFORE_SEQUENCE_VALIDATION_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_before_sequence_validation_hook() {}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OperationPolicy {
    GovernanceOnly,
    BootstrapSetup {
        missing_skill_roots: Vec<PathBuf>,
    },
    SourceUpdate,
    LibraryGovernance,
    #[cfg(test)]
    TestOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    #[serde(default)]
    pub path_projections: Vec<PathProjection>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PathProjection {
    pub path: PathBuf,
    pub expected_before_fingerprint: String,
    pub expected_after_fingerprint: String,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    #[serde(default)]
    pub path_projections: Vec<PathProjection>,
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
    RestoreRemovedSymlink {
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        staged_replacement: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_windows_security: Option<String>,
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
    match &ctx.operation_policy {
        OperationPolicy::GovernanceOnly if !input.operations.is_empty() => {
            return Err(ChangeError::new(
                "operations_not_declarative",
                "Agent-authored Plans may request Roster state changes only; filesystem operations are derived by trusted SkillRoster workflows",
            ));
        }
        OperationPolicy::BootstrapSetup { .. }
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
    if !matches!(ctx.operation_policy, OperationPolicy::BootstrapSetup { .. })
        && !input.path_projections.is_empty()
    {
        return Err(ChangeError::new(
            "invalid_path_projection",
            "path projections are reserved for trusted Bootstrap Setup Plans",
        ));
    }

    let state_dir = canonical_directory(&ctx.state_dir, "state_dir")?;
    let roots = normalize_roots(&ctx.approved_roots, &state_dir)?;
    let bootstrap_roots = if matches!(
        &ctx.operation_policy,
        OperationPolicy::BootstrapSetup { .. }
    ) {
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
    let bootstrap_missing_skill_roots = match &ctx.operation_policy {
        OperationPolicy::BootstrapSetup {
            missing_skill_roots,
        } => missing_skill_roots
            .iter()
            .map(|root| normalize_target(root, &bootstrap_roots))
            .collect::<Result<Vec<_>>>()?,
        _ => Vec::new(),
    };
    if !matches!(
        &ctx.operation_policy,
        OperationPolicy::BootstrapSetup { .. }
    ) && roots.iter().any(|root| {
        (state_dir.starts_with(root) || root.starts_with(&state_dir))
            && !is_controlled_state_root(root, &state_dir)
    }) {
        return Err(ChangeError::new(
            "unsafe_state_directory",
            "state_dir must be separate from approved skill roots",
        ));
    }

    let raw_path_projections = input.path_projections;
    let mut targets = HashSet::new();
    let mut operations = Vec::with_capacity(input.operations.len());
    let mut projected_missing = HashSet::new();
    let mut projected_present = HashSet::new();
    let mut projected_fingerprints: HashMap<PathBuf, String> = HashMap::new();
    for raw in input.operations {
        let operation = normalize_operation(raw, &roots, &projected_present)?;
        if matches!(
            &ctx.operation_policy,
            OperationPolicy::BootstrapSetup { .. }
        ) {
            validate_bootstrap_operation_target(
                &operation,
                &bootstrap_roots,
                &bootstrap_missing_skill_roots,
            )?;
        }
        let target = operation.target().to_path_buf();
        if (state_dir.starts_with(&target) || target.starts_with(&state_dir))
            && !is_controlled_state_path(&target, &state_dir)
        {
            return Err(ChangeError::new(
                "unsafe_state_directory",
                "operation target must be separate from the state directory",
            ));
        }
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
    if matches!(
        &ctx.operation_policy,
        OperationPolicy::BootstrapSetup { .. }
    ) && operations
        .iter()
        .filter_map(|operation| bootstrap_skill_root(operation.target()))
        .any(|root| state_dir.starts_with(root) || root.starts_with(&state_dir))
    {
        return Err(ChangeError::new(
            "unsafe_state_directory",
            "state_dir must be separate from Bootstrap Skill roots",
        ));
    }

    let allowed_projection_paths = operations
        .iter()
        .filter_map(|operation| bootstrap_skill_directory(operation.target()))
        .map(Path::to_path_buf)
        .collect::<HashSet<_>>();
    let mut projection_paths = HashSet::new();
    let mut path_projections = Vec::with_capacity(raw_path_projections.len());
    for projection in raw_path_projections {
        let path = normalize_target(&projection.path, &roots)?;
        if !allowed_projection_paths.contains(&path) {
            return Err(ChangeError::new(
                "invalid_path_projection",
                format!(
                    "Bootstrap projection {} is not a planned fixed package",
                    path.display()
                ),
            ));
        }
        if !projection_paths.insert(path.clone()) {
            return Err(ChangeError::new(
                "ambiguous_path_projection",
                format!("multiple projections bind {}", path.display()),
            ));
        }
        let actual = package_fingerprint(&path)?;
        if actual != projection.expected_before_fingerprint {
            return Err(ChangeError::new(
                "plan_drifted",
                format!(
                    "{} expected {}, found {actual}",
                    path.display(),
                    projection.expected_before_fingerprint
                ),
            ));
        }
        path_projections.push(PathProjection {
            path,
            expected_before_fingerprint: projection.expected_before_fingerprint,
            expected_after_fingerprint: projection.expected_after_fingerprint,
        });
    }

    let digest_payload = plan_digest_payload(
        &input.scan_id,
        &input.evidence_ids,
        &operations,
        &input.roster_changes,
        &input.source_updates,
        &input.library_changes,
        &path_projections,
    )?;
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
        path_projections,
        approved_roots: roots,
        state_dir,
    })
}

pub fn apply(plan: &PreparedPlan) -> Result<ApplyOutcome> {
    validate_prepared_plan(plan)?;
    let lock = WriteLock::acquire(&plan.state_dir)?;
    apply_locked(plan, &lock._guard)
}

pub(crate) fn apply_locked(plan: &PreparedPlan, state_lock: &StateLock) -> Result<ApplyOutcome> {
    let anchored =
        state_lock.filesystem(&plan.approved_roots, &plan.state_dir, &SystemDirectorySync)?;
    apply_locked_with_anchored(plan, anchored)
}

#[cfg(test)]
fn apply_locked_with(
    plan: &PreparedPlan,
    directory_sync: &dyn DirectorySync,
) -> Result<ApplyOutcome> {
    let anchored = open_anchored_fs(&plan.approved_roots, &plan.state_dir, directory_sync)?;
    apply_locked_with_anchored(plan, anchored)
}

fn apply_locked_with_anchored(
    plan: &PreparedPlan,
    anchored: AnchoredFs<'_>,
) -> Result<ApplyOutcome> {
    validate_prepared_plan(plan)?;
    run_before_sequence_validation_hook();
    validate_path_projections_anchored(&plan.path_projections, &anchored, false)?;
    validate_operation_sequence_anchored(&plan.operations, &anchored)?;
    reject_unresolved_recovery_except(&anchored, &plan.state_dir, "")?;
    anchored
        .require_atomic_noreplace_rename()
        .map_err(|error| ChangeError::io("validate mutation filesystem", &plan.state_dir, error))?;

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
    persist_journal_with(&receipt, &anchored)?;
    run_before_mutation_hook();

    for (index, operation) in plan.operations.iter().enumerate() {
        receipt.operation_results.push(JournalOperationResult {
            id: format!("operation_{}", ulid::Ulid::new()),
            position: index as u32,
            action: operation_kind(operation).to_owned(),
            target: operation.target().to_path_buf(),
            status: "applying".to_owned(),
            before_fingerprint: anchored.fingerprint(operation.target()).ok(),
            after_fingerprint: None,
            error: None,
        });
        persist_journal_with(&receipt, &anchored)?;
        if let Some(compensation) =
            stage_compensation(operation, &receipt.id, index, &plan.state_dir, &anchored)?
        {
            receipt.compensations.push(compensation);
            persist_journal_with(&receipt, &anchored)?;
        }
        run_before_execute_hook();
        match execute(operation, &receipt.id, index, &anchored) {
            Ok(compensation) => {
                receipt.changed_paths.push(operation.target().to_path_buf());
                if let Some(compensation) = compensation {
                    receipt.compensations.push(compensation);
                }
                if let Some(result) = receipt.operation_results.last_mut() {
                    result.status = "applied".to_owned();
                    result.after_fingerprint = anchored.fingerprint(operation.target()).ok();
                }
                if let Err(journal_error) = persist_journal_with(&receipt, &anchored) {
                    receipt.error = Some(journal_error.to_string());
                    let rollback = compensate_all(&receipt, &anchored);
                    receipt.status = if rollback.is_ok() {
                        ReceiptStatus::FailedRolledBack
                    } else {
                        ReceiptStatus::RecoveryRequired
                    };
                    finalize_rollback_results(&mut receipt, rollback.is_ok(), &anchored);
                    if let Err(rollback_error) = rollback {
                        receipt.error = Some(format!(
                            "{journal_error}; compensation failed: {rollback_error}"
                        ));
                    }
                    // Best effort: if storage remains unavailable, the last durable
                    // journal stays Applying and therefore blocks future writes.
                    let _ = persist_journal_with(&receipt, &anchored);
                    return Ok(ApplyOutcome {
                        receipt,
                        verification_passed: false,
                    });
                }
            }
            Err(error) => {
                if let Some(result) = receipt.operation_results.last_mut() {
                    result.status = "failed".to_owned();
                    result.after_fingerprint = anchored.fingerprint(operation.target()).ok();
                    result.error = Some(error.to_string());
                }
                receipt.error = Some(error.to_string());
                if error.code == "entry_identity_changed"
                    || operation_is_uncompensated_create(operation)
                {
                    let rollback = compensate_all(&receipt, &anchored);
                    receipt.status = ReceiptStatus::RecoveryRequired;
                    finalize_recovery_required_results(&mut receipt, rollback.is_ok(), &anchored);
                    if let Err(rollback_error) = rollback {
                        receipt.error = Some(format!(
                            "{error}; prior-operation compensation failed: {rollback_error}"
                        ));
                    }
                    let _ = persist_journal_with(&receipt, &anchored);
                    return Ok(ApplyOutcome {
                        receipt,
                        verification_passed: false,
                    });
                }
                // A durability barrier can fail after the directory entry changed.
                // Record the concrete recovery action before compensating so the
                // mutation is never reported as a clean rollback.
                match partial_operation_compensation(operation, &receipt.id, index, &anchored) {
                    Ok(Some(compensation)) => {
                        receipt.compensations.push(compensation);
                        receipt.changed_paths.push(operation.target().to_path_buf());
                        let _ = persist_journal_with(&receipt, &anchored);
                    }
                    Ok(None) => {}
                    Err(fingerprint_error) => {
                        receipt.status = ReceiptStatus::RecoveryRequired;
                        receipt.error = Some(format!(
                            "{error}; partial target could not be fingerprinted: {fingerprint_error}"
                        ));
                        let rollback = compensate_all(&receipt, &anchored);
                        finalize_rollback_results(&mut receipt, rollback.is_ok(), &anchored);
                        if let Err(rollback_error) = rollback {
                            receipt.error = Some(format!(
                                "{error}; partial target could not be fingerprinted: {fingerprint_error}; compensation failed: {rollback_error}"
                            ));
                        }
                        let _ = persist_journal_with(&receipt, &anchored);
                        return Ok(ApplyOutcome {
                            receipt,
                            verification_passed: false,
                        });
                    }
                }
                let rollback = compensate_all(&receipt, &anchored);
                receipt.status = if rollback.is_ok() {
                    ReceiptStatus::FailedRolledBack
                } else {
                    ReceiptStatus::RecoveryRequired
                };
                finalize_rollback_results(&mut receipt, rollback.is_ok(), &anchored);
                if let Err(rollback_error) = rollback {
                    receipt.error = Some(format!("{error}; compensation failed: {rollback_error}"));
                }
                persist_journal_with(&receipt, &anchored)?;
                return Ok(ApplyOutcome {
                    receipt,
                    verification_passed: false,
                });
            }
        }
    }

    if let Err(error) = validate_path_projections_anchored(&plan.path_projections, &anchored, true)
    {
        receipt.error = Some(error.to_string());
        let rollback = compensate_all(&receipt, &anchored);
        receipt.status = if rollback.is_ok() {
            ReceiptStatus::FailedRolledBack
        } else {
            ReceiptStatus::RecoveryRequired
        };
        finalize_rollback_results(&mut receipt, rollback.is_ok(), &anchored);
        if let Err(rollback_error) = rollback {
            receipt.error = Some(format!("{error}; compensation failed: {rollback_error}"));
        }
        persist_journal_with(&receipt, &anchored)?;
        return Ok(ApplyOutcome {
            receipt,
            verification_passed: false,
        });
    }

    receipt.status = ReceiptStatus::Applied;
    if let Err(error) = persist_journal_with(&receipt, &anchored) {
        receipt.status = ReceiptStatus::RecoveryRequired;
        receipt.error = Some(format!(
            "changes were applied but the final receipt could not be persisted: {error}"
        ));
        let _ = persist_journal_with(&receipt, &anchored);
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

fn validate_path_projections_anchored(
    projections: &[PathProjection],
    anchored: &AnchoredFs<'_>,
    after_apply: bool,
) -> Result<()> {
    for projection in projections {
        let expected = if after_apply {
            &projection.expected_after_fingerprint
        } else {
            &projection.expected_before_fingerprint
        };
        let actual = anchored
            .package_fingerprint(&projection.path)
            .map_err(|error| {
                ChangeError::io(
                    "inspect package through approved root handle",
                    &projection.path,
                    error,
                )
            })?;
        if actual != *expected {
            return Err(ChangeError::new(
                if after_apply {
                    "plan_postcondition_failed"
                } else {
                    "plan_drifted"
                },
                format!(
                    "{} expected {expected}, found {actual}",
                    projection.path.display()
                ),
            ));
        }
    }
    Ok(())
}

pub fn undo(receipt: &ChangeReceipt) -> Result<ApplyOutcome> {
    if receipt.status != ReceiptStatus::Applied {
        return Err(ChangeError::new(
            "receipt_not_undoable",
            "only an applied receipt can be undone",
        ));
    }
    let lock = WriteLock::acquire(&receipt.state_dir)?;
    undo_locked(receipt, &lock._guard)
}

pub(crate) fn undo_locked(receipt: &ChangeReceipt, state_lock: &StateLock) -> Result<ApplyOutcome> {
    let anchored = state_lock.filesystem(
        &receipt.approved_roots,
        &receipt.state_dir,
        &SystemDirectorySync,
    )?;
    undo_locked_with_anchored(receipt, anchored)
}

fn undo_locked_with_anchored(
    receipt: &ChangeReceipt,
    anchored: AnchoredFs<'_>,
) -> Result<ApplyOutcome> {
    if receipt.status != ReceiptStatus::Applied {
        return Err(ChangeError::new(
            "receipt_not_undoable",
            "only an applied receipt can be undone",
        ));
    }
    reject_unresolved_recovery_except(&anchored, &receipt.state_dir, &receipt.id)?;
    anchored
        .require_atomic_noreplace_rename()
        .map_err(|error| {
            ChangeError::io("validate mutation filesystem", &receipt.state_dir, error)
        })?;
    if reverse_receipt_exists(&anchored, &receipt.state_dir, &receipt.id)? {
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
                before_fingerprint: anchored.fingerprint(&original.target).ok(),
                after_fingerprint: None,
                error: None,
            })
            .collect(),
    };
    persist_journal_with(&undo_receipt, &anchored)?;
    match compensate_all(receipt, &anchored) {
        Ok(()) => {
            for result in &mut undo_receipt.operation_results {
                result.status = "undone".to_owned();
                result.after_fingerprint = anchored.fingerprint(&result.target).ok();
            }
            undo_receipt.status = ReceiptStatus::Undone;
            undo_receipt.changed_paths = receipt.changed_paths.clone();
            persist_journal_with(&undo_receipt, &anchored)?;
            Ok(ApplyOutcome {
                receipt: undo_receipt,
                verification_passed: true,
            })
        }
        Err(error) => {
            for result in &mut undo_receipt.operation_results {
                result.status = "failed".to_owned();
                result.after_fingerprint = anchored.fingerprint(&result.target).ok();
                result.error = Some(error.to_string());
            }
            undo_receipt.status = ReceiptStatus::RecoveryRequired;
            undo_receipt.error = Some(error.to_string());
            persist_journal_with(&undo_receipt, &anchored)?;
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
    let lock = WriteLock::acquire(&receipt.state_dir)?;
    rollback_apply_locked(receipt, &lock._guard)
}

pub(crate) fn rollback_apply_locked(
    receipt: &ChangeReceipt,
    state_lock: &StateLock,
) -> Result<ApplyOutcome> {
    let anchored = state_lock.filesystem(
        &receipt.approved_roots,
        &receipt.state_dir,
        &SystemDirectorySync,
    )?;
    rollback_apply_locked_with_anchored(receipt, anchored)
}

fn rollback_apply_locked_with_anchored(
    receipt: &ChangeReceipt,
    anchored: AnchoredFs<'_>,
) -> Result<ApplyOutcome> {
    if receipt.status != ReceiptStatus::Applied {
        return Err(ChangeError::new(
            "receipt_not_rollbackable",
            "only an applied receipt can be rolled back",
        ));
    }
    let mut rolled_back = receipt.clone();
    match compensate_all(receipt, &anchored) {
        Ok(()) => {
            rolled_back.status = ReceiptStatus::FailedRolledBack;
            for result in &mut rolled_back.operation_results {
                result.status = "failed_rolled_back".to_owned();
                result.after_fingerprint = anchored.fingerprint(&result.target).ok();
            }
            persist_journal_with(&rolled_back, &anchored)?;
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
                result.after_fingerprint = anchored.fingerprint(&result.target).ok();
                result.error = Some(error.to_string());
            }
            persist_journal_with(&rolled_back, &anchored)?;
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

fn finalize_rollback_results(receipt: &mut ChangeReceipt, recovered: bool, anchored: &AnchoredFs) {
    for result in &mut receipt.operation_results {
        result.after_fingerprint = anchored.fingerprint(&result.target).ok();
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

fn finalize_recovery_required_results(
    receipt: &mut ChangeReceipt,
    prior_recovered: bool,
    anchored: &AnchoredFs,
) {
    let last = receipt.operation_results.len().saturating_sub(1);
    for (index, result) in receipt.operation_results.iter_mut().enumerate() {
        result.after_fingerprint = anchored.fingerprint(&result.target).ok();
        result.status = if index == last || !prior_recovered {
            "recovery_required".to_owned()
        } else {
            "rolled_back".to_owned()
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

fn operation_is_uncompensated_create(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::CreateDirectory { .. }
            | Operation::CreateSymlink { .. }
            | Operation::WriteFile { .. }
            | Operation::Copy { .. }
    )
}

fn move_backup_path(source: &Path, receipt_id: &str, index: usize) -> Result<PathBuf> {
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ChangeError::new("unsafe_path", source.display().to_string()))?;
    Ok(source.with_file_name(format!(
        ".{file_name}.skillroster-backup-{receipt_id}-{index}"
    )))
}

fn partial_operation_compensation(
    operation: &Operation,
    receipt_id: &str,
    index: usize,
    anchored: &AnchoredFs<'_>,
) -> Result<Option<Compensation>> {
    if let Operation::RemoveSymlink {
        target,
        expected_fingerprint,
    } = operation
    {
        let backup = move_backup_path(target, receipt_id, index)?;
        let target_fingerprint = anchored_fingerprint(anchored, target)?;
        let backup_fingerprint = anchored_fingerprint(anchored, &backup)?;
        if target_fingerprint == "missing" && backup_fingerprint == *expected_fingerprint {
            return Ok(Some(Compensation::RestoreRemovedSymlink {
                from: backup,
                to: target.clone(),
                expected_fingerprint: expected_fingerprint.clone(),
            }));
        }
        if target_fingerprint == *expected_fingerprint && backup_fingerprint == "missing" {
            return Ok(None);
        }
        return Err(ChangeError::new(
            "partial_mutation_ambiguous",
            format!(
                "removed symlink state is ambiguous: target {target_fingerprint}, recovery {backup_fingerprint}"
            ),
        ));
    }
    if !operation_creates_missing_target(operation) {
        return Ok(None);
    }
    let expected_fingerprint = anchored_fingerprint(anchored, operation.target())?;
    if expected_fingerprint == "missing" {
        return Ok(None);
    }
    if let Operation::MoveRecoverable { target, source, .. } = operation {
        let source_fingerprint = anchored_fingerprint(anchored, source)?;
        let backup = move_backup_path(source, receipt_id, index)?;
        let backup_fingerprint = anchored_fingerprint(anchored, &backup)?;
        if source_fingerprint == "missing" && backup_fingerprint != "missing" {
            return Ok(Some(Compensation::RestoreBackup {
                backup,
                original: source.clone(),
                created: target.clone(),
                expected_created: expected_fingerprint,
            }));
        }
        if source_fingerprint == "missing" {
            return Ok(Some(Compensation::RenameBack {
                from: target.clone(),
                to: source.clone(),
                expected_fingerprint,
            }));
        }
    }
    Ok(Some(Compensation::StashCreated {
        path: operation.target().to_path_buf(),
        expected_fingerprint,
    }))
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

fn validate_bootstrap_operation_target(
    operation: &Operation,
    roots: &[PathBuf],
    missing_skill_roots: &[PathBuf],
) -> Result<()> {
    let allowed = roots.iter().any(|root| {
        let target = operation.target();
        let Ok(relative) = target.strip_prefix(root) else {
            return false;
        };
        let package_target_allowed = |relative_to_skill_root: &Path| match operation {
            Operation::CreateDirectory { .. } => {
                relative_to_skill_root.as_os_str().is_empty()
                    || relative_to_skill_root == Path::new("skillroster")
                    || relative_to_skill_root == Path::new("skillroster/references")
            }
            Operation::WriteFile { .. } | Operation::ReplaceFile { .. } => {
                crate::bootstrap::is_managed_target(relative_to_skill_root)
            }
            _ => false,
        };
        let is_anchor = missing_skill_roots
            .iter()
            .any(|skill_root| skill_root != root && skill_root.starts_with(root));
        if !is_anchor && package_target_allowed(relative) {
            return true;
        }
        missing_skill_roots.iter().any(|skill_root| {
            if matches!(operation, Operation::CreateDirectory { .. })
                && target != root
                && skill_root.starts_with(target)
            {
                return true;
            }
            target
                .strip_prefix(skill_root)
                .is_ok_and(package_target_allowed)
        })
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

fn bootstrap_skill_root(target: &Path) -> Option<&Path> {
    bootstrap_skill_directory(target).and_then(Path::parent)
}

fn bootstrap_skill_directory(target: &Path) -> Option<&Path> {
    target
        .ancestors()
        .find(|ancestor| ancestor.file_name() == Some(OsStr::new("skillroster")))
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

fn validate_operation_sequence_anchored(
    operations: &[Operation],
    anchored: &AnchoredFs<'_>,
) -> Result<()> {
    let mut projected_missing = HashSet::new();
    let mut projected_present = HashSet::new();
    let mut projected_fingerprints: HashMap<PathBuf, String> = HashMap::new();
    for operation in operations {
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
                let actual_source = if let Some(actual) = projected_fingerprints.get(source) {
                    actual.clone()
                } else {
                    anchored.fingerprint(source).map_err(|error| {
                        ChangeError::io(
                            "inspect projected link source through approved root handle",
                            source,
                            error,
                        )
                    })?
                };
                if &actual_source != expected_source_fingerprint {
                    return Err(ChangeError::new(
                        "plan_drifted",
                        format!(
                            "{} expected {expected_source_fingerprint}, found {actual_source}",
                            source.display()
                        ),
                    ));
                }
            } else {
                validate_anchored_current_state(operation, anchored)?;
            }
        } else {
            validate_anchored_current_state(operation, anchored)?;
        }
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

fn open_anchored_fs<'a>(
    roots: &[PathBuf],
    state_dir: &Path,
    directory_sync: &'a dyn DirectorySync,
) -> Result<AnchoredFs<'a>> {
    AnchoredFs::open(roots, state_dir, directory_sync)
        .map_err(|error| ChangeError::io("open approved root handles", state_dir, error))
}

fn validate_anchored_current_state(operation: &Operation, anchored: &AnchoredFs) -> Result<()> {
    let (path, expected) = match operation {
        Operation::CreateDirectory {
            target,
            expected_fingerprint,
        }
        | Operation::CreateSymlink {
            target,
            expected_fingerprint,
            ..
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
        }
        | Operation::RemoveSymlink {
            target,
            expected_fingerprint,
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
    let actual = anchored
        .fingerprint(path)
        .map_err(|error| ChangeError::io("inspect through approved root handle", path, error))?;
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
        let actual = anchored.fingerprint(source).map_err(|error| {
            ChangeError::io(
                "inspect link source through approved root handle",
                source,
                error,
            )
        })?;
        if &actual != expected_source_fingerprint {
            return Err(ChangeError::new(
                "plan_drifted",
                format!(
                    "{} expected {expected_source_fingerprint}, found {actual}",
                    source.display()
                ),
            ));
        }
    }
    if let Operation::Copy { target, .. } | Operation::MoveRecoverable { target, .. } = operation {
        let target_fingerprint = anchored.fingerprint(target).map_err(|error| {
            ChangeError::io("inspect target through approved root handle", target, error)
        })?;
        if target_fingerprint != "missing" {
            return Err(ChangeError::new(
                "target_exists",
                format!("{} already exists", target.display()),
            ));
        }
    }
    Ok(())
}

fn stage_compensation(
    operation: &Operation,
    receipt_id: &str,
    index: usize,
    state_dir: &Path,
    anchored: &AnchoredFs,
) -> Result<Option<Compensation>> {
    match operation {
        Operation::ReplaceFile {
            target,
            content,
            expected_fingerprint,
        } => {
            verify_anchored_fingerprint(anchored, target, expected_fingerprint)?;
            let original_windows_security = anchored
                .original_windows_security(target)
                .map_err(|error| ChangeError::io("inspect replacement metadata", target, error))?;
            let directory = create_recovery_dir(anchored, state_dir, receipt_id)?;
            let backup = directory.join(format!("replace-{index}.backup"));
            require_anchored_missing(anchored, &backup)?;
            anchored
                .copy_private_backup(target, &backup)
                .map_err(|error| ChangeError::io("persist replacement backup", target, error))?;
            let backup_fingerprint = anchored
                .fingerprint(&backup)
                .map_err(|error| ChangeError::io("verify replacement backup", &backup, error))?;
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
                staged_replacement: Some(replacement_temp_path(target, receipt_id, index)?),
                original_windows_security,
            }))
        }
        Operation::RemoveSymlink { .. } => Ok(None),
        _ => Ok(None),
    }
}

fn replace_file(
    anchored: &AnchoredFs,
    target: &Path,
    content: &str,
    receipt_id: &str,
    index: usize,
) -> Result<()> {
    let temp = replacement_temp_path(target, receipt_id, index)?;
    require_anchored_missing(anchored, &temp)?;
    anchored
        .replace_file(target, &temp, content.as_bytes())
        .map_err(|error| anchored_mutation_error("publish replacement", target, error))
}

fn replacement_temp_path(target: &Path, receipt_id: &str, index: usize) -> Result<PathBuf> {
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ChangeError::new("unsafe_path", target.display().to_string()))?;
    Ok(target.with_file_name(format!(
        ".{file_name}.skillroster-replace-{receipt_id}-{index}"
    )))
}

fn file_content_fingerprint(content: &[u8]) -> String {
    format!("file:sha256:{}", hex::encode(Sha256::digest(content)))
}

fn execute(
    operation: &Operation,
    receipt_id: &str,
    index: usize,
    anchored: &AnchoredFs,
) -> Result<Option<Compensation>> {
    validate_anchored_current_state(operation, anchored)?;
    match operation {
        Operation::CreateDirectory { target, .. } => {
            anchored
                .create_dir(target)
                .map_err(|e| anchored_mutation_error("create directory", target, e))?;
            Ok(Some(Compensation::StashCreated {
                path: target.clone(),
                expected_fingerprint: anchored_fingerprint(anchored, target)?,
            }))
        }
        Operation::CreateSymlink { target, source, .. } => {
            anchored
                .create_symlink(source, target)
                .map_err(|e| anchored_mutation_error("create symlink", target, e))?;
            Ok(Some(Compensation::StashCreated {
                path: target.clone(),
                expected_fingerprint: anchored_fingerprint(anchored, target)?,
            }))
        }
        Operation::WriteFile {
            target, content, ..
        } => {
            anchored
                .create_file(target, content.as_bytes())
                .map_err(|e| anchored_mutation_error("create file", target, e))?;
            Ok(Some(Compensation::StashCreated {
                path: target.clone(),
                expected_fingerprint: anchored_fingerprint(anchored, target)?,
            }))
        }
        Operation::ReplaceFile {
            target, content, ..
        } => {
            replace_file(anchored, target, content, receipt_id, index)?;
            Ok(None)
        }
        Operation::RemoveSymlink {
            target,
            expected_fingerprint,
        } => {
            let backup = move_backup_path(target, receipt_id, index)?;
            require_anchored_missing(anchored, &backup)?;
            rename_removed_symlink(anchored, target, &backup, expected_fingerprint)?;
            Ok(Some(Compensation::RestoreRemovedSymlink {
                from: backup,
                to: target.clone(),
                expected_fingerprint: expected_fingerprint.clone(),
            }))
        }
        Operation::Copy { target, source, .. } => {
            anchored
                .copy_tree(source, target)
                .map_err(|e| anchored_mutation_error("copy path", source, e))?;
            Ok(Some(Compensation::StashCreated {
                path: target.clone(),
                expected_fingerprint: anchored_fingerprint(anchored, target)?,
            }))
        }
        Operation::MoveRecoverable { target, source, .. } => {
            let expected = anchored_fingerprint(anchored, source)?;
            match anchored.rename(source, target) {
                Ok(()) => {
                    return Ok(Some(Compensation::RenameBack {
                        from: target.clone(),
                        to: source.clone(),
                        expected_fingerprint: expected,
                    }));
                }
                Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {}
                Err(error) => return Err(anchored_mutation_error("move path", source, error)),
            }
            // Cross-device recovery must remain on the source filesystem, so the
            // original can be hidden atomically after the destination copy lands.
            let backup = move_backup_path(source, receipt_id, index)?;
            require_anchored_missing(anchored, &backup)?;
            anchored
                .copy_tree(source, target)
                .map_err(|e| ChangeError::io("copy path", source, e))?;
            if let Err(error) = anchored.rename(source, &backup) {
                return Err(anchored_mutation_error(
                    "move source to recovery",
                    source,
                    error,
                ));
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

fn anchored_mutation_error(action: &str, path: &Path, error: io::Error) -> ChangeError {
    if error.kind() == io::ErrorKind::InvalidData {
        ChangeError::new(
            "entry_identity_changed",
            format!("{action} {}: {error}", path.display()),
        )
    } else {
        ChangeError::io(action, path, error)
    }
}

fn rename_removed_symlink(
    anchored: &AnchoredFs<'_>,
    target: &Path,
    backup: &Path,
    expected_fingerprint: &str,
) -> Result<()> {
    verify_anchored_fingerprint(anchored, target, expected_fingerprint)?;
    run_before_remove_symlink_rename_hook();
    let rename_error = anchored.rename(target, backup).err();
    let target_fingerprint = anchored_fingerprint(anchored, target)?;
    let backup_fingerprint = match anchored_fingerprint(anchored, backup) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            if target_fingerprint == "missing" {
                anchored.rename(backup, target).map_err(|restore_error| {
                    ChangeError::new(
                        "partial_mutation_ambiguous",
                        format!(
                            "removed symlink entry could not be inspected or restored: {error}; {restore_error}"
                        ),
                    )
                })?;
            }
            return Err(ChangeError::new(
                "entry_identity_changed",
                format!("removed symlink entry could not be verified after rename: {error}"),
            ));
        }
    };
    if rename_error.is_none()
        && target_fingerprint == "missing"
        && backup_fingerprint == expected_fingerprint
    {
        return Ok(());
    }
    if target_fingerprint == "missing" && backup_fingerprint != "missing" {
        anchored.rename(backup, target).map_err(|restore_error| {
            ChangeError::new(
                "partial_mutation_ambiguous",
                format!("removed symlink entry changed and could not be restored: {restore_error}"),
            )
        })?;
    }
    Err(if backup_fingerprint != expected_fingerprint {
        ChangeError::new(
            "entry_identity_changed",
            "removed symlink entry changed before its rename completed",
        )
    } else if let Some(error) = rename_error {
        ChangeError::io("move removed symlink to recovery", target, error)
    } else {
        ChangeError::new(
            "partial_mutation_ambiguous",
            "removed symlink target changed while its rename completed",
        )
    })
}

fn compensate_all(receipt: &ChangeReceipt, anchored: &AnchoredFs) -> Result<()> {
    for (index, compensation) in receipt.compensations.iter().enumerate().rev() {
        compensate(
            compensation,
            &receipt.id,
            index,
            &receipt.state_dir,
            anchored,
        )?;
    }
    Ok(())
}

fn compensate(
    compensation: &Compensation,
    receipt_id: &str,
    index: usize,
    state_dir: &Path,
    anchored: &AnchoredFs,
) -> Result<()> {
    match compensation {
        Compensation::StashCreated {
            path,
            expected_fingerprint,
        } => {
            verify_anchored_fingerprint(anchored, path, expected_fingerprint)?;
            let stash =
                create_recovery_dir(anchored, state_dir, receipt_id)?.join(format!("undo-{index}"));
            anchored
                .rename(path, &stash)
                .map_err(|e| ChangeError::io("stash created path", path, e))?;
        }
        Compensation::RestoreSymlink { path, target } => {
            require_anchored_missing(anchored, path)?;
            anchored
                .restore_symlink_contents(target, path)
                .map_err(|e| ChangeError::io("restore symlink", path, e))?;
        }
        Compensation::RenameBack {
            from,
            to,
            expected_fingerprint,
        } => {
            verify_anchored_fingerprint(anchored, from, expected_fingerprint)?;
            require_anchored_missing(anchored, to)?;
            anchored
                .rename(from, to)
                .map_err(|e| ChangeError::io("restore moved path", from, e))?;
        }
        Compensation::RestoreRemovedSymlink {
            from,
            to,
            expected_fingerprint,
        } => {
            verify_anchored_fingerprint(anchored, from, expected_fingerprint)?;
            require_anchored_missing(anchored, to)?;
            anchored
                .rename(from, to)
                .map_err(|e| ChangeError::io("restore removed symlink", from, e))?;
        }
        Compensation::RestoreBackup {
            backup,
            original,
            created,
            expected_created,
        } => {
            verify_anchored_fingerprint(anchored, created, expected_created)?;
            verify_anchored_fingerprint(anchored, backup, expected_created)?;
            require_anchored_missing(anchored, original)?;
            let stash = create_recovery_dir(anchored, state_dir, receipt_id)?
                .join(format!("undo-created-{index}"));
            anchored
                .rename(created, &stash)
                .map_err(|e| ChangeError::io("stash copied destination", created, e))?;
            anchored
                .rename(backup, original)
                .map_err(|e| ChangeError::io("restore source backup", backup, e))?;
        }
        Compensation::RestoreReplacedFile {
            backup,
            target,
            expected_original,
            expected_replacement,
            staged_replacement,
            original_windows_security,
        } => {
            if let Some(staged_replacement) = staged_replacement {
                remove_expected_anchored_file_if_present(
                    anchored,
                    staged_replacement,
                    expected_replacement,
                    "remove staged replacement",
                )?;
            }
            let actual = anchored_fingerprint(anchored, target)?;
            if actual != "missing" {
                anchored
                    .verify_restoration_metadata(
                        backup,
                        target,
                        original_windows_security.as_deref(),
                    )
                    .map_err(|error| {
                        ChangeError::io("verify replacement restoration metadata", target, error)
                    })?;
            }
            if actual == *expected_original {
                remove_expected_anchored_file_if_present(
                    anchored,
                    backup,
                    expected_original,
                    "remove unused backup",
                )?;
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
            require_anchored_missing(anchored, &restore)?;
            anchored
                .restore_private_backup(backup, &restore, original_windows_security.as_deref())
                .map_err(|error| ChangeError::io("stage restored file", backup, error))?;
            if actual != "missing" {
                anchored
                    .remove_file(target)
                    .map_err(|error| ChangeError::io("remove replacement", target, error))?;
            }
            anchored
                .rename(&restore, target)
                .map_err(|error| ChangeError::io("restore replaced file", target, error))?;
            verify_anchored_fingerprint(anchored, target, expected_original)?;
            remove_expected_anchored_file_if_present(
                anchored,
                backup,
                expected_original,
                "remove consumed backup",
            )?;
        }
    }
    Ok(())
}

fn remove_expected_anchored_file_if_present(
    anchored: &AnchoredFs<'_>,
    path: &Path,
    expected_fingerprint: &str,
    action: &str,
) -> Result<()> {
    let actual = anchored_fingerprint(anchored, path)?;
    if actual == "missing" {
        return Ok(());
    }
    if actual != expected_fingerprint {
        return Err(ChangeError::new(
            "undo_drifted",
            format!(
                "{} expected {expected_fingerprint}, found {actual}",
                path.display()
            ),
        ));
    }
    anchored
        .remove_file(path)
        .map_err(|error| ChangeError::io(action, path, error))
}

fn plan_digest_payload(
    scan_id: &str,
    evidence_ids: &[String],
    operations: &[Operation],
    roster_changes: &[RosterChange],
    source_updates: &[SourceUpdateAction],
    library_changes: &[LibraryChangeAction],
    path_projections: &[PathProjection],
) -> Result<Vec<u8>> {
    if path_projections.is_empty() {
        serde_json::to_vec(&(
            scan_id,
            evidence_ids,
            operations,
            roster_changes,
            source_updates,
            library_changes,
        ))
    } else {
        serde_json::to_vec(&(
            scan_id,
            evidence_ids,
            operations,
            roster_changes,
            source_updates,
            library_changes,
            path_projections,
        ))
    }
    .map_err(|error| ChangeError::new("plan_encoding_failed", error.to_string()))
}

fn validate_prepared_plan(plan: &PreparedPlan) -> Result<()> {
    let payload = plan_digest_payload(
        &plan.scan_id,
        &plan.evidence_ids,
        &plan.operations,
        &plan.roster_changes,
        &plan.source_updates,
        &plan.library_changes,
        &plan.path_projections,
    )?;
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

pub fn package_fingerprint(directory: &Path) -> Result<String> {
    projected_package_fingerprint(directory, &BTreeMap::new())
}

pub fn projected_package_fingerprint(
    directory: &Path,
    regular_file_overrides: &BTreeMap<PathBuf, String>,
) -> Result<String> {
    let regular_file_overrides = regular_file_overrides
        .iter()
        .map(|(relative_path, content)| {
            normalized_relative_package_path(relative_path)
                .map(|relative_path| (relative_path, content.clone()))
                .map_err(|_| {
                    ChangeError::new(
                        "invalid_path_projection",
                        "projected file path must be a normalized relative path",
                    )
                })
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(ChangeError::io(
                "inspect projected package",
                directory,
                error,
            ));
        }
    };
    if metadata.as_ref().is_some_and(|metadata| !metadata.is_dir()) {
        return Err(ChangeError::new(
            "unsupported_file_type",
            format!("{} is not a directory", directory.display()),
        ));
    }
    if metadata.is_none() && regular_file_overrides.is_empty() {
        return Ok("missing".into());
    }

    let mut hashes = PackageHashBuilder::new();
    project_package_files(
        directory,
        Path::new(""),
        0,
        &regular_file_overrides,
        &mut hashes,
    )?;
    Ok(format!("package:sha256:{}", hashes.finish().digest))
}

fn project_package_files(
    path: &Path,
    relative_path: &Path,
    depth: usize,
    regular_file_overrides: &BTreeMap<PathBuf, String>,
    hashes: &mut PackageHashBuilder,
) -> Result<()> {
    if depth > MAX_SKILL_PACKAGE_DEPTH {
        return Err(ChangeError::new(
            "package_fingerprint_bounded",
            format!("Skill package fingerprint exceeds depth {MAX_SKILL_PACKAGE_DEPTH}"),
        ));
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(ChangeError::io("inspect projected path", path, error)),
    };
    let mut names = std::collections::BTreeSet::new();
    if metadata.is_some() {
        names.extend(
            fs::read_dir(path)
                .map_err(|error| ChangeError::io("read projected directory", path, error))?
                .map(|entry| {
                    entry
                        .map(|entry| entry.file_name())
                        .map_err(|error| ChangeError::io("read projected directory", path, error))
                })
                .collect::<Result<Vec<_>>>()?,
        );
    }
    for override_path in regular_file_overrides.keys() {
        let Ok(remainder) = override_path.strip_prefix(relative_path) else {
            continue;
        };
        if let Some(Component::Normal(name)) = remainder.components().next() {
            names.insert(name.to_os_string());
        }
    }
    for name in names {
        if ignored_package_entry_name(&name) {
            continue;
        }
        let child = path.join(&name);
        let relative_child = relative_path.join(&name);
        let relative_text = relative_child.to_str().ok_or_else(|| {
            ChangeError::new(
                "non_unicode_identity_path",
                "non-Unicode path cannot participate in stable identity",
            )
        })?;
        if let Some(content) = regular_file_overrides.get(&relative_child) {
            hashes
                .add_regular_file(relative_text, content.as_bytes())
                .map_err(|error| ChangeError::io("hash projected package", &child, error))?;
            continue;
        }
        let metadata = match fs::symlink_metadata(&child) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(ChangeError::io("inspect projected path", &child, error)),
        };
        if metadata.as_ref().is_some_and(|metadata| metadata.is_dir())
            || (metadata.is_none()
                && regular_file_overrides
                    .keys()
                    .any(|override_path| override_path.starts_with(&relative_child)))
        {
            project_package_files(
                &child,
                &relative_child,
                depth + 1,
                regular_file_overrides,
                hashes,
            )?;
        } else if metadata
            .as_ref()
            .is_some_and(|metadata| metadata.file_type().is_symlink())
        {
            let target = fs::read_link(&child)
                .map_err(|error| ChangeError::io("read projected symlink", &child, error))?;
            let target = target.to_str().ok_or_else(|| {
                ChangeError::new(
                    "non_unicode_identity_path",
                    "non-Unicode path cannot participate in stable identity",
                )
            })?;
            hashes.add_symlink(relative_text, target);
        } else if metadata.as_ref().is_some_and(|metadata| metadata.is_file()) {
            let mut bytes = Vec::new();
            File::open(&child)
                .map_err(|error| ChangeError::io("open projected package file", &child, error))?
                .take(hashes.remaining_bytes().saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|error| ChangeError::io("read projected package file", &child, error))?;
            hashes
                .add_regular_file(relative_text, &bytes)
                .map_err(|error| ChangeError::io("hash projected package", &child, error))?;
        } else if metadata.is_some() {
            return Err(ChangeError::new(
                "unsupported_file_type",
                format!("{} is not a file, directory, or symlink", child.display()),
            ));
        }
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

fn is_controlled_state_path(path: &Path, state_dir: &Path) -> bool {
    path.starts_with(state_dir.join("library")) || path.starts_with(state_dir.join("plan-backups"))
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

fn anchored_fingerprint(anchored: &AnchoredFs, path: &Path) -> Result<String> {
    anchored
        .fingerprint(path)
        .map_err(|error| ChangeError::io("inspect through approved root handle", path, error))
}

fn require_anchored_missing(anchored: &AnchoredFs, path: &Path) -> Result<()> {
    let actual = anchored_fingerprint(anchored, path)?;
    if actual != "missing" {
        return Err(ChangeError::new(
            "target_exists",
            format!("{} already exists", path.display()),
        ));
    }
    Ok(())
}

fn verify_anchored_fingerprint(anchored: &AnchoredFs, path: &Path, expected: &str) -> Result<()> {
    let actual = anchored_fingerprint(anchored, path)?;
    if actual != expected {
        return Err(ChangeError::new(
            "undo_drifted",
            format!("{} expected {expected}, found {actual}", path.display()),
        ));
    }
    Ok(())
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

#[cfg(all(test, unix))]
fn create_symlink(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target)
        .map_err(|e| ChangeError::io("create symlink", target, e))
}

#[cfg(all(test, windows))]
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

fn create_recovery_dir(
    anchored: &AnchoredFs<'_>,
    state_dir: &Path,
    receipt_id: &str,
) -> Result<PathBuf> {
    let root = state_dir.join("recovery");
    let directory = root.join(receipt_id);
    anchored
        .create_dir_all(&directory)
        .map_err(|error| ChangeError::io("create recovery directory", &directory, error))?;
    anchored
        .secure_private_directory(&root)
        .map_err(|error| ChangeError::io("secure recovery directory", &root, error))?;
    anchored
        .secure_private_directory(&directory)
        .map_err(|error| ChangeError::io("secure recovery directory", &directory, error))?;
    Ok(directory)
}

#[cfg(test)]
fn persist_journal(receipt: &ChangeReceipt) -> Result<()> {
    let anchored = open_anchored_fs(&[], &receipt.state_dir, &SystemDirectorySync)?;
    persist_journal_with(receipt, &anchored)
}

fn persist_journal_with(receipt: &ChangeReceipt, anchored: &AnchoredFs<'_>) -> Result<()> {
    let directory = receipt.state_dir.join("receipts");
    anchored
        .create_dir_all(&directory)
        .map_err(|e| ChangeError::io("create receipt directory", &directory, e))?;
    anchored
        .secure_private_directory(&directory)
        .map_err(|e| ChangeError::io("secure receipt directory", &directory, e))?;
    let path = directory.join(format!("{}.json", receipt.id));
    let temp = directory.join(format!(".{}.tmp", receipt.id));
    let bytes = serde_json::to_vec_pretty(receipt)
        .map_err(|e| ChangeError::new("receipt_encoding_failed", e.to_string()))?;
    anchored
        .write_private_file_atomic(&path, &temp, &bytes)
        .map_err(|e| ChangeError::io("publish receipt journal", &path, e))
}

pub(crate) fn persist_journal_state(receipt: &ChangeReceipt, state_lock: &StateLock) -> Result<()> {
    let anchored = state_lock.filesystem(
        &receipt.approved_roots,
        &receipt.state_dir,
        &SystemDirectorySync,
    )?;
    persist_journal_with(receipt, &anchored)
}

pub(crate) fn owned_receipt_control_file(name: &OsStr, file: &mut File) -> io::Result<bool> {
    let Some(name) = name.to_str() else {
        return Ok(false);
    };
    if let Some(id) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
        .and_then(|name| name.strip_prefix("receipt_"))
    {
        return Ok(ulid::Ulid::from_string(id).is_ok());
    }
    let Some(id) = name
        .strip_suffix(".json")
        .and_then(|name| name.strip_prefix("receipt_"))
    else {
        return Ok(false);
    };
    if ulid::Ulid::from_string(id).is_err() {
        return Ok(false);
    }
    let Ok(receipt) = serde_json::from_reader::<_, ChangeReceipt>(file) else {
        return Ok(false);
    };
    Ok(receipt.id == name.trim_end_matches(".json"))
}

/// Loads the durable filesystem journal without changing it. Invalid entries are
/// surfaced as errors because silently skipping one could permit an unsafe write.
pub fn journals(state_dir: &Path) -> Result<Vec<ChangeReceipt>> {
    let state_dir = fs::canonicalize(state_dir)
        .map_err(|error| ChangeError::io("canonicalize state directory", state_dir, error))?;
    let anchored = open_anchored_fs(&[], &state_dir, &SystemDirectorySync)?;
    journals_with(&anchored, &state_dir)
}

pub(crate) fn journals_locked(
    state_dir: &Path,
    state_lock: &StateLock,
) -> Result<Vec<ChangeReceipt>> {
    let anchored = state_lock.filesystem(&[], state_dir, &SystemDirectorySync)?;
    journals_with(&anchored, state_dir)
}

pub(crate) fn has_external_recovery_material(
    state_dir: &Path,
    state_lock: &StateLock,
) -> Result<bool> {
    let receipts = journals_locked(state_dir, state_lock)?;
    let reversed_receipts = receipts
        .iter()
        .filter(|receipt| receipt.status == ReceiptStatus::Undone)
        .filter_map(|receipt| receipt.reverses_receipt_id.clone())
        .collect::<HashSet<_>>();
    for receipt in receipts {
        let active = matches!(
            receipt.status,
            ReceiptStatus::Applying | ReceiptStatus::Applied | ReceiptStatus::RecoveryRequired
        ) && !reversed_receipts.contains(receipt.id.as_str());
        let anchored = state_lock.filesystem(
            &receipt.approved_roots,
            &receipt.state_dir,
            &SystemDirectorySync,
        )?;
        for compensation in receipt.compensations {
            let (artifact, expected) = match compensation {
                Compensation::RestoreRemovedSymlink {
                    from,
                    expected_fingerprint,
                    ..
                } if !from.starts_with(state_dir) => (from, Some(expected_fingerprint)),
                Compensation::RestoreBackup {
                    backup: from,
                    expected_created,
                    ..
                } if !from.starts_with(state_dir) => (from, Some(expected_created)),
                _ => continue,
            };
            if artifact
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
            {
                return Err(ChangeError::new(
                    "invalid_receipt_journal",
                    "recovery material path is not normalized",
                ));
            }
            let actual = anchored.fingerprint(&artifact).map_err(|error| {
                ChangeError::io("inspect Receipt-owned recovery material", &artifact, error)
            })?;
            if actual == "missing" {
                if active {
                    return Err(ChangeError::new(
                        "recovery_material_missing",
                        "active Receipt recovery material is missing",
                    ));
                }
                continue;
            }
            if expected
                .as_ref()
                .is_some_and(|expected| expected != &actual)
            {
                return Err(ChangeError::new(
                    "recovery_material_drifted",
                    "active Receipt recovery material no longer matches its fingerprint",
                ));
            }
            return Ok(true);
        }
    }
    Ok(false)
}

fn journals_with(anchored: &AnchoredFs<'_>, state_dir: &Path) -> Result<Vec<ChangeReceipt>> {
    let directory = state_dir.join("receipts");
    let entries = match anchored.read_directory_names(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ChangeError::io("read receipts", &directory, error)),
    };
    let mut receipts = Vec::new();
    for entry in entries {
        let path = directory.join(entry);
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = anchored.read_regular_file(&path).map_err(|error| {
            ChangeError::new(
                "invalid_receipt_journal",
                format!(
                    "receipt journal {} is not a regular file: {error}",
                    path.display()
                ),
            )
        })?;
        let receipt: ChangeReceipt = serde_json::from_slice(&bytes)
            .map_err(|error| ChangeError::new("invalid_receipt_journal", error.to_string()))?;
        if !anchored
            .matches_state_directory(&receipt.state_dir)
            .unwrap_or(false)
        {
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

fn reverse_receipt_exists(
    anchored: &AnchoredFs<'_>,
    state_dir: &Path,
    original_receipt_id: &str,
) -> Result<bool> {
    for existing in journals_with(anchored, state_dir)? {
        if existing.status == ReceiptStatus::Undone
            && existing.reverses_receipt_id.as_deref() == Some(original_receipt_id)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn reject_unresolved_recovery_except(
    anchored: &AnchoredFs<'_>,
    state_dir: &Path,
    except: &str,
) -> Result<()> {
    for existing in journals_with(anchored, state_dir)? {
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

pub(crate) struct StateLock {
    file: File,
    anchored: AnchoredFs<'static>,
}

impl fmt::Debug for StateLock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StateLock")
    }
}

impl StateLock {
    pub(crate) fn acquire_shared(state_dir: &Path) -> Result<Self> {
        Self::acquire(state_dir, false)
    }

    pub(crate) fn acquire_exclusive(state_dir: &Path) -> Result<Self> {
        Self::acquire(state_dir, true)
    }

    fn acquire(state_dir: &Path, exclusive: bool) -> Result<Self> {
        let path = state_dir.join("write.lock");
        let anchored = AnchoredFs::open(&[], state_dir, &SystemDirectorySync)
            .map_err(|error| ChangeError::io("open state root handle", state_dir, error))?;
        let file = anchored
            .open_private_lock(&path)
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
        Ok(Self { file, anchored })
    }

    fn filesystem<'a>(
        &self,
        approved_roots: &[PathBuf],
        state_dir: &Path,
        directory_sync: &'a dyn DirectorySync,
    ) -> Result<AnchoredFs<'a>> {
        if !self
            .anchored
            .matches_state_directory(state_dir)
            .map_err(|error| ChangeError::io("verify retained state directory", state_dir, error))?
        {
            return Err(ChangeError::new(
                "unsafe_state_directory",
                "retained lock and requested state directory differ",
            ));
        }
        AnchoredFs::open_with_retained_state(
            approved_roots,
            state_dir,
            &self.anchored,
            directory_sync,
        )
        .map_err(|error| {
            ChangeError::io(
                "extend retained state root handle",
                self.anchored_state_dir(),
                error,
            )
        })
    }

    fn anchored_state_dir(&self) -> &Path {
        self.anchored.state_root()
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
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
    use std::cell::Cell;
    #[cfg(windows)]
    use std::rc::Rc;
    use tempfile::TempDir;

    struct FailOnceDirectorySync {
        path: PathBuf,
        failed: Cell<bool>,
    }

    impl FailOnceDirectorySync {
        fn new(path: PathBuf) -> Self {
            Self {
                path,
                failed: Cell::new(false),
            }
        }
    }

    impl DirectorySync for FailOnceDirectorySync {
        fn sync_directory(&self, path: &Path) -> io::Result<()> {
            if path == self.path && !self.failed.replace(true) {
                Err(io::Error::other("injected directory sync failure"))
            } else {
                SystemDirectorySync.sync_directory(path)
            }
        }
    }

    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let state = temp.path().join("state");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&state).unwrap();
        (
            temp,
            fs::canonicalize(root).unwrap(),
            fs::canonicalize(state).unwrap(),
        )
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
    fn projected_package_fingerprint_matches_the_materialized_tree() {
        let (_temp, root, _state) = fixture();
        let package = root.join("skillroster");
        fs::create_dir(&package).unwrap();
        fs::write(package.join("notes.local.md"), "preserved\n").unwrap();
        let overrides = BTreeMap::from([
            (PathBuf::from("SKILL.md"), "managed entrypoint\n".into()),
            (
                PathBuf::from("references/routing.md"),
                "managed reference\n".into(),
            ),
        ]);

        let projected = projected_package_fingerprint(&package, &overrides).unwrap();
        fs::create_dir(package.join("references")).unwrap();
        for (relative_path, content) in overrides {
            fs::write(package.join(relative_path), content).unwrap();
        }

        assert_eq!(projected, package_fingerprint(&package).unwrap());
    }

    #[test]
    fn package_projection_rejects_content_beyond_the_scan_byte_bound() {
        let (_temp, root, _state) = fixture();
        let package = root.join("skillroster");
        fs::create_dir(&package).unwrap();
        File::create(package.join("oversized.bin"))
            .unwrap()
            .set_len(crate::package_fingerprint::MAX_SKILL_PACKAGE_BYTES + 1)
            .unwrap();

        let error = package_fingerprint(&package).unwrap_err();

        assert!(error.to_string().contains("byte fingerprint safety limit"));
    }

    #[test]
    fn bootstrap_apply_rolls_back_when_package_projection_drifts_during_apply() {
        let (_temp, root, state) = fixture();
        let package = root.join("skillroster");
        fs::create_dir(&package).unwrap();
        let extra = package.join("notes.local.md");
        fs::write(&extra, "before\n").unwrap();
        let target = package.join("SKILL.md");
        let overrides = BTreeMap::from([(PathBuf::from("SKILL.md"), "managed\n".into())]);
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "write_file",
                "target": target,
                "content": "managed\n",
                "expected_fingerprint": "missing"
            }],
            "path_projections": [{
                "path": package,
                "expected_before_fingerprint": package_fingerprint(&package).unwrap(),
                "expected_after_fingerprint": projected_package_fingerprint(&package, &overrides).unwrap()
            }]
        })
        .to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::BootstrapSetup {
                    missing_skill_roots: vec![],
                },
            },
        )
        .unwrap();
        BEFORE_EXECUTE_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::write(extra, "after\n").unwrap();
            }));
        });

        let outcome = apply(&plan).unwrap();

        assert!(!outcome.verification_passed);
        assert_eq!(outcome.receipt.status, ReceiptStatus::FailedRolledBack);
        assert!(
            outcome
                .receipt
                .error
                .as_deref()
                .is_some_and(|error| error.contains("plan_postcondition_failed"))
        );
        assert!(!target.exists());
        assert_eq!(
            fs::read_to_string(package.join("notes.local.md")).unwrap(),
            "after\n"
        );
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
                state_dir: state.clone(),
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "path_escapes_through_symlink");
    }

    #[cfg(unix)]
    #[test]
    fn apply_cannot_follow_an_ancestor_swapped_after_root_handles_open() {
        let (temp, root, state) = fixture();
        let ancestor = root.join("mutable");
        let held_ancestor = root.join("held-mutable");
        let outside = temp.path().join("outside");
        fs::create_dir(&ancestor).unwrap();
        fs::create_dir(&outside).unwrap();
        let target = ancestor.join("written.txt");
        let external_target = outside.join("written.txt");
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "write_file",
                "target": target,
                "content": "must stay contained",
                "expected_fingerprint": "missing"
            }]
        })
        .to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root.clone()],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let hook_ancestor = ancestor.clone();
        let hook_held = held_ancestor.clone();
        let hook_outside = outside.clone();
        BEFORE_MUTATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&hook_ancestor, &hook_held).unwrap();
                std::os::unix::fs::symlink(&hook_outside, &hook_ancestor).unwrap();
            }));
        });

        let outcome = apply(&plan).unwrap();

        assert!(!outcome.verification_passed);
        assert!(!external_target.exists());
        assert!(!held_ancestor.join("written.txt").exists());
        fs::remove_file(&ancestor).unwrap();
        fs::rename(&held_ancestor, &ancestor).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn apply_sequence_validation_uses_the_retained_root_handle() {
        let (temp, root, state) = fixture();
        let ancestor = root.join("mutable");
        let held_ancestor = root.join("held-mutable");
        let outside = temp.path().join("outside");
        fs::create_dir(&ancestor).unwrap();
        fs::create_dir(&outside).unwrap();
        let target = ancestor.join("written.txt");
        let plan = prepared_write(&root, &state, &target);
        let hook_ancestor = ancestor.clone();
        let hook_held = held_ancestor.clone();
        let hook_outside = outside.clone();
        BEFORE_SEQUENCE_VALIDATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&hook_ancestor, &hook_held).unwrap();
                std::os::unix::fs::symlink(&hook_outside, &hook_ancestor).unwrap();
            }));
        });

        let error = apply(&plan).unwrap_err();

        assert_eq!(error.code, "filesystem_error");
        assert!(!outside.join("written.txt").exists());
        assert!(!held_ancestor.join("written.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn apply_rejects_an_approved_root_ancestor_swapped_before_handle_open() {
        let temp = TempDir::new().unwrap();
        let root_parent = temp.path().join("root-parent");
        let held_parent = temp.path().join("held-root-parent");
        let outside_parent = temp.path().join("outside-root-parent");
        let root = root_parent.join("root");
        let outside_root = outside_parent.join("root");
        let state = temp.path().join("state");
        fs::create_dir(&root_parent).unwrap();
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside_parent).unwrap();
        fs::create_dir(&outside_root).unwrap();
        fs::create_dir(&state).unwrap();
        let target = root.join("written.txt");
        let plan = prepared_write(&root, &state, &target);
        let hook_root_parent = root_parent.clone();
        let hook_held_parent = held_parent.clone();
        let hook_outside_parent = outside_parent.clone();
        crate::anchored_fs::set_before_approved_root_open_hook(move || {
            fs::rename(&hook_root_parent, &hook_held_parent).unwrap();
            std::os::unix::fs::symlink(&hook_outside_parent, &hook_root_parent).unwrap();
        });

        let error = apply(&plan).unwrap_err();

        assert_eq!(error.code, "filesystem_error");
        assert!(!outside_root.join("written.txt").exists());
        assert!(!held_parent.join("root/written.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn approved_root_walk_rejects_a_swap_after_canonical_validation() {
        let temp = TempDir::new().unwrap();
        let root_parent = fs::canonicalize(temp.path()).unwrap().join("root-parent");
        let held_parent = fs::canonicalize(temp.path())
            .unwrap()
            .join("held-root-parent");
        let outside_parent = fs::canonicalize(temp.path())
            .unwrap()
            .join("outside-root-parent");
        let root = root_parent.join("root");
        let outside_root = outside_parent.join("root");
        let state = fs::canonicalize(temp.path()).unwrap().join("state");
        fs::create_dir(&root_parent).unwrap();
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside_parent).unwrap();
        fs::create_dir(&outside_root).unwrap();
        fs::create_dir(&state).unwrap();
        let target = root.join("written.txt");
        let plan = prepared_write(&root, &state, &target);
        let hook_root_parent = root_parent.clone();
        let hook_held_parent = held_parent.clone();
        let hook_outside_parent = outside_parent.clone();
        crate::anchored_fs::set_after_approved_root_canonicalize_hook(move || {
            fs::rename(&hook_root_parent, &hook_held_parent).unwrap();
            std::os::unix::fs::symlink(&hook_outside_parent, &hook_root_parent).unwrap();
        });

        let error = apply(&plan).unwrap_err();

        assert_eq!(error.code, "filesystem_error");
        assert!(!outside_root.join("written.txt").exists());
        assert!(!held_parent.join("root/written.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn apply_keeps_journaling_through_the_retained_state_handle() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let state_parent = temp.path().join("state-parent");
        let state = state_parent.join("state");
        let held_parent = temp.path().join("held-state-parent");
        let outside_parent = temp.path().join("outside-state-parent");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&state_parent).unwrap();
        fs::create_dir(&state).unwrap();
        fs::create_dir(&outside_parent).unwrap();
        fs::create_dir(outside_parent.join("state")).unwrap();
        let target = root.join("written.txt");
        let plan = prepared_write(&root, &state, &target);
        let hook_state_parent = state_parent.clone();
        let hook_held_parent = held_parent.clone();
        let hook_outside_parent = outside_parent.clone();
        BEFORE_MUTATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&hook_state_parent, &hook_held_parent).unwrap();
                std::os::unix::fs::symlink(&hook_outside_parent, &hook_state_parent).unwrap();
            }));
        });

        let outcome = apply(&plan).unwrap();

        assert!(outcome.verification_passed, "{:?}", outcome.receipt.error);
        assert!(target.exists());
        assert!(
            held_parent
                .join("state/receipts")
                .join(format!("{}.json", outcome.receipt.id))
                .exists()
        );
        assert!(!outside_parent.join("state/receipts").exists());
    }

    #[cfg(unix)]
    #[test]
    fn compensation_uses_retained_handles_after_an_unrelated_ancestor_swap() {
        let (temp, root, state) = fixture();
        let mutable = root.join("mutable");
        let held_mutable = root.join("held-mutable");
        let outside = temp.path().join("outside");
        fs::create_dir(&mutable).unwrap();
        fs::create_dir(&outside).unwrap();
        let stable_target = root.join("stable.txt");
        let swapped_target = mutable.join("must-not-escape.txt");
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [
                {
                    "kind": "write_file",
                    "target": stable_target,
                    "content": "first mutation",
                    "expected_fingerprint": "missing"
                },
                {
                    "kind": "write_file",
                    "target": swapped_target,
                    "content": "must stay contained",
                    "expected_fingerprint": "missing"
                }
            ]
        })
        .to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root.clone()],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let hook_mutable = mutable.clone();
        let hook_held = held_mutable.clone();
        let hook_outside = outside.clone();
        BEFORE_MUTATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&hook_mutable, &hook_held).unwrap();
                std::os::unix::fs::symlink(&hook_outside, &hook_mutable).unwrap();
            }));
        });

        let outcome = apply(&plan).unwrap();

        assert!(!outcome.verification_passed);
        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert!(!stable_target.exists());
        assert!(!outside.join("must-not-escape.txt").exists());
        assert!(!held_mutable.join("must-not-escape.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_creation_rejects_a_source_ancestor_swap_after_open() {
        let (temp, root, state) = fixture();
        let source_parent = root.join("source-parent");
        let held_parent = root.join("held-source-parent");
        let outside_parent = temp.path().join("outside-source-parent");
        fs::create_dir(&source_parent).unwrap();
        fs::create_dir(&outside_parent).unwrap();
        let source = source_parent.join("source");
        let outside_source = outside_parent.join("source");
        let target = root.join("link");
        fs::write(&source, "approved").unwrap();
        fs::write(&outside_source, "external").unwrap();
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "create_symlink",
                "source": source,
                "target": target,
                "expected_fingerprint": "missing",
                "expected_source_fingerprint": fingerprint(&source).unwrap()
            }]
        })
        .to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state.clone(),
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let hook_source_parent = source_parent.clone();
        let hook_held_parent = held_parent.clone();
        let hook_outside_parent = outside_parent.clone();
        crate::anchored_fs::set_after_symlink_source_open_hook(move || {
            fs::rename(&hook_source_parent, &hook_held_parent).unwrap();
            std::os::unix::fs::symlink(&hook_outside_parent, &hook_source_parent).unwrap();
        });

        let outcome = apply(&plan).unwrap();

        assert!(!outcome.verification_passed);
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_publication_cannot_follow_a_swapped_target_ancestor() {
        let (temp, root, state) = fixture();
        let mutable = root.join("mutable");
        let held_mutable = root.join("held-mutable");
        let outside = temp.path().join("outside");
        let source = root.join("source");
        let target = mutable.join("link");
        let outside_link = outside.join("link");
        fs::create_dir(&mutable).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(&source, "approved").unwrap();
        std::os::unix::fs::symlink(&source, &outside_link).unwrap();
        let plan = prepare(
            &serde_json::json!({
                "schema_version": 1,
                "scan_id": "scan_1",
                "operations": [{
                    "kind": "create_symlink",
                    "source": source,
                    "target": target,
                    "expected_fingerprint": "missing",
                    "expected_source_fingerprint": fingerprint(&source).unwrap(),
                }],
            })
            .to_string(),
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let hook_mutable = mutable.clone();
        let hook_held = held_mutable.clone();
        let hook_outside = outside.clone();
        BEFORE_EXECUTE_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                crate::anchored_fs::set_after_created_symlink_hook(move || {
                    fs::rename(&hook_mutable, &hook_held).unwrap();
                    std::os::unix::fs::symlink(&hook_outside, &hook_mutable).unwrap();
                });
            }));
        });

        let outcome = apply(&plan).unwrap();

        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_link(&outside_link).unwrap(), source);
        assert!(
            fs::symlink_metadata(held_mutable.join("link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
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
        let (_temp, home, state) = fixture();
        let root = home.join(".codex/skills");
        fs::create_dir_all(&root).unwrap();
        let library = state.join("library");
        let plan_backups = state.join("plan-backups");
        fs::create_dir(&library).unwrap();
        fs::create_dir(&plan_backups).unwrap();
        let context = PrepareContext {
            approved_roots: vec![root.clone(), library.clone(), plan_backups.clone()],
            state_dir: state,
            operation_policy: OperationPolicy::BootstrapSetup {
                missing_skill_roots: Vec::new(),
            },
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

    #[test]
    fn bootstrap_home_anchor_rejects_state_inside_the_fixed_skill_root() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let state = home.join(".codex/skills/state");
        fs::create_dir_all(&state).unwrap();
        let context = PrepareContext {
            approved_roots: vec![home.clone()],
            state_dir: state,
            operation_policy: OperationPolicy::BootstrapSetup {
                missing_skill_roots: vec![home.join(".codex/skills")],
            },
        };
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "write_file",
                "target": home.join(".codex/skills/skillroster/SKILL.md"),
                "content": "bootstrap",
                "expected_fingerprint": "missing"
            }]
        })
        .to_string();

        assert_eq!(
            prepare(&input, &context).unwrap_err().code,
            "unsafe_state_directory"
        );
    }

    #[test]
    fn bootstrap_home_anchor_rejects_an_undetected_agent_root() {
        let (_temp, home, state) = fixture();
        let context = PrepareContext {
            approved_roots: vec![home.clone()],
            state_dir: state,
            operation_policy: OperationPolicy::BootstrapSetup {
                missing_skill_roots: vec![home.join(".codex/skills")],
            },
        };
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "write_file",
                "target": home.join(".claude/skills/skillroster/SKILL.md"),
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
            operation_policy: OperationPolicy::BootstrapSetup {
                missing_skill_roots: Vec::new(),
            },
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
                state_dir: state.clone(),
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        fs::write(&source, "after").unwrap();
        assert_eq!(apply(&plan).unwrap_err().code, "plan_drifted");
    }

    fn prepared_write(root: &Path, state: &Path, target: &Path) -> PreparedPlan {
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "write_file",
                "target": target,
                "content": "published before injected barrier failure",
                "expected_fingerprint": "missing"
            }]
        })
        .to_string();
        prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root.to_path_buf()],
                state_dir: state.to_path_buf(),
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap()
    }

    #[test]
    fn journal_directory_sync_failure_precedes_governed_mutation() {
        let (_temp, root, state) = fixture();
        let target = root.join("not-created.md");
        let plan = prepared_write(&root, &state, &target);
        let directory_sync = FailOnceDirectorySync::new(plan.state_dir.join("receipts"));

        let error = apply_locked_with(&plan, &directory_sync).unwrap_err();

        assert_eq!(error.code, "filesystem_error");
        assert!(directory_sync.failed.get());
        assert!(!target.exists());
    }

    #[test]
    fn storage_errno_failures_keep_a_durable_recovery_boundary() {
        struct StorageError {
            path: PathBuf,
            errno: i32,
            persistent: bool,
            failed: Cell<bool>,
        }
        impl DirectorySync for StorageError {
            fn sync_directory(&self, path: &Path) -> io::Result<()> {
                if path == self.path && (self.persistent || !self.failed.get()) {
                    self.failed.set(true);
                    Err(io::Error::from_raw_os_error(self.errno))
                } else {
                    SystemDirectorySync.sync_directory(path)
                }
            }
        }
        #[cfg(unix)]
        let errors = [libc::ENOSPC, libc::EIO];
        #[cfg(windows)]
        let errors = [112, 1117]; // ERROR_DISK_FULL, ERROR_IO_DEVICE
        for errno in errors {
            for persistent in [false, true] {
                for at_journal in [false, true] {
                    let (_temp, root, state) = fixture();
                    let target = root.join("target.md");
                    let plan = prepared_write(&root, &state, &target);
                    let sync = StorageError {
                        path: if at_journal {
                            state.join("receipts")
                        } else {
                            root.clone()
                        },
                        errno,
                        persistent,
                        failed: Cell::new(false),
                    };
                    let outcome = apply_locked_with(&plan, &sync);
                    assert!(sync.failed.get());
                    if at_journal {
                        assert_eq!(outcome.unwrap_err().code, "filesystem_error");
                        assert!(!target.exists());
                    } else {
                        let outcome = outcome.unwrap();
                        assert!(!outcome.verification_passed);
                        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
                        assert!(
                            outcome
                                .receipt
                                .error
                                .unwrap()
                                .contains(&format!("os error {errno}"))
                        );
                        assert_eq!(
                            fs::read_to_string(&target).unwrap(),
                            "published before injected barrier failure"
                        );
                    }
                    let receipts = journals(&state).unwrap();
                    assert_eq!(receipts.len(), 1);
                    assert!(matches!(
                        receipts[0].status,
                        ReceiptStatus::Applying | ReceiptStatus::RecoveryRequired
                    ));
                    let next = prepared_write(&root, &state, &root.join("next.md"));
                    assert_eq!(apply(&next).unwrap_err().code, "recovery_required");
                    assert!(!root.join("next.md").exists());
                }
            }
        }
    }

    // Subprocess entrypoint: these environment keys are read only by this test
    // binary, never by the shipped CLI. Each child receives one disposable root.
    #[test]
    fn recovery_process_child() {
        let Some(base) = std::env::var_os("SKILLROSTER_RECOVERY_TEST_ROOT") else {
            return;
        };
        let base = PathBuf::from(base);
        let root = base.join("root");
        let state = base.join("state");
        let role = std::env::var("SKILLROSTER_RECOVERY_TEST_ROLE").unwrap();
        if role == "restart" {
            let receipts = journals(&state).unwrap();
            assert_eq!(receipts.len(), 1);
            assert_eq!(receipts[0].status, ReceiptStatus::Applying);
            let next = prepared_write(&root, &state, &root.join("next.md"));
            assert_eq!(apply(&next).unwrap_err().code, "recovery_required");
            assert!(!root.join("next.md").exists());
            return;
        }
        let marker = base.join("checkpoint");
        let stop = move || {
            fs::write(marker, b"ready").unwrap();
            // Parent kills this process; no destructor or compensation runs.
            loop {
                std::thread::park();
            }
        };
        match role.as_str() {
            "journal" => {
                BEFORE_MUTATION_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(stop)))
            }
            "target" => {
                struct StopAfterTargetSync {
                    root: PathBuf,
                    stop: std::cell::RefCell<Option<Box<dyn FnOnce()>>>,
                }
                impl DirectorySync for StopAfterTargetSync {
                    fn sync_directory(&self, path: &Path) -> io::Result<()> {
                        SystemDirectorySync.sync_directory(path)?;
                        if path == self.root {
                            if let Some(stop) = self.stop.borrow_mut().take() {
                                stop();
                            }
                        }
                        Ok(())
                    }
                }
                let sync = StopAfterTargetSync {
                    root: root.clone(),
                    stop: std::cell::RefCell::new(Some(Box::new(stop))),
                };
                let plan = prepared_write(&root, &state, &root.join("target.md"));
                let _lock = WriteLock::acquire(&state).unwrap();
                apply_locked_with(&plan, &sync).unwrap();
                panic!("target checkpoint was not reached");
            }
            _ => panic!("unknown subprocess role"),
        }
        let plan = prepared_write(&root, &state, &root.join("target.md"));
        apply(&plan).unwrap();
        panic!("journal checkpoint was not reached");
    }

    #[test]
    fn killed_apply_blocks_new_writes_after_process_restart() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        for checkpoint in ["journal", "target"] {
            let (temp, root, state) = fixture();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "change::tests::recovery_process_child",
                    "--nocapture",
                ])
                .env(
                    "SKILLROSTER_RECOVERY_TEST_ROOT",
                    fs::canonicalize(temp.path()).unwrap(),
                )
                .env("SKILLROSTER_RECOVERY_TEST_ROLE", checkpoint)
                .stdout(Stdio::null());
            let mut child = command.spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            while !temp.path().join("checkpoint").exists() {
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("child exited before {checkpoint}: {status}");
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("child did not reach {checkpoint}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            child.kill().unwrap();
            let status = child.wait().unwrap();
            assert!(!status.success());
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                assert_eq!(status.signal(), Some(libc::SIGKILL));
            }
            assert_eq!(root.join("target.md").exists(), checkpoint == "target");
            let before = journals(&state).unwrap();
            assert_eq!(before.len(), 1);
            let receipt_path = state
                .join("receipts")
                .join(format!("{}.json", before[0].id));
            let journal_bytes = fs::read(&receipt_path).unwrap();
            command.env("SKILLROSTER_RECOVERY_TEST_ROLE", "restart");
            let status = command.status().unwrap();
            assert!(status.success());
            assert_eq!(fs::read(receipt_path).unwrap(), journal_bytes);
            if checkpoint == "target" {
                assert_eq!(
                    fs::read_to_string(root.join("target.md")).unwrap(),
                    "published before injected barrier failure"
                );
            }
        }
    }

    #[test]
    fn unsupported_atomic_rename_is_rejected_before_mutation_or_receipt() {
        let (_temp, root, state) = fixture();
        let target = root.join("not-created.md");
        let plan = prepared_write(&root, &state, &target);
        crate::anchored_fs::force_atomic_noreplace_unsupported_once();

        let error = apply(&plan).unwrap_err();

        assert_eq!(error.code, "filesystem_error");
        assert!(
            error
                .message
                .contains("atomic no-replace rename is unavailable")
        );
        assert!(!target.exists());
        assert!(!state.join("receipts").exists());
    }

    #[test]
    fn journal_reuses_an_owned_temp_left_by_an_interrupted_write() {
        let (_temp, root, state) = fixture();
        let receipt = ChangeReceipt {
            id: format!("receipt_{}", ulid::Ulid::new()),
            plan_id: "plan_retry".to_owned(),
            status: ReceiptStatus::Applying,
            changed_paths: Vec::new(),
            compensations: Vec::new(),
            approved_roots: vec![root],
            state_dir: state.clone(),
            error: None,
            reverses_receipt_id: None,
            operation_results: Vec::new(),
        };
        let receipts = state.join("receipts");
        fs::create_dir(&receipts).unwrap();
        let temp = receipts.join(format!(".{}.tmp", receipt.id));
        fs::write(&temp, b"incomplete").unwrap();

        persist_journal(&receipt).unwrap();

        assert!(!temp.exists());
        let published = receipts.join(format!("{}.json", receipt.id));
        let decoded: ChangeReceipt = serde_json::from_slice(&fs::read(published).unwrap()).unwrap();
        assert_eq!(decoded.id, receipt.id);
    }

    #[test]
    fn journal_temp_symlink_never_truncates_its_target() {
        let (temp_dir, root, state) = fixture();
        let receipt = ChangeReceipt {
            id: format!("receipt_{}", ulid::Ulid::new()),
            plan_id: "plan_retry".to_owned(),
            status: ReceiptStatus::Applying,
            changed_paths: Vec::new(),
            compensations: Vec::new(),
            approved_roots: vec![root],
            state_dir: state.clone(),
            error: None,
            reverses_receipt_id: None,
            operation_results: Vec::new(),
        };
        let receipts = state.join("receipts");
        fs::create_dir(&receipts).unwrap();
        let outside = temp_dir.path().join("outside");
        fs::write(&outside, "must survive").unwrap();
        let temp = receipts.join(format!(".{}.tmp", receipt.id));
        create_symlink(&outside, &temp).unwrap();

        let error = persist_journal(&receipt).unwrap_err();

        assert_eq!(error.code, "filesystem_error");
        assert_eq!(fs::read_to_string(outside).unwrap(), "must survive");
    }

    #[cfg(unix)]
    #[test]
    fn journal_never_publishes_a_temp_entry_swapped_after_open() {
        let (_temp, root, state) = fixture();
        let receipt = ChangeReceipt {
            id: format!("receipt_{}", ulid::Ulid::new()),
            plan_id: "plan_retry".to_owned(),
            status: ReceiptStatus::Applying,
            changed_paths: Vec::new(),
            compensations: Vec::new(),
            approved_roots: vec![root],
            state_dir: state.clone(),
            error: None,
            reverses_receipt_id: None,
            operation_results: Vec::new(),
        };
        let receipts = state.join("receipts");
        fs::create_dir(&receipts).unwrap();
        let temp = receipts.join(format!(".{}.tmp", receipt.id));
        let held = receipts.join(format!(".{}.held", receipt.id));
        let hook_temp = temp.clone();
        crate::anchored_fs::set_after_staging_file_open_hook(move || {
            fs::rename(&hook_temp, &held).unwrap();
            fs::write(&hook_temp, "decoy").unwrap();
        });

        let error = persist_journal(&receipt).unwrap_err();

        assert_eq!(error.code, "filesystem_error");
        assert!(!receipts.join(format!("{}.json", receipt.id)).exists());
        assert_eq!(fs::read_to_string(temp).unwrap(), "decoy");
    }

    #[test]
    fn owned_receipt_validation_has_no_smaller_limit_than_receipt_creation() {
        let (_temp, root, state) = fixture();
        let receipt = ChangeReceipt {
            id: format!("receipt_{}", ulid::Ulid::new()),
            plan_id: "plan_large_receipt".to_owned(),
            status: ReceiptStatus::RecoveryRequired,
            changed_paths: Vec::new(),
            compensations: Vec::new(),
            approved_roots: vec![root],
            state_dir: state.clone(),
            error: Some("x".repeat(9 * 1024 * 1024)),
            reverses_receipt_id: None,
            operation_results: Vec::new(),
        };
        let name = format!("{}.json", receipt.id);
        let path = state.join(&name);
        fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        let mut file = File::open(path).unwrap();

        assert!(owned_receipt_control_file(OsStr::new(&name), &mut file).unwrap());
    }

    #[test]
    fn target_directory_sync_failure_requires_recovery_without_path_compensation() {
        let (_temp, root, state) = fixture();
        let target = root.join("rolled-back.md");
        let plan = prepared_write(&root, &state, &target);
        let directory_sync = FailOnceDirectorySync::new(plan.approved_roots[0].clone());

        let outcome = apply_locked_with(&plan, &directory_sync).unwrap();

        assert!(!outcome.verification_passed);
        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert!(directory_sync.failed.get());
        assert_eq!(
            fs::read_to_string(target).unwrap(),
            "published before injected barrier failure"
        );
    }

    #[cfg(unix)]
    #[test]
    fn sync_failure_never_compensates_an_entry_swapped_after_its_first_check() {
        for kind in ["write_file", "create_directory", "copy", "create_symlink"] {
            let (_temp, root, state) = fixture();
            let source = root.join("source");
            let target = root.join("target");
            let held_target = root.join("held-target");
            fs::write(&source, "approved").unwrap();
            let operation = match kind {
                "write_file" => serde_json::json!({
                    "kind": kind,
                    "target": target,
                    "content": "approved",
                    "expected_fingerprint": "missing",
                }),
                "create_directory" => serde_json::json!({
                    "kind": kind,
                    "target": target,
                    "expected_fingerprint": "missing",
                }),
                "copy" => serde_json::json!({
                    "kind": kind,
                    "source": source,
                    "target": target,
                    "expected_fingerprint": fingerprint(&source).unwrap(),
                }),
                "create_symlink" => serde_json::json!({
                    "kind": kind,
                    "source": source,
                    "target": target,
                    "expected_fingerprint": "missing",
                    "expected_source_fingerprint": fingerprint(&source).unwrap(),
                }),
                _ => unreachable!(),
            };
            let plan = prepare(
                &serde_json::json!({
                    "schema_version": 1,
                    "scan_id": "scan_1",
                    "operations": [operation],
                })
                .to_string(),
                &PrepareContext {
                    approved_roots: vec![root],
                    state_dir: state,
                    operation_policy: OperationPolicy::TestOnly,
                },
            )
            .unwrap();
            let hook_target = target.clone();
            let hook_held_target = held_target.clone();
            BEFORE_EXECUTE_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    let swap = move || {
                        fs::rename(&hook_target, &hook_held_target).unwrap();
                        if kind == "create_directory" {
                            fs::create_dir(&hook_target).unwrap();
                        } else {
                            fs::write(&hook_target, "external decoy").unwrap();
                        }
                    };
                    if kind == "create_symlink" {
                        crate::anchored_fs::set_after_created_symlink_first_check_hook(swap);
                    } else {
                        crate::anchored_fs::set_after_created_entry_first_check_hook(swap);
                    }
                }));
            });
            let directory_sync = FailOnceDirectorySync::new(plan.approved_roots[0].clone());

            let outcome = apply_locked_with(&plan, &directory_sync).unwrap();

            assert_eq!(
                outcome.receipt.status,
                ReceiptStatus::RecoveryRequired,
                "{kind}: {:?}",
                outcome.receipt.error
            );
            assert!(target.exists());
            assert!(held_target.exists());
            if kind != "create_directory" {
                assert_eq!(fs::read_to_string(&target).unwrap(), "external decoy");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn move_sync_failure_never_compensates_a_swapped_destination() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("target");
        let held_target = root.join("held-target");
        fs::write(&source, "approved").unwrap();
        let plan = prepare(
            &serde_json::json!({
                "schema_version": 1,
                "scan_id": "scan_1",
                "operations": [{
                    "kind": "move_recoverable",
                    "source": source,
                    "target": target,
                    "expected_fingerprint": fingerprint(&source).unwrap(),
                }],
            })
            .to_string(),
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let hook_target = target.clone();
        let hook_held_target = held_target.clone();
        BEFORE_EXECUTE_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                crate::anchored_fs::set_after_renamed_entry_first_check_hook(move || {
                    fs::rename(&hook_target, &hook_held_target).unwrap();
                    fs::write(&hook_target, "external decoy").unwrap();
                });
            }));
        });
        let directory_sync = FailOnceDirectorySync::new(plan.approved_roots[0].clone());

        let outcome = apply_locked_with(&plan, &directory_sync).unwrap();

        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_to_string(target).unwrap(), "external decoy");
        assert_eq!(fs::read_to_string(held_target).unwrap(), "approved");
        assert!(!source.exists());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn move_never_replaces_a_destination_created_after_validation() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("target");
        fs::write(&source, "approved").unwrap();
        let plan = prepare(
            &serde_json::json!({
                "schema_version": 1,
                "scan_id": "scan_1",
                "operations": [{
                    "kind": "move_recoverable",
                    "source": source,
                    "target": target,
                    "expected_fingerprint": fingerprint(&source).unwrap(),
                }],
            })
            .to_string(),
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let hook_target = target.clone();
        BEFORE_EXECUTE_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                crate::anchored_fs::set_before_rename_noreplace_hook(move || {
                    fs::write(&hook_target, "external decoy").unwrap();
                });
            }));
        });

        let outcome = apply(&plan).unwrap();

        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert!(
            outcome
                .receipt
                .error
                .as_deref()
                .is_some_and(|error| error.contains("entry_identity_changed")),
            "{:?}",
            outcome.receipt.error
        );
        assert_eq!(fs::read_to_string(source).unwrap(), "approved");
        assert_eq!(fs::read_to_string(target).unwrap(), "external decoy");
    }

    #[cfg(unix)]
    #[test]
    fn replacement_never_reports_applied_after_its_published_entry_is_swapped() {
        let (_temp, root, state) = fixture();
        let target = root.join("target");
        let held_target = root.join("held-target");
        fs::write(&target, "before").unwrap();
        let plan = prepare(
            &serde_json::json!({
                "schema_version": 1,
                "scan_id": "scan_1",
                "operations": [{
                    "kind": "replace_file",
                    "target": target,
                    "content": "after",
                    "expected_fingerprint": fingerprint(&target).unwrap(),
                }],
            })
            .to_string(),
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let hook_target = target.clone();
        let hook_held_target = held_target.clone();
        BEFORE_EXECUTE_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                crate::anchored_fs::set_after_renamed_entry_first_check_hook(move || {
                    fs::rename(&hook_target, &hook_held_target).unwrap();
                    fs::write(&hook_target, "external decoy").unwrap();
                });
            }));
        });

        let outcome = apply(&plan).unwrap();

        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_to_string(target).unwrap(), "external decoy");
        assert_eq!(fs::read_to_string(held_target).unwrap(), "after");
    }

    #[test]
    fn moved_path_sync_failure_requires_recovery_without_path_compensation() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("target");
        fs::write(&source, "original").unwrap();
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "move_recoverable",
                "source": source,
                "target": target,
                "expected_fingerprint": fingerprint(&source).unwrap()
            }]
        })
        .to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root.clone()],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let directory_sync = FailOnceDirectorySync::new(plan.approved_roots[0].clone());

        let outcome = apply_locked_with(&plan, &directory_sync).unwrap();

        assert!(!outcome.verification_passed);
        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert!(!source.exists());
        assert_eq!(fs::read_to_string(target).unwrap(), "original");
    }

    #[test]
    fn removed_symlink_sync_failure_restores_the_link() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("link");
        fs::write(&source, "original").unwrap();
        create_symlink(&source, &target).unwrap();
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "remove_symlink",
                "target": target,
                "expected_fingerprint": fingerprint(&target).unwrap()
            }]
        })
        .to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root.clone()],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let directory_sync = FailOnceDirectorySync::new(plan.approved_roots[0].clone());

        let outcome = apply_locked_with(&plan, &directory_sync).unwrap();

        assert!(!outcome.verification_passed);
        assert_eq!(
            outcome.receipt.status,
            ReceiptStatus::FailedRolledBack,
            "{:?}",
            outcome.receipt.error
        );
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::canonicalize(&target).unwrap(),
            fs::canonicalize(source).unwrap()
        );
    }

    #[test]
    fn replaced_file_sync_failure_restores_original_content() {
        let (_temp, root, state) = fixture();
        let target = root.join("replace.txt");
        fs::write(&target, "before").unwrap();
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "replace_file",
                "target": target,
                "content": "after",
                "expected_fingerprint": fingerprint(&target).unwrap()
            }]
        })
        .to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root.clone()],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let directory_sync = FailOnceDirectorySync::new(plan.approved_roots[0].clone());

        let outcome = apply_locked_with(&plan, &directory_sync).unwrap();

        assert!(!outcome.verification_passed);
        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_to_string(&target).unwrap(), "before");
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
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
                state_dir: state.clone(),
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let applied = apply(&plan).unwrap();
        assert_eq!(
            applied.receipt.status,
            ReceiptStatus::Applied,
            "{:?}",
            applied.receipt.error
        );
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
    fn undo_refuses_a_drifted_cross_device_backup_before_mutation() {
        let (_temp, root, state) = fixture();
        let original = root.join("original");
        let backup = root.join(".original.skillroster-backup-test");
        let created = root.join("created");
        fs::write(&backup, "drifted backup").unwrap();
        fs::write(&created, "approved").unwrap();
        let expected = fingerprint(&created).unwrap();
        let receipt = ChangeReceipt {
            id: format!("receipt_{}", ulid::Ulid::new()),
            plan_id: format!("plan_{}", ulid::Ulid::new()),
            status: ReceiptStatus::Applied,
            changed_paths: vec![created.clone()],
            compensations: vec![Compensation::RestoreBackup {
                backup: backup.clone(),
                original: original.clone(),
                created: created.clone(),
                expected_created: expected,
            }],
            approved_roots: vec![root],
            state_dir: state,
            error: None,
            reverses_receipt_id: None,
            operation_results: vec![],
        };

        let outcome = undo(&receipt).unwrap();

        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_to_string(created).unwrap(), "approved");
        assert_eq!(fs::read_to_string(backup).unwrap(), "drifted backup");
        assert!(!original.exists());
    }

    #[test]
    fn lifecycle_check_refuses_a_drifted_cross_device_backup() {
        let (_temp, root, state) = fixture();
        let backup = root.join(".original.skillroster-backup-test");
        let created = root.join("created");
        fs::write(&backup, "drifted backup").unwrap();
        fs::write(&created, "approved").unwrap();
        let receipt = ChangeReceipt {
            id: format!("receipt_{}", ulid::Ulid::new()),
            plan_id: format!("plan_{}", ulid::Ulid::new()),
            status: ReceiptStatus::Applied,
            changed_paths: vec![created.clone()],
            compensations: vec![Compensation::RestoreBackup {
                backup,
                original: root.join("original"),
                created: created.clone(),
                expected_created: fingerprint(&created).unwrap(),
            }],
            approved_roots: vec![root],
            state_dir: state.clone(),
            error: None,
            reverses_receipt_id: None,
            operation_results: vec![],
        };
        persist_journal(&receipt).unwrap();
        let state_lock = StateLock::acquire_exclusive(&state).unwrap();

        let error = has_external_recovery_material(&state, &state_lock).unwrap_err();

        assert_eq!(error.code, "recovery_material_drifted");
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
                state_dir: state.clone(),
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let applied = apply(&plan).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let receipts = state.join("receipts");
            let journal = receipts.join(format!("{}.json", applied.receipt.id));
            assert_eq!(
                fs::metadata(receipts).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(journal).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "---\nname: test\n---\n"
        );
        let undone = undo(&applied.receipt).unwrap();
        assert_eq!(undone.receipt.status, ReceiptStatus::Undone);
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn replace_and_undo_preserve_private_and_executable_modes() {
        use std::os::unix::fs::PermissionsExt as _;

        for mode in [0o600, 0o500] {
            assert_replace_and_undo_preserves_permissions(fs::Permissions::from_mode(mode));
        }
    }

    #[cfg(unix)]
    #[test]
    fn replacement_undo_refuses_permission_drift() {
        use std::os::unix::fs::PermissionsExt;
        let (_temp, root, state) = fixture();
        let target = root.join("target");
        fs::write(&target, "original").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let input = serde_json::json!({"schema_version":1,"scan_id":"scan_1","operations":[{
            "kind":"replace_file","target":target,"content":"replacement","expected_fingerprint":fingerprint(&target).unwrap()
        }]}).to_string();
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
        assert!(applied.verification_passed);
        fs::set_permissions(&target, fs::Permissions::from_mode(0o400)).unwrap();
        let undone = undo(&applied.receipt).unwrap();
        assert_eq!(undone.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_to_string(&target).unwrap(), "replacement");
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o400
        );
    }

    #[cfg(unix)]
    #[test]
    fn recursive_copy_preserves_readonly_directory_mode_after_copying_children() {
        use std::os::unix::fs::PermissionsExt;
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), "retained").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o500)).unwrap();
        let anchored = open_anchored_fs(&[root], &state, &SystemDirectorySync).unwrap();
        anchored.copy_tree(&source, &target).unwrap();
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o500
        );
        assert_eq!(fs::read_to_string(target.join("file")).unwrap(), "retained");
        // Only release the two test-owned readonly directories for fixture cleanup.
        fs::set_permissions(source, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(target, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn recursive_directory_copy_and_undo_round_trip() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/file"), "retained").unwrap();
        let input = serde_json::json!({"schema_version":1,"scan_id":"scan_1","operations":[{
            "kind":"copy","source":source,"target":target,"expected_fingerprint":fingerprint(&source).unwrap()
        }]}).to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        #[cfg(windows)]
        let rename_checks_ran = {
            let ran = std::rc::Rc::new(std::cell::Cell::new(0));
            let hook_ran = ran.clone();
            let directory_hook_ran = ran.clone();
            let directories = [source.clone(), target.clone()];
            let paths = [
                source.clone(),
                source.join("nested"),
                target.clone(),
                target.join("nested"),
            ];
            BEFORE_EXECUTE_HOOK.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    crate::anchored_fs::set_after_created_entry_first_check_hook(move || {
                        for path in directories {
                            assert!(
                                fs::rename(&path, path.with_extension("moved")).is_err(),
                                "copy directory must be protected before children: {}",
                                path.display()
                            );
                        }
                        directory_hook_ran.set(directory_hook_ran.get() + 1);
                    });
                    crate::anchored_fs::set_after_staging_file_open_hook(move || {
                        for path in paths {
                            assert!(
                                fs::rename(&path, path.with_extension("moved")).is_err(),
                                "copy directory must remain rename-protected: {}",
                                path.display()
                            );
                        }
                        hook_ran.set(hook_ran.get() + 1);
                    });
                }));
            });
            ran
        };
        let applied = apply(&plan).unwrap();
        assert!(applied.verification_passed, "{:?}", applied.receipt.error);
        #[cfg(windows)]
        assert_eq!(rename_checks_ran.get(), 2, "both copy checkpoints ran");
        assert_eq!(
            fs::read_to_string(target.join("nested/file")).unwrap(),
            "retained"
        );
        let undone = undo(&applied.receipt).unwrap();
        assert!(undone.verification_passed, "{:?}", undone.receipt.error);
        assert!(!target.exists());
        assert_eq!(
            fs::read_to_string(source.join("nested/file")).unwrap(),
            "retained"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn copy_and_replace_refuse_extended_attributes_without_losing_original() {
        use std::os::fd::AsRawFd;
        for kind in ["copy", "replace_file"] {
            let (_temp, root, state) = fixture();
            let source = root.join("source.md");
            let target = root.join("copy.md");
            fs::write(&source, "original").unwrap();
            let file = File::open(&source).unwrap();
            let name = c"user.skillroster-test";
            let value = b"user metadata";
            #[cfg(target_os = "linux")]
            let result = unsafe {
                libc::fsetxattr(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                )
            };
            #[cfg(target_os = "macos")]
            let result = unsafe {
                libc::fsetxattr(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                    0,
                )
            };
            assert_eq!(result, 0, "fixture xattr: {}", io::Error::last_os_error());
            let operation = if kind == "copy" {
                serde_json::json!({"kind":kind,"source":source,"target":target,"expected_fingerprint":fingerprint(&source).unwrap()})
            } else {
                serde_json::json!({"kind":kind,"target":source,"content":"replacement","expected_fingerprint":fingerprint(&source).unwrap()})
            };
            let input =
                serde_json::json!({"schema_version":1,"scan_id":"scan_1","operations":[operation]})
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
            match apply(&plan) {
                Err(error) => assert!(error.message.contains("metadata"), "{error}"),
                Ok(outcome) => assert!(
                    !outcome.verification_passed,
                    "silently discarded extended metadata"
                ),
            }
            assert_eq!(fs::read_to_string(&source).unwrap(), "original");
            assert!(!target.exists());
        }
    }

    #[cfg(windows)]
    #[test]
    fn replace_and_undo_preserve_readonly_state() {
        let temp = tempfile::tempdir().unwrap();
        let probe = temp.path().join("permissions-probe");
        fs::write(&probe, "probe").unwrap();
        let mut permissions = fs::metadata(&probe).unwrap().permissions();
        permissions.set_readonly(true);
        assert_replace_and_undo_preserves_permissions(permissions);
    }

    fn assert_replace_and_undo_preserves_permissions(original_permissions: fs::Permissions) {
        let (_temp, root, state) = fixture();
        let target = root.join("replace.txt");
        fs::write(&target, "before").unwrap();
        fs::set_permissions(&target, original_permissions.clone()).unwrap();
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "replace_file",
                "target": target,
                "content": "after",
                "expected_fingerprint": fingerprint(&target).unwrap()
            }]
        })
        .to_string();
        let plan = prepare(
            &input,
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state.clone(),
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();

        let applied = apply(&plan).unwrap();

        assert!(
            applied.verification_passed,
            "Apply failed: {:?}",
            applied.receipt.error
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "after");
        assert_permissions_equal(&target, &original_permissions);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let recovery = state.join("recovery");
            let receipt_recovery = recovery.join(&applied.receipt.id);
            assert_eq!(
                fs::metadata(recovery).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(receipt_recovery).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }

        let undone = undo(&applied.receipt).unwrap();

        assert!(undone.verification_passed);
        assert_eq!(fs::read_to_string(&target).unwrap(), "before");
        assert_permissions_equal(&target, &original_permissions);
        #[cfg(windows)]
        {
            let mut writable = original_permissions;
            writable.set_readonly(false);
            fs::set_permissions(&target, writable).unwrap();
        }
    }

    #[test]
    fn replacement_staging_failure_leaves_original_content_and_permissions() {
        let (_temp, root, state) = fixture();
        let target = root.join("replace.txt");
        fs::write(&target, "before").unwrap();
        let permissions = fs::metadata(&target).unwrap().permissions();
        let receipt_id = "receipt_fixture";
        let staged = root.join(format!(".replace.txt.skillroster-replace-{receipt_id}-0"));
        fs::write(&staged, "must not be overwritten").unwrap();
        let anchored = open_anchored_fs(&[root], &state, &SystemDirectorySync).unwrap();

        let error = replace_file(&anchored, &target, "after", receipt_id, 0).unwrap_err();

        assert_eq!(error.code, "target_exists");
        assert_eq!(fs::read_to_string(&target).unwrap(), "before");
        assert_permissions_equal(&target, &permissions);
        assert_eq!(
            fs::read_to_string(staged).unwrap(),
            "must not be overwritten"
        );
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_copy_never_exposes_private_source_under_wider_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let state = temp.path().join("state");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&state).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let state = fs::canonicalize(state).unwrap();
        let source = root.join("private-source");
        let staged = root.join("visible-staging");
        fs::write(&source, "private contents").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        let anchored = open_anchored_fs(&[root], &state, &SystemDirectorySync).unwrap();

        anchored.copy_tree(&source, &staged).unwrap();

        assert_eq!(
            fs::metadata(&staged).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(fs::read_to_string(&staged).unwrap(), "private contents");
    }

    #[cfg(unix)]
    #[test]
    fn staging_permissions_are_applied_to_the_created_file_handle() {
        use std::os::unix::fs::PermissionsExt as _;

        let (temp, root, state) = fixture();
        let source = root.join("private-source");
        let target = root.join("staging");
        let held_target = root.join("held-staging");
        fs::write(&source, "private contents").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        let hook_target = target.clone();
        let hook_held_target = held_target.clone();
        crate::anchored_fs::set_after_staging_file_open_hook(move || {
            fs::rename(&hook_target, &hook_held_target).unwrap();
            fs::write(&hook_target, "decoy").unwrap();
            fs::set_permissions(&hook_target, fs::Permissions::from_mode(0o666)).unwrap();
        });
        let anchored = open_anchored_fs(&[root], &state, &SystemDirectorySync).unwrap();

        let error = anchored.copy_tree(&source, &target).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            fs::metadata(&held_target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o666
        );
        assert_eq!(fs::read_to_string(held_target).unwrap(), "private contents");
        assert_eq!(fs::read_to_string(target).unwrap(), "decoy");
        drop(temp);
    }

    #[cfg(unix)]
    #[test]
    fn apply_never_compensates_an_entry_swapped_onto_a_copy_target() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("target");
        let held_target = root.join("held-target");
        fs::write(&source, "approved").unwrap();
        let input = serde_json::json!({
            "schema_version": 1,
            "scan_id": "scan_1",
            "operations": [{
                "kind": "copy",
                "source": source,
                "target": target,
                "expected_fingerprint": fingerprint(&source).unwrap()
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
        let hook_target = target.clone();
        let hook_held_target = held_target.clone();
        BEFORE_EXECUTE_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                crate::anchored_fs::set_after_staging_file_open_hook(move || {
                    fs::rename(&hook_target, &hook_held_target).unwrap();
                    fs::write(&hook_target, "external decoy").unwrap();
                });
            }));
        });

        let outcome = apply(&plan).unwrap();

        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_to_string(&target).unwrap(), "external decoy");
        assert_eq!(fs::read_to_string(&held_target).unwrap(), "approved");
    }

    #[cfg(unix)]
    #[test]
    fn apply_never_compensates_entries_swapped_after_creation() {
        for kind in [
            "write_file",
            "create_directory",
            "create_symlink",
            "create_symlink_after_first_check",
        ] {
            let (_temp, root, state) = fixture();
            let source = root.join("source");
            let target = root.join("target");
            let held_target = root.join("held-target");
            fs::write(&source, "approved").unwrap();
            let operation = match kind {
                "write_file" => serde_json::json!({
                    "kind": kind,
                    "target": target,
                    "content": "approved",
                    "expected_fingerprint": "missing",
                }),
                "create_directory" => serde_json::json!({
                    "kind": kind,
                    "target": target,
                    "expected_fingerprint": "missing",
                }),
                "create_symlink" | "create_symlink_after_first_check" => serde_json::json!({
                    "kind": "create_symlink",
                    "source": source,
                    "target": target,
                    "expected_fingerprint": "missing",
                    "expected_source_fingerprint": fingerprint(&source).unwrap(),
                }),
                _ => unreachable!(),
            };
            let plan = prepare(
                &serde_json::json!({
                    "schema_version": 1,
                    "scan_id": "scan_1",
                    "operations": [operation],
                })
                .to_string(),
                &PrepareContext {
                    approved_roots: vec![root],
                    state_dir: state,
                    operation_policy: OperationPolicy::TestOnly,
                },
            )
            .unwrap();
            let hook_target = target.clone();
            let hook_held_target = held_target.clone();
            BEFORE_EXECUTE_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || match kind {
                    "write_file" => {
                        crate::anchored_fs::set_after_staging_file_open_hook(move || {
                            fs::rename(&hook_target, &hook_held_target).unwrap();
                            fs::write(&hook_target, "external decoy").unwrap();
                        });
                    }
                    "create_directory" => {
                        crate::anchored_fs::set_after_created_directory_open_hook(move || {
                            fs::rename(&hook_target, &hook_held_target).unwrap();
                            fs::create_dir(&hook_target).unwrap();
                        });
                    }
                    "create_symlink" => {
                        crate::anchored_fs::set_after_created_symlink_hook(move || {
                            fs::remove_file(&hook_target).unwrap();
                            fs::hard_link(&source, &hook_target).unwrap();
                        });
                    }
                    "create_symlink_after_first_check" => {
                        crate::anchored_fs::set_after_created_symlink_first_check_hook(move || {
                            fs::remove_file(&hook_target).unwrap();
                            fs::hard_link(&source, &hook_target).unwrap();
                        });
                    }
                    _ => unreachable!(),
                }));
            });

            let outcome = apply(&plan).unwrap();

            assert_eq!(
                outcome.receipt.status,
                ReceiptStatus::RecoveryRequired,
                "{kind}: {:?}",
                outcome.receipt.error
            );
            assert!(target.exists());
            if !kind.starts_with("create_symlink") {
                assert!(held_target.exists());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn remove_symlink_restores_an_entry_swapped_before_rename() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("target");
        let held_target = root.join("held-target");
        fs::write(&source, "approved").unwrap();
        create_symlink(&source, &target).unwrap();
        let plan = prepare(
            &serde_json::json!({
                "schema_version": 1,
                "scan_id": "scan_1",
                "operations": [{
                    "kind": "remove_symlink",
                    "target": target,
                    "expected_fingerprint": fingerprint(&target).unwrap(),
                }],
            })
            .to_string(),
            &PrepareContext {
                approved_roots: vec![root],
                state_dir: state,
                operation_policy: OperationPolicy::TestOnly,
            },
        )
        .unwrap();
        let hook_target = target.clone();
        let hook_held_target = held_target.clone();
        BEFORE_REMOVE_SYMLINK_RENAME_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&hook_target, &hook_held_target).unwrap();
                fs::write(&hook_target, "external decoy").unwrap();
            }));
        });

        let outcome = apply(&plan).unwrap();

        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_to_string(&target).unwrap(), "external decoy");
        assert!(
            fs::symlink_metadata(&held_target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let backup = move_backup_path(&target, &outcome.receipt.id, 0).unwrap();
        assert!(!backup.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_undo_restores_exact_legacy_relative_symlink_contents() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("target");
        fs::write(&source, "approved").unwrap();
        std::os::windows::fs::symlink_file(Path::new("source"), &target).unwrap();
        let original_contents = fs::read_link(&target).unwrap();
        let original_fingerprint = fingerprint(&target).unwrap();
        fs::remove_file(&target).unwrap();
        let receipt = ChangeReceipt {
            id: format!("receipt_{}", ulid::Ulid::new()),
            plan_id: format!("plan_{}", ulid::Ulid::new()),
            status: ReceiptStatus::Applied,
            changed_paths: vec![target.clone()],
            compensations: vec![Compensation::RestoreSymlink {
                path: target.clone(),
                target: original_contents.clone(),
            }],
            approved_roots: vec![root],
            state_dir: state,
            error: None,
            reverses_receipt_id: None,
            operation_results: vec![],
        };

        let outcome = undo(&receipt).unwrap();

        assert_eq!(
            outcome.receipt.status,
            ReceiptStatus::Undone,
            "{:?}",
            outcome.receipt.error
        );
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&target).unwrap(), original_contents);
        assert_eq!(fingerprint(&target).unwrap(), original_fingerprint);
        assert_eq!(
            fs::canonicalize(target).unwrap(),
            fs::canonicalize(source).unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_legacy_absolute_symlink_restore_fails_before_mutation() {
        let (_temp, root, state) = fixture();
        let source = root.join("source");
        let target = root.join("target");
        fs::write(&source, "approved").unwrap();
        let receipt = ChangeReceipt {
            id: format!("receipt_{}", ulid::Ulid::new()),
            plan_id: format!("plan_{}", ulid::Ulid::new()),
            status: ReceiptStatus::Applied,
            changed_paths: vec![target.clone()],
            compensations: vec![Compensation::RestoreSymlink {
                path: target.clone(),
                target: source,
            }],
            approved_roots: vec![root],
            state_dir: state,
            error: None,
            reverses_receipt_id: None,
            operation_results: vec![],
        };

        let outcome = undo(&receipt).unwrap();

        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert!(fs::symlink_metadata(target).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_legacy_relative_restore_cannot_be_redirected_by_an_ancestor_swap() {
        let (temp, root, state) = fixture();
        let mutable = root.join("mutable");
        let held_mutable = root.join("held-mutable");
        let outside = temp.path().join("outside");
        let source = mutable.join("source");
        let outside_source = outside.join("source");
        let target = root.join("target");
        fs::create_dir(&mutable).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(&source, "approved").unwrap();
        fs::write(&outside_source, "external").unwrap();
        let receipt = ChangeReceipt {
            id: format!("receipt_{}", ulid::Ulid::new()),
            plan_id: format!("plan_{}", ulid::Ulid::new()),
            status: ReceiptStatus::Applied,
            changed_paths: vec![target.clone()],
            compensations: vec![Compensation::RestoreSymlink {
                path: target.clone(),
                target: PathBuf::from("mutable").join("source"),
            }],
            approved_roots: vec![root],
            state_dir: state,
            error: None,
            reverses_receipt_id: None,
            operation_results: vec![],
        };
        let swap_succeeded = Rc::new(Cell::new(false));
        let hook_swap_succeeded = Rc::clone(&swap_succeeded);
        crate::anchored_fs::set_after_symlink_source_open_hook(move || {
            if fs::rename(&mutable, &held_mutable).is_ok() {
                std::os::windows::fs::symlink_dir(&outside, &mutable).unwrap();
                hook_swap_succeeded.set(true);
            }
        });

        let outcome = undo(&receipt).unwrap();

        assert!(matches!(
            outcome.receipt.status,
            ReceiptStatus::Undone | ReceiptStatus::RecoveryRequired
        ));
        match fs::symlink_metadata(&target) {
            Ok(metadata) => {
                assert!(metadata.file_type().is_symlink());
                assert_eq!(
                    fs::read_link(&target).unwrap(),
                    PathBuf::from("mutable").join("source")
                );
                assert_eq!(fs::read_to_string(&target).unwrap(), "approved");
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
            }
            Err(error) => panic!("unexpected target state: {error}"),
        }
        assert!(!swap_succeeded.get() || outcome.receipt.status == ReceiptStatus::RecoveryRequired);
        assert_eq!(fs::read_to_string(outside_source).unwrap(), "external");
    }

    fn assert_permissions_equal(path: &Path, expected: &fs::Permissions) {
        let actual = fs::metadata(path).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(actual.mode() & 0o7777, expected.mode() & 0o7777);
        }
        #[cfg(windows)]
        assert_eq!(actual.readonly(), expected.readonly());
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
    fn failed_copy_is_journaled_without_path_compensating_its_partial_target() {
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
        assert_eq!(outcome.receipt.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(outcome.receipt.operation_results[0].status, "rolled_back");
        assert_eq!(
            outcome.receipt.operation_results[1].status,
            "recovery_required"
        );
        assert!(outcome.receipt.operation_results[1].error.is_some());
        assert!(!root.join("first").exists());
        assert!(root.join("partial-copy").is_dir());
    }
}
