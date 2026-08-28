use std::path::PathBuf;

use serde_json::{Value, json};

use super::action;
use crate::action_context::ActionContext;
use crate::model::{ApiError, FindingId, JsonEnvelope, PlanId, ScanId};
use crate::scan;

/// Agent tool-result transport bound, deliberately narrower than the 2 MiB
/// inventory parser bound. Larger Skills should disclose references on demand.
pub(super) const MAX_AGENT_LOADED_SKILL_BYTES: u64 = 128 * 1024;

#[derive(Debug)]
pub(super) struct FindSnapshotChanged {
    pub(super) expected_snapshot_id: ScanId,
    pub(super) actual_snapshot_id: ScanId,
    pub(super) retry_argv: Vec<String>,
}

impl std::fmt::Display for FindSnapshotChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Find recovery expected Snapshot {} but latest is {}",
            self.expected_snapshot_id, self.actual_snapshot_id
        )
    }
}

impl std::error::Error for FindSnapshotChanged {}

#[derive(Debug)]
pub(super) struct SkillLoadBlocked {
    pub(super) reason: &'static str,
    pub(super) skill_id: String,
    pub(super) skill_name: String,
    pub(super) path: Option<PathBuf>,
    pub(super) roster_state: String,
    pub(super) mutation_scopes: Vec<String>,
    pub(super) expected_digest: Option<String>,
    pub(super) actual_digest: Option<String>,
    pub(super) retry_argv: Option<Vec<String>>,
}

impl std::fmt::Display for SkillLoadBlocked {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "verified Skill load blocked for {} ({}): {}",
            self.skill_name, self.skill_id, self.reason
        )
    }
}

impl std::error::Error for SkillLoadBlocked {}

#[derive(Debug)]
pub(super) struct ContentIdentityRescanRequired {
    pub(super) reason: &'static str,
}

impl std::fmt::Display for ContentIdentityRescanRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.reason {
            "non_unicode_identity_coverage_incomplete" => formatter.write_str(
                "latest Snapshot excluded non-Unicode identity paths; resolve them and run skillroster scan",
            ),
            _ => write!(
                formatter,
                "latest Snapshot has no {} content identity; run skillroster scan",
                scan::CONTENT_IDENTITY_ALGORITHM
            ),
        }
    }
}

impl std::error::Error for ContentIdentityRescanRequired {}

pub fn error_json(command: &str, error: &(dyn std::error::Error + 'static)) -> String {
    error_json_with_context(command, error, &ActionContext::default())
}

pub fn error_json_with_context(
    command: &str,
    error: &(dyn std::error::Error + 'static),
    action_context: &ActionContext,
) -> String {
    if let Some(changed) = error.downcast_ref::<FindSnapshotChanged>() {
        let mut envelope = JsonEnvelope::<Value>::failure(
            command,
            ApiError {
                code: "find_snapshot_changed".into(),
                message: error.to_string(),
                retryable: true,
                relevant_ids: vec![
                    changed.expected_snapshot_id.to_string(),
                    changed.actual_snapshot_id.to_string(),
                ],
                paths: Vec::new(),
                details: Some(json!({
                    "expected_snapshot_id": changed.expected_snapshot_id,
                    "actual_snapshot_id": changed.actual_snapshot_id,
                    "files_changed": false,
                    "state_files_changed": false,
                    "next_action": "rerun_find_on_latest_snapshot",
                })),
            },
        );
        let argv = changed
            .retry_argv
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        envelope.suggested_actions = vec![action(
            "rerun_find_on_latest_snapshot",
            &argv,
            false,
            false,
            "find_snapshot_changed",
        )];
        action_context.apply(&mut envelope.suggested_actions);
        return serde_json::to_string(&envelope)
            .unwrap_or_else(|_| r#"{"schema_version":1,"ok":false}"#.into());
    }
    if let Some(rescan) = error.downcast_ref::<ContentIdentityRescanRequired>() {
        let mut envelope = JsonEnvelope::<Value>::failure(
            command,
            ApiError {
                code: "content_identity_rescan_required".into(),
                message: error.to_string(),
                retryable: true,
                relevant_ids: Vec::new(),
                paths: Vec::new(),
                details: Some(json!({
                    "reason": rescan.reason,
                    "required_algorithm": scan::CONTENT_IDENTITY_ALGORITHM,
                    "files_changed": false,
                    "state_files_changed": false,
                    "next_action": "scan"
                })),
            },
        );
        envelope.suggested_actions = vec![action(
            "refresh_snapshot_content_identity",
            &["scan", "--summary", "--json"],
            false,
            false,
            "content_identity_rescan_required",
        )];
        action_context.apply(&mut envelope.suggested_actions);
        return serde_json::to_string(&envelope)
            .unwrap_or_else(|_| r#"{"schema_version":1,"ok":false}"#.into());
    }
    if let Some(blocked) = error.downcast_ref::<crate::roster_plan::RosterPlanBlocked>() {
        let mut envelope = JsonEnvelope::<Value>::failure(
            command,
            ApiError {
                code: "trusted_canonical_sources_required".into(),
                message: blocked.message.clone(),
                retryable: false,
                relevant_ids: blocked.relevant_ids.clone(),
                paths: blocked.paths.clone(),
                details: Some(blocked.details.clone()),
            },
        );
        if !blocked.paths.is_empty() {
            let mut argv = Vec::new();
            for path in &blocked.paths {
                argv.push("--source-root");
                argv.push(path.as_str());
            }
            argv.extend(["scan", "--summary", "--json"]);
            envelope.suggested_actions = vec![action(
                "scan_with_confirmed_source_roots",
                &argv,
                false,
                true,
                "trusted_canonical_sources_required",
            )];
            action_context.apply(&mut envelope.suggested_actions);
        }
        return serde_json::to_string(&envelope)
            .unwrap_or_else(|_| r#"{"schema_version":1,"ok":false}"#.into());
    }
    if let Some(blocked) = error.downcast_ref::<SkillLoadBlocked>() {
        let classified = classify_error(error);
        let mut envelope = JsonEnvelope::<Value>::failure(
            command,
            ApiError {
                code: classified.code.into(),
                message: error.to_string(),
                retryable: classified.retryable,
                details: classified.details,
                relevant_ids: extract_relevant_ids(&error.to_string()),
                paths: extract_paths(&error.to_string()),
            },
        );
        let safe_retry = match blocked.reason {
            "same_name_variants_ambiguous" => blocked.retry_argv.clone().map(|argv| {
                (
                    "inspect_same_name_variants",
                    argv,
                    "same_name_variants_ambiguous",
                )
            }),
            "no_routable_match" => Some((
                "inspect_current_report",
                vec!["report".into(), "--summary".into(), "--json".into()],
                "verified_skill_load_blocked",
            )),
            "placement_missing_from_snapshot"
            | "package_fingerprint_incomplete"
            | "legacy_snapshot_requires_rescan"
            | "eligible_placement_missing"
            | "entrypoint_content_drift"
            | "package_identity_drift" => Some((
                "refresh_snapshot",
                vec!["scan".into(), "--summary".into(), "--json".into()],
                "verified_skill_load_blocked",
            )),
            _ => None,
        };
        if let Some((name, argv, reason_code)) = safe_retry {
            let argv = argv.iter().map(String::as_str).collect::<Vec<_>>();
            envelope.suggested_actions = vec![action(name, &argv, false, false, reason_code)];
            action_context.apply(&mut envelope.suggested_actions);
        }
        return serde_json::to_string(&envelope)
            .unwrap_or_else(|_| r#"{"schema_version":1,"ok":false}"#.into());
    }
    let classified = classify_error(error);
    serde_json::to_string(&JsonEnvelope::<Value>::failure(
        command,
        ApiError {
            code: classified.code.into(),
            message: error.to_string(),
            retryable: classified.retryable,
            details: classified.details,
            relevant_ids: extract_relevant_ids(&error.to_string()),
            paths: extract_paths(&error.to_string()),
        },
    ))
    .unwrap_or_else(|_| r#"{"schema_version":1,"ok":false}"#.into())
}

struct ClassifiedError {
    code: &'static str,
    retryable: bool,
    details: Option<Value>,
}

#[derive(Debug)]
pub(super) struct PlanSnapshotDrift {
    pub(super) plan_id: PlanId,
    pub(super) expected_snapshot_id: ScanId,
    pub(super) current_snapshot_id: ScanId,
}

impl std::fmt::Display for PlanSnapshotDrift {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Plan {} is stale; a newer Snapshot exists",
            self.plan_id
        )
    }
}

impl std::error::Error for PlanSnapshotDrift {}

#[derive(Debug)]
pub(super) struct StoredFindingCoverageInvalid {
    pub(super) finding_id: FindingId,
    pub(super) reason: &'static str,
}

impl std::fmt::Display for StoredFindingCoverageInvalid {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Finding {} has invalid stored coverage facts; run scan again",
            self.finding_id
        )
    }
}

impl std::error::Error for StoredFindingCoverageInvalid {}

#[derive(Debug)]
pub(super) struct LibraryRootConflict {
    pub(super) library_root: PathBuf,
    pub(super) agent_roots: Vec<PathBuf>,
}

impl std::fmt::Display for LibraryRootConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Hosted Library root {} overlaps an Agent Skill root",
            self.library_root.display()
        )
    }
}

impl std::error::Error for LibraryRootConflict {}

#[derive(Debug)]
pub(super) struct IncompleteFingerprintBlocker {
    skill_id: String,
    placement_id: String,
    path: PathBuf,
    completeness: scan::FingerprintCompleteness,
    stage: &'static str,
}

impl std::fmt::Display for IncompleteFingerprintBlocker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Skill {} placement {} has a {} package fingerprint; resolve fingerprint incompleteness before governance",
            self.skill_id,
            self.placement_id,
            self.completeness.id()
        )
    }
}

impl std::error::Error for IncompleteFingerprintBlocker {}

pub(super) fn incomplete_fingerprint_blocker<'a>(
    placements: impl IntoIterator<Item = &'a scan::SkillPlacement>,
    stage: &'static str,
) -> Option<IncompleteFingerprintBlocker> {
    placements
        .into_iter()
        .find(|placement| {
            placement.fingerprint_completeness != scan::FingerprintCompleteness::Complete
        })
        .map(|placement| IncompleteFingerprintBlocker {
            skill_id: placement.skill_id.clone(),
            placement_id: placement.id.clone(),
            path: placement.entrypoint.clone(),
            completeness: placement.fingerprint_completeness,
            stage,
        })
}

fn fingerprint_remediation(completeness: scan::FingerprintCompleteness) -> Value {
    let (reason, required_before_rescan, options) = match completeness {
        scan::FingerprintCompleteness::Complete => ("none", false, Vec::<&str>::new()),
        scan::FingerprintCompleteness::Bounded => (
            "package_exceeds_fingerprint_bounds",
            true,
            vec![
                "reduce_package_below_fingerprint_byte_limit",
                "move_relevant_content_within_supported_depth",
            ],
        ),
        scan::FingerprintCompleteness::Unreadable => (
            "package_could_not_be_read_completely",
            true,
            vec!["repair_local_read_access", "confirm_safe_source_boundary"],
        ),
        scan::FingerprintCompleteness::Unknown => (
            "legacy_snapshot_has_no_completeness_fact",
            false,
            vec!["scan_with_current_skillroster"],
        ),
    };
    json!({
        "automatic_change_supported": false,
        "reason": reason,
        "required_before_rescan": required_before_rescan,
        "options": options,
        "then": "scan"
    })
}

fn classify_error(error: &(dyn std::error::Error + 'static)) -> ClassifiedError {
    if let Some(blocker) = error.downcast_ref::<SkillLoadBlocked>() {
        let (next_action, retry_mode) = match blocker.reason {
            "same_name_variants_ambiguous" => {
                ("inspect_same_name_variants", "read_only_command_available")
            }
            "variant_selector_requires_load" => (
                "add_load_or_remove_variant_selector",
                "agent_correction_required",
            ),
            "variant_selector_requires_ambiguous_top_match" => {
                ("remove_variant_selector", "agent_correction_required")
            }
            "variant_not_in_top_match" => {
                ("choose_exposed_top_match_variant", "agent_choice_required")
            }
            "no_routable_match" => (
                "inspect_current_report_or_refine_task",
                "read_only_command_available",
            ),
            "untrusted_external_source" => (
                "confirm_exact_source_root_then_scan",
                "user_decision_required",
            ),
            "archived_skill_not_routable" => ("choose_non_archived_skill", "agent_choice_required"),
            "entrypoint_exceeds_content_limit" => (
                "reduce_entrypoint_below_limit_then_scan",
                "manual_resolution_required",
            ),
            "entrypoint_unreadable" | "package_identity_unreadable" => (
                "repair_local_read_access_then_scan",
                "manual_resolution_required",
            ),
            "entrypoint_escapes_approved_roots" => (
                "move_under_approved_root_or_confirm_source_then_scan",
                "user_decision_required",
            ),
            "entrypoint_not_utf8" => (
                "convert_entrypoint_to_utf8_then_scan",
                "manual_resolution_required",
            ),
            "package_fingerprint_incomplete" => (
                "resolve_incomplete_package_then_scan",
                "read_only_command_available",
            ),
            "placement_missing_from_snapshot"
            | "legacy_snapshot_requires_rescan"
            | "eligible_placement_missing"
            | "entrypoint_content_drift"
            | "package_identity_drift" => ("scan", "read_only_command_available"),
            _ => ("inspect_blocker", "manual_resolution_required"),
        };
        return ClassifiedError {
            code: "verified_skill_load_blocked",
            retryable: false,
            details: Some(json!({
                "reason": blocker.reason,
                "skill_id": blocker.skill_id,
                "skill_name": blocker.skill_name,
                "path": blocker.path,
                "roster_state": blocker.roster_state,
                "mutation_scopes": blocker.mutation_scopes,
                "expected_digest": blocker.expected_digest,
                "actual_digest": blocker.actual_digest,
                "content_limit_bytes": MAX_AGENT_LOADED_SKILL_BYTES,
                "files_changed": false,
                "state_files_changed": false,
                "task_success": "not_evaluated",
                "next_action": next_action,
                "retry_mode": retry_mode,
            })),
        };
    }
    if let Some(policy) = error.downcast_ref::<crate::source_policy::SourceRootPolicyError>() {
        use crate::source_policy::SourceRootPolicyError as PolicyError;
        let facts = match policy {
            PolicyError::FindingNotFound { finding_id }
            | PolicyError::NotEscapingLinkFinding { finding_id } => {
                json!({"finding_id": finding_id})
            }
            PolicyError::FindingSnapshotNotCurrent {
                finding_id,
                finding_snapshot,
                current_snapshot,
            } => json!({
                "finding_id": finding_id,
                "finding_snapshot_id": finding_snapshot,
                "current_snapshot_id": current_snapshot,
            }),
            PolicyError::PathNotResolvable { path, reason } => {
                json!({"path": path, "reason": reason})
            }
            PolicyError::PathNotObservedTarget { path }
            | PolicyError::PathNotDirectory { path } => json!({"path": path}),
            PolicyError::PermissionNotFound { permission_id }
            | PolicyError::PermissionAlreadyRevoked { permission_id } => {
                json!({"permission_id": permission_id})
            }
            PolicyError::ActivePermissionIdentityDrift {
                permission_id,
                path,
            } => json!({"permission_id": permission_id, "path": path}),
        };
        let mut details = json!({
            "permission_scope": "exact_local_read_only",
            "content_endorsed": false,
            "evidence_quality_changed": false,
            "governance_authorized": false,
            "plan_apply_authorized": false,
            "files_changed": false,
            "state_files_changed": false,
            "next_action": match policy {
                PolicyError::FindingSnapshotNotCurrent { .. } => "scan_then_open_current_finding",
                PolicyError::PermissionAlreadyRevoked { .. } => "inspect_source_root_permissions",
                PolicyError::ActivePermissionIdentityDrift { .. } => "revoke_then_confirm_current_observed_target",
                _ => "inspect_exact_source_root_facts",
            }
        });
        if let (Some(details), Some(facts)) = (details.as_object_mut(), facts.as_object()) {
            details.extend(facts.clone());
        }
        return ClassifiedError {
            code: policy.code(),
            retryable: false,
            details: Some(details),
        };
    }
    if let Some(blocker) = error.downcast_ref::<IncompleteFingerprintBlocker>() {
        return ClassifiedError {
            code: "incomplete_package_fingerprint",
            retryable: false,
            details: Some(json!({
                "reason": "exact_content_evidence_incomplete",
                "skill_id": blocker.skill_id,
                "placement_id": blocker.placement_id,
                "path": blocker.path,
                "completeness": blocker.completeness,
                "stage": blocker.stage,
                "remediation": fingerprint_remediation(blocker.completeness),
                "files_changed": false,
                "next_action": "resolve_fingerprint_incompleteness_then_scan"
            })),
        };
    }
    if let Some(conflict) = error.downcast_ref::<LibraryRootConflict>() {
        let agent_root_count = conflict.agent_roots.len();
        return ClassifiedError {
            code: "library_root_conflicts_with_agent_root",
            retryable: false,
            details: Some(json!({
                "reason": "library_root_overlaps_agent_skill_root",
                "library_root": conflict.library_root,
                "agent_root_count": agent_root_count,
                "agent_roots": conflict.agent_roots.iter().take(10).collect::<Vec<_>>(),
                "agent_roots_truncated": agent_root_count > 10,
                "files_changed": false,
                "next_action": "choose_state_dir_outside_agent_skill_roots"
            })),
        };
    }
    if let Some(invalid) = error.downcast_ref::<StoredFindingCoverageInvalid>() {
        return ClassifiedError {
            code: "stored_finding_coverage_invalid",
            retryable: false,
            details: Some(json!({
                "finding_id": invalid.finding_id,
                "reason": invalid.reason,
                "files_changed": false,
                "next_action": "scan"
            })),
        };
    }
    if let Some(drift) = error.downcast_ref::<PlanSnapshotDrift>() {
        return ClassifiedError {
            code: "state_drift",
            retryable: false,
            details: Some(json!({
                "reason": "plan_snapshot_stale",
                "plan_id": drift.plan_id,
                "expected_snapshot_id": drift.expected_snapshot_id,
                "current_snapshot_id": drift.current_snapshot_id,
                "files_changed": false,
                "next_action": "create_plan_for_current_snapshot"
            })),
        };
    }
    if let Some(conflict) = error.downcast_ref::<crate::roster_plan::RosterPhysicalConflict>() {
        return ClassifiedError {
            code: "roster_physical_state_conflict",
            retryable: false,
            details: Some(json!({
                "reason": "shared_physical_state_conflict",
                "skill_id": conflict.skill_id,
                "agents": conflict.agents,
                "files_changed": false,
                "next_action": "request_consistent_exposure_or_separate_shared_roots"
            })),
        };
    }
    if let Some(blocker) = error.downcast_ref::<crate::roster_plan::RosterDiscoveryIncomplete>() {
        return ClassifiedError {
            code: "roster_skill_root_discovery_incomplete",
            retryable: false,
            details: Some(json!({
                "reason": "skill_root_discovery_bounded",
                "agent": blocker.agent,
                "path": blocker.path,
                "detail": blocker.detail,
                "files_changed": false,
                "next_action": "scan"
            })),
        };
    }
    if let Some(conflict) = error.downcast_ref::<crate::roster_plan::RosterOperationConflict>() {
        return ClassifiedError {
            code: "roster_operation_identity_conflict",
            retryable: false,
            details: Some(json!({
                "reason": "duplicate_physical_operation_identity",
                "identity_role": conflict.identity_role,
                "conflicting_operation_kinds": conflict.operation_kinds,
                "path": conflict.path,
                "files_changed": false,
                "next_action": "resolve_same_name_variant_or_shared_ownership"
            })),
        };
    }
    if let Some(blocker) =
        error.downcast_ref::<crate::roster_plan::RosterPackageFingerprintVariants>()
    {
        return ClassifiedError {
            code: "roster_package_fingerprint_variants",
            retryable: false,
            details: Some(json!({
                "reason": "multiple_package_fingerprints_require_explicit_preservation",
                "skill_id": blocker.skill_id,
                "placement_ids": blocker.placement_ids,
                "fingerprint_count": blocker.fingerprint_count,
                "files_changed": false,
                "next_action": "review_each_package_before_roster_mutation"
            })),
        };
    }
    if let Some(blocker) = error.downcast_ref::<crate::roster_plan::RosterSafetyBlocker>() {
        let (
            code,
            reason,
            skill_id,
            placement_ids,
            paths,
            providers,
            mutation_scopes,
            owned_by_agent,
            next_action,
        ) = match blocker {
            crate::roster_plan::RosterSafetyBlocker::ProviderManaged {
                skill_id,
                placement_ids,
                paths,
                providers,
            } => (
                "roster_provider_managed_read_only",
                "provider_managed_placement_is_read_only",
                skill_id,
                placement_ids,
                paths,
                Some(providers),
                None,
                json!(false),
                "exclude_provider_managed_placement",
            ),
            crate::roster_plan::RosterSafetyBlocker::MutationScopeReadOnly {
                skill_id,
                placement_ids,
                paths,
                owned_by_agent,
                mutation_scopes,
            } => (
                "roster_mutation_scope_read_only",
                "non_mutable_placement_blocks_mutation",
                skill_id,
                placement_ids,
                paths,
                None,
                Some(mutation_scopes),
                json!(owned_by_agent),
                "rescan_or_choose_mutable_placement",
            ),
            crate::roster_plan::RosterSafetyBlocker::DependentSource {
                skill_id,
                placement_ids,
                paths,
            } => (
                "roster_dependent_source_conflict",
                "dependent_source_would_break",
                skill_id,
                placement_ids,
                paths,
                None,
                None,
                Value::Null,
                "preserve_or_retarget_dependent_source",
            ),
        };
        return ClassifiedError {
            code,
            retryable: false,
            details: Some(json!({
                "reason": reason,
                "skill_id": skill_id,
                "placement_ids": placement_ids,
                "paths": paths,
                "providers": providers,
                "mutation_scopes": mutation_scopes,
                "owned_by_agent": owned_by_agent,
                "files_changed": false,
                "next_action": next_action
            })),
        };
    }
    if let Some(drift) = error.downcast_ref::<crate::scan::PhysicalDirectoryDrift>() {
        return ClassifiedError {
            code: "state_drift",
            retryable: false,
            details: Some(json!({
                "reason": "physical_source_drift",
                "placement_id": drift.placement_id,
                "expected_path": drift.expected,
                "current_path": drift.current,
                "files_changed": false,
                "next_action": "scan"
            })),
        };
    }
    let (code, retryable) = classify_generic_error(error);
    ClassifiedError {
        code,
        retryable,
        details: None,
    }
}

fn classify_generic_error(error: &(dyn std::error::Error + 'static)) -> (&'static str, bool) {
    let message = error.to_string().to_lowercase();
    if error
        .downcast_ref::<crate::roster_plan::RosterPlanBlocked>()
        .is_some()
    {
        return ("trusted_canonical_sources_required", false);
    }
    if error.downcast_ref::<clap::Error>().is_some() {
        return ("invalid_cli_arguments", false);
    }
    if error.downcast_ref::<crate::model::InvalidId>().is_some() {
        return ("invalid_id", false);
    }
    if let Some(change) = error.downcast_ref::<crate::change::ChangeError>() {
        return (change.code, change.retryable);
    }
    if let Some(storage) = error.downcast_ref::<crate::sqlite::StorageError>() {
        return match storage {
            crate::sqlite::StorageError::Sql(rusqlite::Error::SqliteFailure(code, _))
                if matches!(
                    code.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) =>
            {
                ("state_store_busy", true)
            }
            crate::sqlite::StorageError::Sql(_) => ("state_store_error", false),
            crate::sqlite::StorageError::Json(_) => ("stored_data_invalid", false),
            crate::sqlite::StorageError::InvalidData(_) => ("state_invariant_failed", false),
        };
    }
    if let Some(io) = error.downcast_ref::<std::io::Error>() {
        if scan::is_non_unicode_identity_error(io) {
            return ("non_unicode_identity_path", false);
        }
        return match io.kind() {
            std::io::ErrorKind::PermissionDenied => ("path_permission_denied", false),
            std::io::ErrorKind::NotFound => ("path_not_found", false),
            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock => {
                ("io_temporarily_unavailable", true)
            }
            _ => ("io_error", false),
        };
    }
    if message.contains("no completed snapshot") {
        ("snapshot_required", false)
    } else if message.contains("recovery is required") {
        ("recovery_required", false)
    } else if message.contains("does not exist") {
        ("not_found", false)
    } else if message.contains("drift") || message.contains("fingerprint") {
        ("state_drift", false)
    } else if message.contains("cancelled") {
        ("cancelled", false)
    } else if message.contains("must be absolute")
        || message.contains("must be valid unicode")
        || message.contains("unsupported agent")
    {
        ("invalid_input", false)
    } else {
        ("command_failed", false)
    }
}

fn extract_relevant_ids(message: &str) -> Vec<String> {
    let mut ids = message
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|token| {
            [
                "scan_",
                "skill_",
                "placement_",
                "evidence_",
                "report_",
                "finding_",
                "plan_",
                "receipt_",
            ]
            .iter()
            .any(|prefix| token.starts_with(prefix) && token.len() > prefix.len())
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn extract_paths(message: &str) -> Vec<String> {
    let mut paths = message
        .split_whitespace()
        .map(|token| token.trim_matches(|character: char| "\"'(),:;".contains(character)))
        .filter(|token| token.starts_with('/'))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_io_classification_precedes_legacy_message_fallback() {
        let error =
            std::io::Error::new(std::io::ErrorKind::NotFound, "drift fingerprint cancelled");
        assert_eq!(classify_generic_error(&error), ("path_not_found", false));
    }

    #[test]
    fn content_identity_envelope_preserves_the_typed_reason() {
        let error = ContentIdentityRescanRequired {
            reason: "non_unicode_identity_coverage_incomplete",
        };

        let envelope: Value = serde_json::from_str(&error_json("find", &error)).unwrap();

        assert_eq!(
            envelope.pointer("/error/details/reason"),
            Some(&json!("non_unicode_identity_coverage_incomplete"))
        );
    }

    #[test]
    fn fallback_evidence_extraction_is_bounded_to_supported_shapes() {
        let message = "Plan plan_123 failed at /tmp/skill; ignore other-id and relative/path";
        assert_eq!(extract_relevant_ids(message), vec!["plan_123"]);
        assert_eq!(extract_paths(message), vec!["/tmp/skill"]);
    }
}
