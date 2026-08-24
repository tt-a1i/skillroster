use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::bootstrap::{PACKAGE_FILES as BOOTSTRAP_PACKAGE_FILES, content_version};
use crate::change::{
    self, ChangeReceipt, Operation, OperationPolicy, PrepareContext, PreparedPlan,
};
use crate::cli::{
    Cli, Command, LifecycleCommand, ModifiedBootstrapChoice, ReportCategory, ReportSeverity,
    SourceRootCommand,
};
use crate::harness::{self, AgentKind};
use crate::model::{
    AgentId, AgentRecord, ApiError, EvidenceId, EvidenceKind, EvidenceQuality, EvidenceRecord,
    FindingCategory, FindingId, FindingRecord, GovernanceState, JsonEnvelope, OperationAction,
    OperationId, OperationResult, PlacementId, PlacementKind, PlacementRecord, PlanId,
    PlanOperation, PlanRecord, PlanStatus, ReceiptId, ReceiptRecord, ReceiptStatus, ReportId,
    ReportRecord, RootId, RootRecord, RootStatus, RosterEntry, RosterState, ScanId, ScanRun,
    ScanStatus, Severity, SkillId, SkillRecord, SuggestedAction, UsageEvent, UsageStage,
};
use crate::scan::{self, ExplicitSkillRoot, RootKind, ScanOptions, ScanResult};
use crate::sqlite::StateStore;

const STATUS_PENDING_PLAN_LIMIT: usize = 20;
/// Agent tool-result transport bound, deliberately narrower than the 2 MiB
/// inventory parser bound. Larger Skills should disclose references on demand.
const MAX_AGENT_LOADED_SKILL_BYTES: u64 = 128 * 1024;

/// Global discovery and state options that a suggested action must retain to
/// operate on the same local analysis context as the command that produced it.
#[derive(Clone, Debug, Default)]
pub struct ActionContext {
    argv: Vec<String>,
}

impl ActionContext {
    pub fn from_cli(cli: &Cli) -> Result<Self> {
        let mut argv = Vec::new();
        if let Some(state_dir) = &cli.state_dir {
            let state_dir = if state_dir.is_absolute() {
                state_dir.clone()
            } else {
                std::path::absolute(state_dir).with_context(|| {
                    format!("cannot resolve --state-dir {}", state_dir.display())
                })?
            };
            argv.extend([
                "--state-dir".to_owned(),
                action_path(&state_dir, "--state-dir")?,
            ]);
        }
        if let Some(home) = &cli.home {
            argv.extend(["--home".to_owned(), action_path(home, "--home")?]);
        }
        for root in &cli.roots {
            argv.extend(["--root".to_owned(), root.clone()]);
        }
        for source_root in &cli.source_roots {
            argv.extend([
                "--source-root".to_owned(),
                action_path(source_root, "--source-root")?,
            ]);
        }
        Ok(Self { argv })
    }

    fn apply(&self, actions: &mut [SuggestedAction]) {
        if self.argv.is_empty() {
            return;
        }
        for action in actions {
            let insertion =
                usize::from(action.argv.first().is_some_and(|arg| arg == "skillroster"));
            action.argv.splice(insertion..insertion, self.argv.clone());
        }
    }

    fn argv(&self) -> &[String] {
        &self.argv
    }

    fn apply_json_argv(&self, argv: &mut Value) {
        let Some(values) = argv.as_array_mut() else {
            return;
        };
        if self.argv.is_empty() {
            return;
        }
        let insertion = usize::from(values.first().and_then(Value::as_str) == Some("skillroster"));
        values.splice(
            insertion..insertion,
            self.argv.iter().cloned().map(Value::String),
        );
    }

    fn apply_result(&self, command: &str, result: &mut Value) {
        match command {
            "find" => {
                if let Some(matches) = result.get_mut("matches").and_then(Value::as_array_mut) {
                    for found in matches {
                        if let Some(argv) = found.pointer_mut("/variant_finding/argv") {
                            self.apply_json_argv(argv);
                        }
                    }
                }
            }
            "report" => {
                if let Some(argv) =
                    result.pointer_mut("/resolution/after_confirmation/argv_template")
                {
                    self.apply_json_argv(argv);
                }
            }
            _ => {}
        }
    }
}

fn action_path(path: &Path, option: &str) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{option} path must be valid Unicode for suggested action argv"))
}

pub struct Output {
    pub json: String,
    pub human: String,
}

pub fn run(cli: Cli) -> Result<Output> {
    let action_context = ActionContext::from_cli(&cli)?;
    let home = resolve_home(cli.home)?;
    let state_dir = cli.state_dir.unwrap_or_else(|| home.join(".skillroster"));
    let database_path = state_dir.join("skillroster.db");
    if let Some(Command::Lifecycle(args)) = &cli.command {
        if let LifecycleCommand::Delete(args) = &args.command {
            if args.confirm != "DELETE-LOCAL-STATE" {
                bail!("database deletion requires --confirm DELETE-LOCAL-STATE");
            }
            if !cli.json
                && !require_human_confirmation(
                    &format!(
                        "Delete SkillRoster local state at {}?",
                        database_path.display()
                    ),
                    "The SQLite database, Receipt journals, and source-confirmation details will be deleted. Agent and Library files are preserved. A new Scan rebuilds inventory state.",
                )?
            {
                return cancelled_output("lifecycle");
            }
            let result = lifecycle_delete_command(&database_path, &state_dir)?;
            let envelope = JsonEnvelope::success("lifecycle", result.clone());
            return Ok(Output {
                json: serde_json::to_string(&envelope)?,
                human: crate::present::human("lifecycle", &result),
            });
        }
    }
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("cannot create {}", state_dir.display()))?;
    // Shared guards let Agent read/analysis commands run concurrently while
    // still excluding lifecycle deletion and filesystem mutations on Windows.
    let _state_lock = if command_requires_exclusive_state_lock(cli.command.as_ref()) {
        change::StateLock::acquire_exclusive(&state_dir)?
    } else {
        change::StateLock::acquire_shared(&state_dir)?
    };
    let store = StateStore::open(&database_path)?;

    let (command, mut result, warnings, actions) = match cli.command {
        None => ("home", home_result(&store, &state_dir)?, vec![], vec![]),
        Some(Command::Status) => {
            let result = status_result(&store, &database_path, &state_dir)?;
            let recovery_required = result["recovery_state"] == "required";
            let actions = if recovery_required {
                vec![action(
                    "inspect_recovery",
                    &["lifecycle", "recovery", "--json"],
                    false,
                    false,
                    "recovery_required",
                )]
            } else if result["latest_snapshot_id"].is_null() {
                vec![action(
                    "scan",
                    &["scan", "--json"],
                    false,
                    false,
                    "snapshot_required",
                )]
            } else if let Some(plan_id) = result["pending_plans"]
                .as_array()
                .and_then(|plans| plans.first())
                .and_then(|plan| plan["plan_id"].as_str())
            {
                vec![action(
                    "inspect_pending_plan",
                    &["plan", "--show", plan_id, "--json"],
                    false,
                    false,
                    "pending_plan_requires_review",
                )]
            } else {
                Vec::new()
            };
            ("status", result, vec![], actions)
        }
        Some(Command::SourceRoot(args)) => match args.command {
            SourceRootCommand::Confirm(args) => {
                if !args.path.is_absolute() {
                    bail!("source root must be absolute: {}", args.path.display());
                }
                if !cli.json
                    && !require_human_confirmation(
                        &format!("Permit local reads of exactly {}?", args.path.display()),
                        "This records one local read permission only. It does not endorse the content, raise Evidence quality, authorize a Plan, or change Agent/Skill files.",
                    )?
                {
                    return cancelled_output("source-root");
                }
                let finding_id = FindingId::parse(args.finding)?;
                let finding = store.get_finding(&finding_id)?.ok_or_else(|| {
                    crate::source_policy::SourceRootPolicyError::FindingNotFound {
                        finding_id: finding_id.to_string(),
                    }
                })?;
                let (_, snapshot) = latest_scan(&store)?;
                let outcome = crate::source_policy::confirm_source_root(
                    &store, &finding, &snapshot, &args.path,
                )?;
                let result = json!({
                    "operation": "confirm",
                    "permission": crate::source_policy::permission_json(&outcome.permission, None),
                    "already_permitted": outcome.already_permitted,
                    "permission_scope": "exact_local_read_only",
                    "content_endorsed": false,
                    "evidence_quality_changed": false,
                    "governance_authorized": false,
                    "plan_apply_authorized": false,
                    "local_state_changed": !outcome.already_permitted,
                    "state_files_changed": !outcome.already_permitted,
                    "agent_files_changed": false,
                    "skill_files_changed": false,
                    "files_changed": false,
                });
                (
                    "source-root",
                    result,
                    vec![],
                    vec![action(
                        "scan",
                        &["scan", "--json"],
                        false,
                        false,
                        "source_root_permission_recorded",
                    )],
                )
            }
            SourceRootCommand::Inspect(args) => {
                let mut result = crate::source_policy::policy_value(
                    &store,
                    true,
                    usize::from(args.limit),
                    usize::try_from(args.offset)?,
                )?;
                result["operation"] = json!("inspect");
                result["permission_scope"] = json!("exact_local_read_only");
                result["content_endorsed"] = json!(false);
                result["evidence_quality_changed"] = json!(false);
                result["governance_authorized"] = json!(false);
                result["files_changed"] = json!(false);
                result["state_files_changed"] = json!(false);
                let actions = result["next_offset"]
                    .as_u64()
                    .map(|offset| {
                        action(
                            "inspect_more_source_root_permissions",
                            &[
                                "source-root",
                                "inspect",
                                "--limit",
                                &args.limit.to_string(),
                                "--offset",
                                &offset.to_string(),
                                "--json",
                            ],
                            false,
                            false,
                            "more_source_root_permissions_available",
                        )
                    })
                    .into_iter()
                    .collect();
                ("source-root", result, vec![], actions)
            }
            SourceRootCommand::Revoke(args) => {
                if !cli.json
                    && !require_human_confirmation(
                        &format!("Revoke source-root read permission {}?", args.id),
                        "Future Scans will fail closed for this exact root unless separately permitted. Agent and Skill files are not changed.",
                    )?
                {
                    return cancelled_output("source-root");
                }
                let permission = crate::source_policy::revoke_permission(&store, &args.id)?;
                let result = json!({
                    "operation": "revoke",
                    "permission": crate::source_policy::permission_json(&permission, None),
                    "permission_scope": "exact_local_read_only",
                    "local_state_changed": true,
                    "state_files_changed": true,
                    "agent_files_changed": false,
                    "skill_files_changed": false,
                    "files_changed": false,
                });
                (
                    "source-root",
                    result,
                    vec![],
                    vec![action(
                        "scan",
                        &["scan", "--json"],
                        false,
                        false,
                        "source_root_permission_revoked",
                    )],
                )
            }
        },
        Some(Command::Scan) => {
            let (result, warnings) = scan_command(
                &store,
                &home,
                &state_dir,
                parse_explicit_roots(&cli.roots)?,
                parse_source_roots(&cli.source_roots)?,
            )?;
            (
                "scan",
                result,
                warnings,
                vec![action(
                    "report",
                    &["report", "--json"],
                    false,
                    false,
                    "scan_complete",
                )],
            )
        }
        Some(Command::Report(args)) => {
            let request = if let Some(id) = args.finding.as_deref() {
                ReportRequest::Finding {
                    id,
                    full: args.full,
                    limit: usize::from(args.limit),
                    offset: usize::try_from(args.offset)?,
                }
            } else if args.full {
                ReportRequest::Exhaustive
            } else if args.summary {
                ReportRequest::Summary
            } else if args.findings {
                ReportRequest::Findings {
                    category: args.category,
                    severity: args.severity,
                    limit: usize::from(args.limit),
                    offset: usize::try_from(args.offset)?,
                }
            } else {
                ReportRequest::Summary
            };
            let result = report_command(&store, &state_dir, request)?;
            let actions = report_actions(&result, request);
            ("report", result, vec![], actions)
        }
        Some(Command::Find(args)) => {
            let (result, actions) = find_command(
                &store,
                &state_dir,
                &args.task,
                &args.hints,
                usize::from(args.limit),
                args.load,
                args.variant_skill_id.as_deref(),
            )?;
            ("find", result, vec![], actions)
        }
        Some(Command::Plan(args)) => {
            let showing_detail = args.show.is_some();
            let result = match args.show.as_deref() {
                Some(id) => plan_detail_command(&store, id)?,
                None => plan_command(&store, &state_dir, action_context.argv())?,
            };
            let id = result["plan_id"].as_str().unwrap_or_default().to_string();
            let mut actions = Vec::new();
            if !showing_detail {
                actions.push(action(
                    "show_plan_detail",
                    &["plan", "--show", &id, "--json"],
                    false,
                    false,
                    "plan_detail_available",
                ));
            }
            if result["state"].as_str() == Some("ready") {
                actions.push(action(
                    "apply",
                    &["apply", &id, "--json"],
                    true,
                    true,
                    "plan_ready",
                ));
            }
            ("plan", result, vec![], actions)
        }
        Some(Command::Apply(args)) => {
            if !cli.json {
                eprintln!("{}\n", human_plan_preview(&store, &args.id)?);
                if !require_human_confirmation(
                    &format!("Apply Plan {} to the approved Agent roots?", args.id),
                    "The immutable Plan shown above will be applied; a Receipt will be written and no canonical Skill content will be deleted.",
                )? {
                    return cancelled_output("apply");
                }
            }
            let progress = crate::present::ProgressGuard::start("apply", cli.json);
            let result = apply_command(&store, &args.id)?;
            progress.finish();
            let id = result["receipt_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            (
                "apply",
                result,
                vec![],
                vec![action(
                    "undo",
                    &["undo", &id, "--json"],
                    true,
                    true,
                    "receipt_undoable",
                )],
            )
        }
        Some(Command::Undo(args)) => {
            if !cli.json {
                eprintln!("{}\n", human_undo_preview(&store, &args.id)?);
                if !require_human_confirmation(
                    &format!("Undo Receipt {}?", args.id),
                    "Only the exact changed paths shown above will be reversed; unrelated Agent and Library files are outside this Undo.",
                )? {
                    return cancelled_output("undo");
                }
            }
            let progress = crate::present::ProgressGuard::start("undo", cli.json);
            let result = undo_command(&store, &args.id)?;
            progress.finish();
            ("undo", result, vec![], vec![])
        }
        Some(Command::Lifecycle(args)) => match args.command {
            LifecycleCommand::Inspect => (
                "lifecycle",
                lifecycle_inspect_command(&store, &database_path, &state_dir)?,
                vec![],
                vec![],
            ),
            LifecycleCommand::Export(args) => (
                "lifecycle",
                lifecycle_export_command(&store, &state_dir, &args.output)?,
                vec![],
                vec![],
            ),
            LifecycleCommand::Exclude(args) => (
                "lifecycle",
                lifecycle_exclude_command(&store, &args.agent, args.remove)?,
                vec![],
                vec![],
            ),
            LifecycleCommand::Purge(args) => {
                if args.raw_days.is_none() && !args.plans_receipts && !args.source_confirmation {
                    bail!(
                        "purge requires --raw-days DAYS, --plans-receipts, and/or --source-confirmation"
                    );
                }
                if args.plans_receipts && args.confirm.as_deref() != Some("PURGE-PLANS-RECEIPTS") {
                    bail!("Plans and Receipts purge requires --confirm PURGE-PLANS-RECEIPTS");
                }
                if !cli.json
                    && !require_human_confirmation(
                        "Purge the explicitly selected local lifecycle state?",
                        if args.plans_receipts {
                            "Selected Plans, Receipts, and their Undo history will be deleted. Agent and Library files are preserved."
                        } else if args.source_confirmation {
                            "Selected source-confirmation details will be deleted. Agent and Library files are preserved."
                        } else {
                            "Only selected SQLite usage/evidence rows are affected; Plans, Receipts, Agent files, and Library files are preserved."
                        },
                    )?
                {
                    return cancelled_output("lifecycle");
                }
                (
                    "lifecycle",
                    lifecycle_purge_command(
                        &store,
                        &state_dir,
                        args.raw_days,
                        args.plans_receipts,
                        args.source_confirmation,
                    )?,
                    vec![],
                    vec![],
                )
            }
            LifecycleCommand::Recovery => (
                "lifecycle",
                lifecycle_recovery_command(&store, &state_dir)?,
                vec![],
                vec![],
            ),
            LifecycleCommand::Delete(_) => unreachable!("handled before opening SQLite"),
        },
        Some(Command::Setup(args)) => {
            let result = setup_command(&store, &home, &state_dir, args.modified_choice)?;
            let mut actions = Vec::new();
            if result["state"].as_str() == Some("scan_required") {
                actions.push(action(
                    "scan",
                    &["scan", "--json"],
                    false,
                    false,
                    "setup_requires_snapshot",
                ));
            } else if result["state"].as_str() == Some("modified_choice_required") {
                actions.push(action(
                    "retain_modified_bootstrap",
                    &["setup", "--modified-choice", "retain-local", "--json"],
                    false,
                    false,
                    "bootstrap_modified_choice_required",
                ));
                actions.push(action(
                    "adopt_current_bootstrap",
                    &["setup", "--modified-choice", "adopt-current", "--json"],
                    false,
                    false,
                    "bootstrap_modified_choice_required",
                ));
            }
            if let Some(plan_id) = result["plan_id"].as_str() {
                actions.push(action(
                    "apply",
                    &["apply", plan_id, "--json"],
                    true,
                    true,
                    "setup_plan_ready",
                ));
            }
            ("setup", result, vec![], actions)
        }
    };

    action_context.apply_result(command, &mut result);
    let mut envelope = JsonEnvelope::success(command, result.clone());
    envelope.warnings = warnings;
    envelope.suggested_actions = actions;
    action_context.apply(&mut envelope.suggested_actions);
    Ok(Output {
        json: serde_json::to_string(&envelope)?,
        human: crate::present::human(command, &result),
    })
}

fn command_requires_exclusive_state_lock(command: Option<&Command>) -> bool {
    match command {
        Some(Command::Scan | Command::Apply(_) | Command::Undo(_) | Command::Setup(_)) => true,
        Some(Command::Report(args)) => args.finding.is_none(),
        Some(Command::Plan(args)) => args.show.is_none(),
        Some(Command::SourceRoot(args)) => matches!(
            &args.command,
            SourceRootCommand::Confirm(_) | SourceRootCommand::Revoke(_)
        ),
        Some(Command::Lifecycle(args)) => matches!(
            &args.command,
            LifecycleCommand::Exclude(_)
                | LifecycleCommand::Purge(_)
                | LifecycleCommand::Recovery
                | LifecycleCommand::Delete(_)
        ),
        Some(Command::Find(_) | Command::Status) | None => false,
    }
}

pub fn error_json(command: &str, error: &(dyn std::error::Error + 'static)) -> String {
    error_json_with_context(command, error, &ActionContext::default())
}

pub fn error_json_with_context(
    command: &str,
    error: &(dyn std::error::Error + 'static),
    action_context: &ActionContext,
) -> String {
    if error
        .downcast_ref::<ContentIdentityRescanRequired>()
        .is_some()
    {
        let mut envelope = JsonEnvelope::<Value>::failure(
            command,
            ApiError {
                code: "content_identity_rescan_required".into(),
                message: error.to_string(),
                retryable: true,
                relevant_ids: Vec::new(),
                paths: Vec::new(),
                details: Some(json!({
                    "reason": "legacy_snapshot_requires_rescan",
                    "required_algorithm": scan::CONTENT_IDENTITY_ALGORITHM,
                    "files_changed": false,
                    "state_files_changed": false,
                    "next_action": "scan"
                })),
            },
        );
        envelope.suggested_actions = vec![action(
            "refresh_snapshot_content_identity",
            &["scan", "--json"],
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
            argv.extend(["scan", "--json"]);
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
            "same_name_variants_ambiguous" | "no_routable_match" => Some((
                "inspect_current_report",
                vec!["report", "--summary", "--json"],
            )),
            "placement_missing_from_snapshot"
            | "package_fingerprint_incomplete"
            | "legacy_snapshot_requires_rescan"
            | "eligible_placement_missing"
            | "entrypoint_content_drift"
            | "package_identity_drift" => Some(("refresh_snapshot", vec!["scan", "--json"])),
            _ => None,
        };
        if let Some((name, argv)) = safe_retry {
            envelope.suggested_actions = vec![action(
                name,
                &argv,
                false,
                false,
                "verified_skill_load_blocked",
            )];
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
struct PlanSnapshotDrift {
    plan_id: PlanId,
    expected_snapshot_id: ScanId,
    current_snapshot_id: ScanId,
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
struct StoredFindingCoverageInvalid {
    finding_id: FindingId,
    reason: &'static str,
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
struct LibraryRootConflict {
    library_root: PathBuf,
    agent_roots: Vec<PathBuf>,
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
struct IncompleteFingerprintBlocker {
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

fn incomplete_fingerprint_blocker<'a>(
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
                ("inspect_variant_finding", "read_only_command_available")
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

fn resolve_home(explicit: Option<PathBuf>) -> Result<PathBuf> {
    let home = explicit
        .or_else(|| directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
        .ok_or_else(|| anyhow!("cannot determine the user home directory"))?;
    if !home.is_absolute() {
        bail!("home directory must be absolute");
    }
    Ok(home)
}

fn require_human_confirmation(question: &str, consequence: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!(
            "human mutation requires an interactive terminal; Agent callers must use --json only after explicit user confirmation"
        );
    }
    eprintln!("{question}\n{consequence}\n\nType confirm to continue:");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(answer.trim() == "confirm")
}

fn cancelled_output(command: &'static str) -> Result<Output> {
    let result = json!({
        "status": "cancelled",
        "changed_path_count": 0,
        "verification": "not_run",
        "undo_available": false,
        "files_changed": false
    });
    let envelope = JsonEnvelope::success(command, result.clone());
    Ok(Output {
        json: serde_json::to_string(&envelope)?,
        human: crate::present::human(command, &result),
    })
}

fn human_plan_preview(store: &StateStore, id: &str) -> Result<String> {
    let id = PlanId::parse(id.to_string())?;
    let record = store
        .get_plan(&id)?
        .ok_or_else(|| anyhow!("Plan {id} does not exist"))?;
    let prepared: PreparedPlan = serde_json::from_value(record.input["prepared"].clone())?;
    if let Some(mut summary) = record.input.get("summary").cloned() {
        summary["state"] = json!(record.status);
        return Ok(crate::present::human("plan", &summary));
    }
    let risk = plan_risk(&prepared);
    Ok(crate::present::human(
        "plan",
        &json!({
            "plan_id": prepared.id,
            "operations": prepared.operations,
            "roster_changes": prepared.roster_changes,
            "risk": risk,
            "reversible": true,
            "canonical_deletion_count": 0,
            "blocked_preconditions": [],
            "state": record.status,
            "files_changed": false
        }),
    ))
}

fn human_undo_preview(store: &StateStore, id: &str) -> Result<String> {
    let id = ReceiptId::parse(id.to_string())?;
    let record = store
        .get_receipt(&id)?
        .ok_or_else(|| anyhow!("Receipt {id} does not exist"))?;
    let paths = record.verification["change_receipt"]["changed_paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let mut preview = format!(
        "SkillRoster · Undo Receipt\n\n  Receipt                {}\n  Plan                   {}\n  Recorded operations    {}\n  Changed paths          {}",
        record.id,
        record.plan_id,
        record.operation_results.len(),
        paths.len()
    );
    for path in paths {
        preview.push_str("\n    ");
        preview.push_str(path);
    }
    preview.push_str("\n\nPreview only · no files changed");
    Ok(preview)
}

fn parse_explicit_roots(values: &[String]) -> Result<Vec<ExplicitSkillRoot>> {
    values
        .iter()
        .map(|value| {
            let (agent, path) = value
                .split_once('=')
                .ok_or_else(|| anyhow!("root must use AGENT=PATH: {value}"))?;
            let agent = parse_agent_kind(agent)?;
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                bail!("explicit root must be absolute: {}", path.display());
            }
            Ok(ExplicitSkillRoot { agent, path })
        })
        .collect()
}

fn parse_source_roots(values: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut roots = values.to_vec();
    for path in &roots {
        if !path.is_absolute() {
            bail!("source root must be absolute: {}", path.display());
        }
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn parse_agent_kind(value: &str) -> Result<AgentKind> {
    AgentKind::ALL
        .into_iter()
        .find(|candidate| candidate.id() == value)
        .ok_or_else(|| anyhow!("unsupported Agent: {value}"))
}

fn home_result(store: &StateStore, state_dir: &Path) -> Result<Value> {
    Ok(json!({
        "state": if store.latest_completed_scan()?.is_some() { "ready" } else { "no_snapshot" },
        "recovery_state": recovery_text(store, state_dir)?,
        "files_changed": false
    }))
}

fn status_result(store: &StateStore, database_path: &Path, state_dir: &Path) -> Result<Value> {
    let latest = store.latest_completed_scan()?;
    let latest_scan_id = latest.as_ref().map(|scan| &scan.id);
    let (pending_plan_count, pending_plans) =
        store.pending_plans(latest_scan_id, STATUS_PENDING_PLAN_LIMIT)?;
    let pending_plans = pending_plans
        .into_iter()
        .map(|plan| {
            json!({
                "plan_id": plan.id,
                "snapshot_id": plan.scan_id,
                "status": plan.status,
                "created_at": plan.created_at,
            })
        })
        .collect::<Vec<_>>();
    let last_receipt = store.latest_receipt()?.map(|receipt| {
        json!({
            "receipt_id": receipt.id,
            "plan_id": receipt.plan_id,
            "status": receipt.status,
            "completed_at": receipt.completed_at,
        })
    });
    let lifecycle = store.lifecycle_counts()?;
    Ok(json!({
        "database_path": database_path,
        "schema_version": store.schema_version()?,
        "latest_snapshot_id": latest.as_ref().map(|scan| scan.id.to_string()),
        "latest_snapshot_at": latest.and_then(|scan| scan.completed_at),
        "pending_plan_count": pending_plan_count,
        "pending_plans_returned": pending_plans.len(),
        "pending_plans_truncated": pending_plan_count > pending_plans.len(),
        "pending_plans": pending_plans,
        "last_receipt": last_receipt,
        "recovery_state": recovery_text(store, state_dir)?,
        "journal_issues": journal_issues(store, state_dir)?,
        "retention": {
            "raw_usage_days": 180,
            "older_usage": "monthly_aggregates_retained",
            "automatic_purge": false,
            "source_confirmation_details": source_confirmation_detail_summary(state_dir)?,
            "current": lifecycle,
        },
        "source_root_permissions": crate::source_policy::policy_value(store, true, 100, 0)?,
        "files_changed": false
    }))
}

fn lifecycle_export_command(store: &StateStore, state_dir: &Path, output: &Path) -> Result<Value> {
    let source_confirmation_details = read_source_confirmation_details(state_dir)?;
    let export = json!({
        "schema_version": 2,
        "generated_at": Utc::now().timestamp(),
        "retention": {
            "raw_usage_days": 180,
            "older_usage": "monthly_aggregates_retained",
        },
        "usage_history": {
            "stable_identity_fields": [
                "skill_id", "agent_id", "stage", "quality", "source_path_digest",
                "observed_event_count", "occurred_at"
            ],
            "raw_value_field": "observed_event_count",
            "monthly_value_field": "max_observed_event_count",
            "aggregation": "maximum observation per source and month",
            "observations_additive": false,
            "legacy_monthly_combinable": false,
        },
        "data": store.export_lifecycle()?,
        "evidence_exclusions": store.evidence_exclusions()?,
        "source_confirmation_details": source_confirmation_details,
        "source_root_permissions": crate::source_policy::policy_export_value(store)?,
        "privacy": "derived summaries only; no raw conversation text",
    });
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        bail!(
            "export parent directory does not exist: {}",
            parent.display()
        );
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .with_context(|| {
            format!(
                "cannot create export {} (existing files are never overwritten)",
                output.display()
            )
        })?;
    serde_json::to_writer_pretty(&mut file, &export)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(json!({
        "operation": "export",
        "output_path": output,
        "counts": store.lifecycle_counts()?,
        "source_confirmation_detail_count": source_confirmation_details.len(),
        "source_root_permission_count": store.list_source_root_permissions()?.len(),
        "files_changed": true,
    }))
}

fn lifecycle_inspect_command(
    store: &StateStore,
    database_path: &Path,
    state_dir: &Path,
) -> Result<Value> {
    let mut counts = serde_json::to_value(store.lifecycle_counts()?)?;
    let source_confirmation_details = source_confirmation_detail_summary(state_dir)?;
    counts["source_confirmation_details"] = source_confirmation_details["count"].clone();
    Ok(json!({
        "operation": "inspect",
        "database_path": database_path,
        "counts": counts,
        "source_confirmation_details": source_confirmation_details,
        "source_root_permissions": crate::source_policy::policy_value(store, true, 100, 0)?,
        "evidence_exclusions": store.evidence_exclusions()?,
        "recovery_state": recovery_text(store, state_dir)?,
        "privacy": "derived summaries only; no raw conversation text",
        "files_changed": false,
    }))
}

fn lifecycle_exclude_command(store: &StateStore, agent: &str, remove: bool) -> Result<Value> {
    let agent = parse_agent_kind(agent)?;
    store.set_evidence_exclusion(agent.id(), !remove)?;
    Ok(json!({
        "operation": if remove { "exclusion_removed" } else { "exclude" },
        "agent": agent.id(),
        "scope": "future_session_evidence_scans",
        "raw_conversations_copied": false,
        "evidence_exclusions": store.evidence_exclusions()?,
        "agent_files_changed": false,
        "library_files_changed": false,
        "files_changed": false,
    }))
}

fn lifecycle_purge_command(
    store: &StateStore,
    state_dir: &Path,
    raw_days: Option<u16>,
    plans_receipts: bool,
    source_confirmation: bool,
) -> Result<Value> {
    if plans_receipts && recovery_text(store, state_dir)? == "required" {
        bail!("recovery is required before Plans and Receipts can be purged");
    }
    if source_confirmation {
        source_confirmation_detail_paths(state_dir)?;
    }
    let (cutoff, usage_result) = if let Some(raw_days) = raw_days {
        let cutoff = Utc::now().timestamp() - i64::from(raw_days) * 24 * 60 * 60;
        (Some(cutoff), Some(store.purge_usage_before(cutoff)?))
    } else {
        (None, None)
    };
    let usage_changed = usage_result.as_ref().is_some_and(|result| {
        result.aggregated_raw_usage_rows > 0
            || result.deleted_raw_usage_rows > 0
            || result.deleted_evidence_rows > 0
            || result.deleted_payload_usage_summaries > 0
    });
    let (plan_receipt_result, plans_or_receipts_changed) = if plans_receipts {
        let result = store.purge_plans_and_receipts()?;
        let removed_journals = remove_receipt_journals(state_dir)?;
        let removed_recovery_directories = remove_recovery_artifacts(state_dir)?;
        let changed = result.plans > 0
            || result.receipts > 0
            || removed_journals > 0
            || removed_recovery_directories > 0;
        (
            Some(json!({
                "plans": result.plans,
                "receipts": result.receipts,
                "receipt_journals": removed_journals,
                "recovery_directories": removed_recovery_directories,
            })),
            changed,
        )
    } else {
        (None, false)
    };
    let removed_source_confirmation_details = if source_confirmation {
        remove_source_confirmation_details(state_dir)?
    } else {
        0
    };
    Ok(json!({
        "operation": "purge",
        "raw_usage_days": raw_days,
        "cutoff": cutoff,
        "usage_result": usage_result,
        "plan_receipt_result": plan_receipt_result,
        "monthly_aggregates_retained": raw_days.is_some(),
        "plans_or_receipts_changed": plans_or_receipts_changed,
        "removed_source_confirmation_details": removed_source_confirmation_details,
        "agent_files_changed": false,
        "library_files_changed": false,
        "files_changed": usage_changed || plans_or_receipts_changed || removed_source_confirmation_details > 0,
    }))
}

fn lifecycle_delete_command(database_path: &Path, state_dir: &Path) -> Result<Value> {
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("cannot create {}", state_dir.display()))?;
    let _write_lock = change::WriteLock::acquire(state_dir)?;
    let existed = database_path.exists();
    let journals = change::journals(state_dir)?;
    source_confirmation_detail_paths(state_dir)?;
    if existed {
        let store = StateStore::open(database_path)?;
        if recovery_text(&store, state_dir)? == "required" {
            bail!("recovery is required before the local state database can be deleted");
        }
        drop(store);
    } else if !journals.is_empty() {
        bail!("recovery is required before the local state database can be deleted");
    }
    let mut removed_database_files = Vec::new();
    for path in [
        database_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", database_path.display())),
        PathBuf::from(format!("{}-shm", database_path.display())),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => removed_database_files.push(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("cannot delete {}", path.display()));
            }
        }
    }
    // Once SQLite is gone, leftover journals or recovery backups are harmless
    // extra local state and can be retried. Deleting them first could destroy
    // the only recovery material before a database deletion error.
    let removed_journals = remove_receipt_journals(state_dir)?;
    let removed_recovery_directories = remove_recovery_artifacts(state_dir)?;
    let removed_source_confirmation_details = remove_source_confirmation_details(state_dir)?;
    let files_changed = !removed_database_files.is_empty()
        || removed_journals > 0
        || removed_recovery_directories > 0
        || removed_source_confirmation_details > 0;
    Ok(json!({
        "operation": "delete_local_state",
        "database_path": database_path,
        "database_existed": existed,
        "removed_database_files": removed_database_files,
        "removed_receipt_journals": removed_journals,
        "removed_recovery_directories": removed_recovery_directories,
        "removed_source_confirmation_details": removed_source_confirmation_details,
        "rebuild_command": "skillroster scan --json",
        "agent_files_changed": false,
        "library_files_changed": false,
        "files_changed": files_changed,
    }))
}

fn source_confirmation_detail_paths(state_dir: &Path) -> Result<Vec<PathBuf>> {
    let directory = state_dir.join("source-confirmation");
    let metadata = match std::fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", directory.display()));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "refusing invalid source-confirmation directory: {}",
            directory.display()
        );
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || path.extension().and_then(|value| value.to_str()) != Some("json")
        {
            bail!(
                "refusing invalid source-confirmation detail: {}",
                path.display()
            );
        }
        validate_source_confirmation_detail(&path)?;
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

fn validate_source_confirmation_detail(path: &Path) -> Result<()> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            anyhow!(
                "invalid source-confirmation detail name: {}",
                path.display()
            )
        })?;
    ulid::Ulid::from_string(stem).map_err(|_| {
        anyhow!(
            "invalid source-confirmation detail name: {}",
            path.display()
        )
    })?;
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let detail: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("cannot parse {}", path.display()))?;
    if !recognized_source_confirmation_detail(&detail) {
        bail!(
            "refusing unrecognized source-confirmation detail: {}",
            path.display()
        );
    }
    Ok(())
}

fn recognized_source_confirmation_detail(detail: &Value) -> bool {
    (|| -> Option<bool> {
        let schema_version = detail["schema_version"].as_u64()?;
        let core_budget = detail["requested_core_budget"].as_u64()?;
        let blocked_changes = detail["blocked_changes"].as_array()?;
        let blocked_change_count = detail["blocked_change_count"].as_u64()?;
        let skill_ids = detail["skill_ids"]
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?;
        let blocker_facts = blocked_changes
            .iter()
            .map(|blocker| {
                let agent = blocker["agent"].as_str()?;
                parse_agent_kind(agent).ok()?;
                let skill_id = blocker["skill_id"].as_str()?;
                SkillId::parse(skill_id.to_owned()).ok()?;
                (!blocker["name"].as_str()?.trim().is_empty()).then_some(())?;
                match blocker["reason"].as_str()? {
                    "no_owned_exact_content_to_preserve" => {}
                    "untrusted_external_placement_blocks_mutation" => {
                        let scopes = blocker["mutation_scopes"].as_array()?;
                        (scopes.len() == 1 && scopes[0] == "untrusted_external").then_some(())?;
                    }
                    _ => return None,
                }
                (blocker["state"] == "unchanged").then_some(())?;
                let observed_source_target =
                    if let Some(target) = blocker.get("observed_source_target") {
                        let target = Path::new(target.as_str()?);
                        (target.is_absolute() && target.parent().is_some()).then_some(())?;
                        Some(target.to_path_buf())
                    } else {
                        None
                    };
                Some((skill_id, observed_source_target))
            })
            .collect::<Option<Vec<_>>>()?;
        let blocker_skill_ids = blocker_facts
            .iter()
            .map(|(skill_id, _)| *skill_id)
            .collect::<Vec<_>>();
        let source_roots = detail["source_roots"]
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?;
        let source_root_count = detail["source_root_count"].as_u64()?;
        let expected_source_roots = crate::roster_plan::minimum_reviewed_source_roots(
            blocker_facts
                .iter()
                .filter_map(|(_, target)| target.clone()),
        )
        .into_iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>();
        let action_context_argv = match schema_version {
            1 => Vec::new(),
            2 => detail["action_context_argv"]
                .as_array()?
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()?,
            _ => return None,
        };
        recognized_action_context_argv(&action_context_argv).then_some(())?;
        let mut expected_argv = vec!["skillroster"];
        expected_argv.extend(action_context_argv);
        for root in &source_roots {
            expected_argv.extend(["--source-root", *root]);
        }
        expected_argv.extend(["scan", "--json"]);
        let argv = detail["after_confirmation"]["argv"]
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?;
        Some(
            matches!(schema_version, 1 | 2)
                && detail["reason"] == "trusted_canonical_sources_required"
                && detail["decision"] == "confirm_trusted_source_roots"
                && (1..=crate::roster_recommendation::MAX_CORE_BUDGET as u64)
                    .contains(&core_budget)
                && blocked_change_count == blocked_changes.len() as u64
                && blocker_skill_ids == skill_ids
                && source_root_count == source_roots.len() as u64
                && source_roots == expected_source_roots
                && detail["after_confirmation"]["repeatable_option"] == "--source-root"
                && detail["after_confirmation"]["source_roots"] == detail["source_roots"]
                && argv == expected_argv,
        )
    })()
    .unwrap_or(false)
}

fn recognized_action_context_argv(argv: &[&str]) -> bool {
    if argv.len() % 2 != 0 {
        return false;
    }
    let mut state_dir_seen = false;
    let mut home_seen = false;
    argv.chunks_exact(2).all(|pair| match pair {
        ["--state-dir", path] => {
            let unique = !state_dir_seen;
            state_dir_seen = true;
            unique && Path::new(path).is_absolute()
        }
        ["--home", path] => {
            let unique = !home_seen;
            home_seen = true;
            unique && Path::new(path).is_absolute()
        }
        ["--root", root] => root.split_once('=').is_some_and(|(agent, path)| {
            parse_agent_kind(agent).is_ok() && Path::new(path).is_absolute()
        }),
        ["--source-root", path] => Path::new(path).is_absolute(),
        _ => false,
    })
}

fn source_confirmation_detail_summary(state_dir: &Path) -> Result<Value> {
    let paths = source_confirmation_detail_paths(state_dir)?;
    let bytes = paths.iter().try_fold(0_u64, |total, path| {
        Ok::<_, anyhow::Error>(total + std::fs::symlink_metadata(path)?.len())
    })?;
    Ok(json!({
        "count": paths.len(),
        "bytes": bytes,
        "retention": "until_explicit_purge_or_delete",
    }))
}

fn read_source_confirmation_details(state_dir: &Path) -> Result<Vec<Value>> {
    source_confirmation_detail_paths(state_dir)?
        .into_iter()
        .map(|path| {
            let bytes =
                std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
            serde_json::from_slice(&bytes)
                .with_context(|| format!("cannot parse {}", path.display()))
        })
        .collect()
}

fn remove_source_confirmation_details(state_dir: &Path) -> Result<u64> {
    let directory = state_dir.join("source-confirmation");
    let paths = source_confirmation_detail_paths(state_dir)?;
    for path in &paths {
        std::fs::remove_file(path).with_context(|| format!("cannot delete {}", path.display()))?;
    }
    if std::fs::symlink_metadata(&directory).is_ok() {
        std::fs::remove_dir(&directory)
            .with_context(|| format!("cannot delete {}", directory.display()))?;
    }
    Ok(paths.len() as u64)
}

fn remove_receipt_journals(state_dir: &Path) -> Result<u64> {
    change::journals(state_dir)?;
    let directory = state_dir.join("receipts");
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", directory.display()));
        }
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!(
                "refusing to delete non-file Receipt journal: {}",
                path.display()
            );
        }
        std::fs::remove_file(&path)?;
        removed += 1;
    }
    Ok(removed)
}

fn remove_recovery_artifacts(state_dir: &Path) -> Result<u64> {
    let directory = state_dir.join("recovery");
    let metadata = match std::fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", directory.display()));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "refusing to delete invalid recovery directory: {}",
            directory.display()
        );
    }
    let mut removed = 0;
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if ReceiptId::parse(name).is_err() {
            continue;
        }
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!(
                "refusing to delete invalid Receipt recovery path: {}",
                path.display()
            );
        }
        std::fs::remove_dir_all(&path)
            .with_context(|| format!("cannot delete {}", path.display()))?;
        removed += 1;
    }
    Ok(removed)
}

fn lifecycle_recovery_command(store: &StateStore, state_dir: &Path) -> Result<Value> {
    let mut imported = Vec::new();
    let mut import_errors = Vec::new();
    for mut journal in change::journals(state_dir)? {
        let receipt_id = ReceiptId::parse(journal.id.clone())?;
        let plan_id = match PlanId::parse(journal.plan_id.clone()) {
            Ok(id) => id,
            Err(error) => {
                import_errors.push(json!({"receipt_id": journal.id, "error": error.to_string()}));
                continue;
            }
        };
        let Some(plan) = store.get_plan(&plan_id)? else {
            import_errors.push(json!({
                "receipt_id": journal.id,
                "error": "journal Plan is not present in SQLite; left unmodified"
            }));
            continue;
        };
        let prepared: PreparedPlan = match serde_json::from_value(plan.input["prepared"].clone()) {
            Ok(prepared) => prepared,
            Err(error) => {
                import_errors.push(json!({"receipt_id": journal.id, "error": error.to_string()}));
                continue;
            }
        };
        journal.status = change::ReceiptStatus::RecoveryRequired;
        journal.error = Some(match journal.error.take() {
            Some(error) => format!("{error}; imported conservatively from filesystem journal"),
            None => {
                "imported conservatively from filesystem journal after missing SQLite finalization"
                    .to_owned()
            }
        });
        let operation_ids = plan
            .operations
            .iter()
            .map(|operation| (operation.position, operation.id.clone()))
            .collect::<Vec<_>>();
        let reverses = journal
            .reverses_receipt_id
            .clone()
            .map(ReceiptId::parse)
            .transpose()?;
        let imported_receipt = receipt_record(
            &journal,
            &operation_ids,
            reverses,
            false,
            json!({
                "before": plan.input["roster_before"].clone(),
                "after": prepared.roster_changes,
            }),
            json!(prepared.source_updates),
            json!(prepared.evidence_ids),
            json!({
                "before": plan.input["library_before"].clone(),
                "after": prepared.library_changes,
            }),
        )?;
        match store.save_recovery_receipt(&plan_id, &imported_receipt) {
            Ok(inserted) => {
                if inserted {
                    imported.push(receipt_id);
                }
            }
            Err(error) => import_errors.push(json!({
                "receipt_id": journal.id,
                "error": error.to_string(),
            })),
        }
    }
    let receipts = store
        .recovery_receipts()?
        .into_iter()
        .map(|receipt| {
            let change_receipt = &receipt.verification["change_receipt"];
            json!({
                "receipt_id": receipt.id,
                "plan_id": receipt.plan_id,
                "created_at": receipt.created_at,
                "completed_at": receipt.completed_at,
                "error": change_receipt["error"],
                "changed_paths": change_receipt["changed_paths"],
            })
        })
        .collect::<Vec<_>>();
    let journals = journal_issues(store, state_dir)?;
    Ok(json!({
        "operation": "recovery_inspect",
        "recovery_state": if receipts.is_empty() && journals.is_empty() { "clear" } else { "required" },
        "receipts": receipts,
        "journal_issues": journals,
        "imported_receipt_ids": imported,
        "import_errors": import_errors,
        "automatic_resolution_available": false,
        "resolution_note": "Orphan journals with an existing immutable Plan are imported as recovery_required, never guessed successful. Inspect exact paths before repair.",
        "state_changed": !imported.is_empty(),
        "files_changed": false,
    }))
}

fn scan_command(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    explicit: Vec<ExplicitSkillRoot>,
    source_roots: Vec<PathBuf>,
) -> Result<(Value, Vec<String>)> {
    let started = Utc::now().timestamp();
    let id = ScanId::new();
    // Freeze durable read permissions once before discovery. Only exact roots
    // whose path and stable filesystem identity still match enter the Scan;
    // drift is local to that permission and persists as typed Snapshot facts.
    let frozen_permissions = crate::source_policy::freeze_active_roots(store)?;
    let durable_read_roots = frozen_permissions
        .iter()
        .filter(|root| root.state == crate::source_policy::SourceRootState::Active)
        .filter_map(|root| {
            root.resolved_path
                .clone()
                .map(|path| crate::scan::DurableReadRoot {
                    permission_id: root.permission.id.as_str().to_owned(),
                    path,
                    identity: root.permission.identity.clone(),
                })
        })
        .collect::<Vec<_>>();
    store.save_scan(&ScanRun {
        id: id.clone(),
        started_at: started,
        completed_at: None,
        status: ScanStatus::Running,
        coverage_notes: vec![],
    })?;
    let mut options = ScanOptions::for_home(home);
    options.explicit_skill_roots = explicit;
    options.explicit_source_roots = source_roots;
    options.durable_read_roots = durable_read_roots;
    options.managed_source_roots = vec![state_dir.join("library")];
    options.excluded_session_agents = store
        .evidence_exclusions()?
        .iter()
        .map(|agent| parse_agent_kind(agent))
        .collect::<Result<_>>()?;
    let mut result = match scan::scan(&options) {
        Ok(result) => result,
        Err(error) => {
            store.save_scan(&ScanRun {
                id,
                started_at: started,
                completed_at: Some(Utc::now().timestamp()),
                status: ScanStatus::Failed,
                coverage_notes: vec![error.to_string()],
            })?;
            return Err(error.into());
        }
    };
    let post_scan_permissions = match crate::source_policy::freeze_active_roots(store) {
        Ok(permissions) => permissions,
        Err(error) => {
            store.save_scan(&ScanRun {
                id,
                started_at: started,
                completed_at: Some(Utc::now().timestamp()),
                status: ScanStatus::Failed,
                coverage_notes: vec![error.to_string()],
            })?;
            return Err(error.into());
        }
    };
    let policy_facts = conservative_source_policy_facts(
        &frozen_permissions,
        &post_scan_permissions,
        &result.durable_read_drifted_permission_ids,
    );
    for fact in policy_facts
        .iter()
        .filter(|fact| fact.state != crate::source_policy::SourceRootState::Active)
    {
        result.warnings.push(format!(
            "source-root permission {} is {} and was excluded: {}",
            fact.permission_id,
            serde_json::to_value(fact.state)?
                .as_str()
                .unwrap_or("drifted"),
            fact.drift_reason.as_deref().unwrap_or("identity drift")
        ));
    }
    result.source_root_policy = policy_facts;
    store.begin_scan_snapshot()?;
    let persistence = (|| -> Result<()> {
        persist_index(store, &id, &result)?;
        store.save_scan_payload(&id, &result)?;
        store.save_scan(&ScanRun {
            id: id.clone(),
            started_at: started,
            completed_at: Some(Utc::now().timestamp()),
            status: ScanStatus::Completed,
            coverage_notes: result.warnings.clone(),
        })?;
        store.commit_scan_snapshot()?;
        Ok(())
    })();
    if let Err(error) = persistence {
        let rollback_error = store.rollback_scan_snapshot().err();
        store.save_scan(&ScanRun {
            id,
            started_at: started,
            completed_at: Some(Utc::now().timestamp()),
            status: ScanStatus::Failed,
            coverage_notes: vec![error.to_string()],
        })?;
        if let Some(rollback_error) = rollback_error {
            bail!("Scan persistence failed: {error}; rollback failed: {rollback_error}");
        }
        return Err(error);
    }
    let agents_checked = result
        .roots
        .iter()
        .filter_map(|root| root.agent)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let roots = result
        .roots
        .iter()
        .map(|root| {
            let mut value = json!(root);
            value["agent"] = json!(root.agent.map(AgentKind::id));
            value
        })
        .collect::<Vec<_>>();
    let coverage = result
        .coverage
        .iter()
        .map(|coverage| {
            let mut value = json!(coverage);
            value["agent"] = json!(coverage.agent.id());
            value
        })
        .collect::<Vec<_>>();
    let warnings = compact_scan_warnings(result.warnings);
    Ok((
        json!({
            "snapshot_id": id,
            "agents_checked": agents_checked,
            "skill_count": result.skills.len(),
            "placement_count": result.placements.len(),
            "roots": roots,
            "coverage": coverage,
            "source_root_policy": {
                "scope": "exact_local_read_only",
                "content_endorsed": false,
                "evidence_quality_changed": false,
                "governance_authorized": false,
                "permissions": result.source_root_policy,
            },
            "files_changed": false
        }),
        warnings,
    ))
}

/// Merge policy observations conservatively across a Scan. Once a permission
/// is observed drifted at any bounded checkpoint, a later path check cannot
/// promote it back to Active. This also carries drift detected inside Scan
/// discovery/consumption into the persisted Snapshot fact.
fn conservative_source_policy_facts(
    before: &[crate::source_policy::FrozenSourceRoot],
    after: &[crate::source_policy::FrozenSourceRoot],
    scan_drifted_permission_ids: &BTreeSet<String>,
) -> Vec<crate::source_policy::SourceRootPolicyFact> {
    let before_by_id = before
        .iter()
        .map(|root| (root.permission.id.as_str(), root))
        .collect::<BTreeMap<_, _>>();
    after
        .iter()
        .map(|root| {
            let id = root.permission.id.as_str();
            let mut fact = crate::source_policy::fact_from_frozen(root);
            if let Some(previous) = before_by_id.get(id) {
                if previous.state != crate::source_policy::SourceRootState::Active
                    && fact.state == crate::source_policy::SourceRootState::Active
                {
                    fact = crate::source_policy::fact_from_frozen(previous);
                }
            }
            if scan_drifted_permission_ids.contains(id) {
                fact.state = crate::source_policy::SourceRootState::Inaccessible;
                fact.resolved_path = None;
                fact.drift_reason = Some(
                    "drift detected during bounded Scan checks; source root was excluded".into(),
                );
            }
            fact
        })
        .collect()
}

fn compact_scan_warnings(warnings: Vec<String>) -> Vec<String> {
    let (unsafe_links, mut remaining): (Vec<_>, Vec<_>) = warnings
        .into_iter()
        .partition(|warning| warning.starts_with("did not read unsafe Skill link "));
    if !unsafe_links.is_empty() {
        remaining.insert(
            0,
            format!(
                "{} unsafe Skill links were not read; inspect the layout Finding for paths and link targets",
                unsafe_links.len()
            ),
        );
    }
    remaining
}

fn usage_finding_evidence_priority(evidence: &EvidenceRecord) -> u8 {
    if evidence.kind == EvidenceKind::Coverage {
        return 0;
    }
    if evidence.kind != EvidenceKind::Usage {
        return 5;
    }
    let exposed = evidence.details["stage"].as_str() == Some("exposed");
    match (&evidence.quality, exposed) {
        (EvidenceQuality::Observed, false) => 1,
        (EvidenceQuality::Observed, true) => 2,
        (_, false) => 3,
        (_, true) => 4,
    }
}

#[derive(Clone, Copy)]
enum ReportRequest<'a> {
    Summary,
    Findings {
        category: Option<ReportCategory>,
        severity: Option<ReportSeverity>,
        limit: usize,
        offset: usize,
    },
    Finding {
        id: &'a str,
        full: bool,
        limit: usize,
        offset: usize,
    },
    Exhaustive,
}

fn report_command(
    store: &StateStore,
    state_dir: &Path,
    request: ReportRequest<'_>,
) -> Result<Value> {
    if let ReportRequest::Finding {
        id,
        full,
        limit,
        offset,
    } = request
    {
        let id = FindingId::parse(id.to_string())?;
        let stored = store
            .get_finding(&id)?
            .ok_or_else(|| anyhow!("Finding {id} does not exist"))?;
        let mut details = stored.details.clone();
        if let Some(object) = details.as_object_mut() {
            object.insert("report_id".into(), json!(stored.report_id));
            object.entry("kind").or_insert_with(|| {
                json!(crate::roster_recommendation::finding_kind(
                    &stored.category,
                    &stored.title
                ))
            });
            object.insert("files_changed".into(), json!(false));
            let report = store
                .get_report(&stored.report_id)?
                .ok_or_else(|| anyhow!("Report {} does not exist", stored.report_id))?;
            let scan: ScanResult = store
                .scan_payload(&report.scan_id)?
                .ok_or_else(|| anyhow!("Snapshot {} is no longer retained", report.scan_id))?;
            require_content_identity(&scan)?;
            let coverage_basis = stored_finding_coverage_basis(&stored, object)?;
            let evidence_quality = object
                .get("evidence_quality")
                .cloned()
                .unwrap_or_else(|| json!("unknown"));
            object.insert(
                "coverage".into(),
                finding_coverage_facts(coverage_basis, evidence_quality, &scan),
            );
            let latest_scan_id = store
                .latest_completed_scan()?
                .ok_or_else(|| anyhow!("no completed Snapshot exists"))?
                .id;
            if let Some(planning) =
                finding_library_planning(&stored, &report.scan_id, &latest_scan_id, &scan)
            {
                object.insert("planning".into(), planning);
            } else if let Some(planning) = finding_roster_planning(
                store,
                &stored,
                &report.scan_id,
                &latest_scan_id,
                &scan,
                full,
            )? {
                object.insert("planning".into(), planning);
            }
            add_semantic_overlap_comparison(object, &scan, state_dir)?;
            add_same_name_resolution(object, &scan);
            let affected_skill_ids = object
                .get("affected_skill_ids")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let affected_placement_ids = object
                .get("affected_placement_ids")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let ordered_evidence = if stored.title == "Five-stage usage evidence" {
                let mut evidence = store.finding_evidence(&id)?;
                evidence.sort_by(|left, right| {
                    usage_finding_evidence_priority(left)
                        .cmp(&usage_finding_evidence_priority(right))
                        .then_with(|| {
                            left.details["agent"]
                                .as_str()
                                .cmp(&right.details["agent"].as_str())
                        })
                        .then_with(|| {
                            left.details["skill_id"]
                                .as_str()
                                .cmp(&right.details["skill_id"].as_str())
                        })
                        .then_with(|| {
                            left.details["stage"]
                                .as_str()
                                .cmp(&right.details["stage"].as_str())
                        })
                        .then_with(|| left.id.as_str().cmp(right.id.as_str()))
                });
                evidence
            } else {
                Vec::new()
            };
            let ordered_evidence_ids = if ordered_evidence.is_empty() {
                stored.evidence_ids.clone()
            } else {
                ordered_evidence
                    .iter()
                    .map(|evidence| evidence.id.clone())
                    .collect()
            };
            let end = offset.saturating_add(limit);
            let paged_skill_ids = affected_skill_ids
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            let paged_placement_ids = affected_placement_ids
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            let paged_evidence_ids = ordered_evidence_ids
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            let mut placements = scan
                .placements
                .iter()
                .filter(|placement| {
                    paged_placement_ids
                        .iter()
                        .any(|id| id.as_str() == Some(&placement.id))
                })
                .map(|placement| {
                    json!({
                        "id": placement.id,
                        "skill_id": placement.skill_id,
                        "agent": placement.agent.map(AgentKind::id),
                        "path": placement.entrypoint,
                        "root": placement.root,
                        "link_target": placement.link_target,
                        "link_status": placement.link_status,
                        "default_exposed": placement.default_exposed,
                        "owned_by_agent": placement.owned_by_agent,
                        "mutation_scope": placement.mutation_scope,
                        "governable": placement.is_mutable(),
                        "provider": placement.provider,
                        "content_digest": placement.content_digest,
                        "fingerprint_completeness": placement.fingerprint_completeness,
                        "fingerprint_detail": placement.fingerprint_detail
                    })
                })
                .collect::<Vec<_>>();
            placements.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
            let evidence = if ordered_evidence.is_empty() {
                paged_evidence_ids
                    .iter()
                    .map(|id| store.get_evidence(id))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            } else {
                ordered_evidence
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .collect()
            };
            let total = affected_skill_ids
                .len()
                .max(affected_placement_ids.len())
                .max(ordered_evidence_ids.len());
            let next_offset = (end < total).then_some(end);
            object.insert("affected_skill_ids".into(), json!(paged_skill_ids));
            object.insert("affected_placement_ids".into(), json!(paged_placement_ids));
            object.insert("evidence_ids".into(), json!(paged_evidence_ids));
            object.insert(
                "primary_evidence_id".into(),
                json!(ordered_evidence_ids.first()),
            );
            object.insert("placements".into(), json!(placements));
            object.insert("evidence".into(), json!(evidence));
            object.insert(
                "page".into(),
                json!({
                    "offset": offset,
                    "limit": limit,
                    "next_offset": next_offset,
                    "has_more": next_offset.is_some(),
                    "totals": {
                        "affected_skills": affected_skill_ids.len(),
                        "affected_placements": affected_placement_ids.len(),
                        "evidence": ordered_evidence_ids.len()
                    },
                    "returned": {
                        "affected_skills": object["affected_skill_ids"].as_array().map_or(0, Vec::len),
                        "affected_placements": object["affected_placement_ids"].as_array().map_or(0, Vec::len),
                        "evidence": object["evidence_ids"].as_array().map_or(0, Vec::len)
                    }
                }),
            );
            add_finding_resolution(object);
            object.insert(
                "detail".into(),
                json!({
                    "mode": if full { "full" } else { "compact" },
                    "full_available": !full
                }),
            );
        }
        return Ok(if full {
            details
        } else {
            compact_finding_detail(details)
        });
    }
    let (scan_id, scan): (ScanId, ScanResult) = latest_scan(store)?;
    require_content_identity(&scan)?;
    if let Some(existing) = store.latest_report()? {
        if existing.scan_id == scan_id {
            return Ok(select_report_view(&existing.summary, request));
        }
    }
    let report = crate::query::build_report(&scan);
    let report_id = ReportId::new();
    let findings = report
        .findings
        .iter()
        .map(|finding| -> Result<FindingRecord> {
            let id = FindingId::new();
            let evidence_ids = finding
                .evidence
                .iter()
                .map(|reference| evidence_id(&scan_id, reference))
                .collect::<Result<Vec<_>>>()?;
            Ok(FindingRecord {
                details: finding_json(&id, finding, &evidence_ids, &scan),
                id,
                report_id: report_id.clone(),
                category: finding_category(finding.category),
                severity: severity(finding.severity),
                title: finding.title.clone(),
                summary: finding.summary.clone(),
                evidence_ids,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let compact_findings = report
        .findings
        .iter()
        .zip(&findings)
        .map(|(finding, stored)| {
            json!({
                "id": stored.id,
                "kind": stored.details["kind"],
                "category": finding.category,
                "severity": finding.severity,
                "title": finding.title,
                "summary": finding.summary,
                "evidence_quality": finding.evidence_quality,
                "evidence_ids": stored.evidence_ids,
                "affected_skill_ids": finding.affected_skill_ids,
                "affected_placement_ids": finding.affected_placement_ids,
                "affected_skill_count": finding.affected_skill_ids.len(),
                "affected_placement_count": finding.affected_placement_ids.len(),
                "impact": finding_impact(finding),
                "coverage": finding_coverage(finding, &scan)
            })
        })
        .collect::<Vec<_>>();
    let finding_rollups = finding_rollups(&compact_findings);
    let session_coverage = json!({
        "supported_agents": AgentKind::ALL.len(),
        "roots_present_agents": report.metrics.agents_with_session_roots,
        "sampled_agents": report.metrics.agents_with_sampled_session_data,
        "complete_agents": report.metrics.agents_with_reliable_session_denominator,
        "limited_agents": report.metrics.agents_with_limited_session_data,
        "missing_root_agents": report.metrics.agents_missing_session_roots,
        "inaccessible_agents": report.metrics.agents_with_inaccessible_session_roots
    });
    let value = json!({
        "report_id": report_id,
        "snapshot_id": scan_id,
        "skill_count": report.metrics.independent_skills,
        "placement_count": report.metrics.placements,
        "default_exposure": report.metrics.default_exposure,
        "observed_use_agent_count": report.metrics.agents_with_observed_usage,
        "coverage_reliable_agent_count": report.metrics.agents_with_reliable_session_denominator,
        "coverage_sampled_agent_count": report.metrics.agents_with_sampled_session_data,
        "coverage_root_agent_count": report.metrics.agents_with_session_roots,
        "coverage_limited_agent_count": report.metrics.agents_with_limited_session_data,
        "coverage_missing_agent_count": report.metrics.agents_missing_session_roots,
        "coverage_inaccessible_agent_count": report.metrics.agents_with_inaccessible_session_roots,
        "session_coverage": session_coverage,
        "primary_metrics": {
            "independent_skills": {"value": report.metrics.independent_skills, "unit": "skills"},
            "placements": {"value": report.metrics.placements, "unit": "placements"},
            "default_exposure": {"value": report.metrics.default_exposure, "unit": "placements"},
            "observed_use_agents": {
                "value": report.metrics.agents_with_observed_usage,
                "unit": "agents",
                "coverage": {
                    "reliable_agents": report.metrics.agents_with_reliable_session_denominator,
                    "sampled_agents": report.metrics.agents_with_sampled_session_data,
                    "roots_present_agents": report.metrics.agents_with_session_roots,
                    "limited_agents": report.metrics.agents_with_limited_session_data,
                    "missing_root_agents": report.metrics.agents_missing_session_roots,
                    "inaccessible_agents": report.metrics.agents_with_inaccessible_session_roots,
                    "supported_agents": AgentKind::ALL.len()
                }
            }
        },
        "findings": compact_findings,
        "finding_rollups": finding_rollups,
        "category_counts": report.category_counts,
        "files_changed": false
    });
    store.save_report(
        &ReportRecord {
            id: report_id,
            scan_id,
            created_at: Utc::now().timestamp(),
            summary: value.clone(),
        },
        &findings,
    )?;
    Ok(select_report_view(&value, request))
}

fn add_finding_resolution(object: &mut serde_json::Map<String, Value>) {
    if object.get("title").and_then(Value::as_str) != Some("Skill links escape an approved root") {
        return;
    }
    let observed_link_targets = object
        .get("placements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|placement| placement.get("link_target").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    object.insert(
        "resolution".into(),
        json!({
            "decision": "confirm_trusted_source_roots",
            "decision_semantics": "confirm_exact_local_read_permission",
            "permission_scope": "exact_local_read_only",
            "content_endorsed": false,
            "evidence_quality_changed": false,
            "governance_authorized": false,
            "plan_apply_authorized": false,
            "automatic_change_supported": false,
            "observed_link_targets": observed_link_targets,
            "after_confirmation": {
                "repeatable_option": "--source-root",
                "value": "absolute canonical source directory",
                "argv_template": [
                    "skillroster",
                    "--source-root",
                    "<confirmed-canonical-source-directory>",
                    "scan",
                    "--json"
                ]
            }
        }),
    );
}

fn add_same_name_resolution(object: &mut serde_json::Map<String, Value>, scan: &ScanResult) {
    if object.get("kind").and_then(Value::as_str)
        != Some(crate::query::SAME_NAME_DIVERGENT_FINDING_KIND)
    {
        return;
    }
    let mut affected_skill_ids = object
        .get("affected_skill_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    affected_skill_ids.sort();
    affected_skill_ids.dedup();
    let variant_count = affected_skill_ids.len();
    let variants = affected_skill_ids
        .iter()
        .take(10)
        .filter_map(|skill_id| {
            let skill = scan.skills.iter().find(|skill| skill.id == *skill_id)?;
            let placements = scan
                .placements
                .iter()
                .filter(|placement| placement.skill_id == *skill_id)
                .collect::<Vec<_>>();
            let content_digests = std::iter::once(skill.content_digest.clone())
                .chain(
                    placements
                        .iter()
                        .map(|placement| placement.content_digest.clone()),
                )
                .collect::<BTreeSet<_>>();
            let paths = placements
                .iter()
                .map(|placement| placement.entrypoint.display().to_string())
                .collect::<BTreeSet<_>>();
            let path_count = paths.len();
            let paths = paths.into_iter().take(10).collect::<Vec<_>>();
            let paths_truncated = paths.len() < path_count;
            let agents = placements
                .iter()
                .filter_map(|placement| placement.agent.map(AgentKind::id))
                .collect::<BTreeSet<_>>();
            let providers = placements
                .iter()
                .filter_map(|placement| placement.provider.clone())
                .collect::<BTreeSet<_>>();
            let roots = placements
                .iter()
                .map(|placement| placement.root.display().to_string())
                .collect::<BTreeSet<_>>();
            let governable = placements.iter().any(|placement| placement.is_mutable());
            let authority = placement_authority_summary(&placements);
            Some(json!({
                "skill_id": skill_id,
                "content_digests": content_digests,
                "paths": paths,
                "path_count": path_count,
                "paths_truncated": paths_truncated,
                "agents": agents,
                "providers": providers,
                "roots": roots,
                "source": skill.metadata.source.clone(),
                "governable": governable,
                "owned_by_agent": authority.owned_by_agent,
                "mutation_scopes": authority.mutation_scopes
            }))
        })
        .collect::<Vec<_>>();
    object.insert(
        "resolution".into(),
        json!({
            "decision": "choose_same_name_variant",
            "automatic_change_supported": false,
            "variant_count": variant_count,
            "variants_truncated": variants.len() < variant_count,
            "variants": variants,
            "next_step": "compare_variant_content_and_choose_canonical"
        }),
    );
}

struct PlacementAuthoritySummary {
    owned_by_agent: Option<bool>,
    mutation_scopes: Vec<scan::MutationScope>,
}

fn placement_authority_summary(placements: &[&scan::SkillPlacement]) -> PlacementAuthoritySummary {
    let owned_by_agent = (!placements.is_empty()
        && placements
            .iter()
            .all(|placement| placement.owned_by_agent.is_some()))
    .then(|| {
        placements
            .iter()
            .any(|placement| placement.owned_by_agent == Some(true))
    });
    let mutation_scopes = placements
        .iter()
        .filter_map(|placement| placement.mutation_scope)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    PlacementAuthoritySummary {
        owned_by_agent,
        mutation_scopes,
    }
}

fn read_only_placement_reason(
    placements: &[&scan::SkillPlacement],
) -> (&'static str, &'static str) {
    let scopes = placements
        .iter()
        .filter_map(|placement| placement.mutation_scope)
        .collect::<BTreeSet<_>>();
    if placements
        .iter()
        .any(|placement| placement.mutation_scope.is_none())
    {
        return (
            "mutation_scope_unknown",
            "Rescan with the current SkillRoster before preparing a mutating Plan.",
        );
    }
    if scopes == BTreeSet::from([scan::MutationScope::ProviderReadOnly]) {
        return (
            "provider_managed_read_only",
            "Keep provider-managed placements read-only.",
        );
    }
    if scopes == BTreeSet::from([scan::MutationScope::DurableReadOnly]) {
        return (
            "durable_source_read_only",
            "The source-read permission permits inspection only; choose a mutable placement for governance.",
        );
    }
    if scopes == BTreeSet::from([scan::MutationScope::UntrustedExternal]) {
        return (
            "untrusted_external_source",
            "Confirm the exact source root for reading, then rescan; confirmation does not authorize mutation.",
        );
    }
    (
        "non_mutable_placements",
        "Resolve each typed mutation scope before preparing a mutating Plan.",
    )
}

const SEMANTIC_COMPARISON_NAME_LIMIT: usize = 160;
const SEMANTIC_COMPARISON_DESCRIPTION_LIMIT: usize = 512;
const SEMANTIC_COMPARISON_METADATA_LIMIT: usize = 256;
const SEMANTIC_COMPARISON_LIST_LIMIT: usize = 10;
const SEMANTIC_COMPARISON_LIST_ITEM_LIMIT: usize = 160;

fn bounded_semantic_text(value: &str, limit: usize) -> (String, bool) {
    let bounded = value.chars().take(limit).collect::<String>();
    let truncated = value.chars().count() > limit;
    (bounded, truncated)
}

fn bounded_semantic_optional_text(value: Option<&str>, limit: usize) -> (Option<String>, bool) {
    value
        .map(|value| {
            let (bounded, truncated) = bounded_semantic_text(value, limit);
            (Some(bounded), truncated)
        })
        .unwrap_or((None, false))
}

fn bounded_semantic_list<'a>(
    values: impl IntoIterator<Item = &'a str>,
    item_limit: usize,
    character_limit: usize,
) -> (Vec<String>, usize, bool) {
    let values = values.into_iter().collect::<Vec<_>>();
    let count = values.len();
    let mut truncated = count > item_limit;
    let bounded = values
        .into_iter()
        .take(item_limit)
        .map(|value| {
            let (bounded, value_truncated) = bounded_semantic_text(value, character_limit);
            truncated |= value_truncated;
            bounded
        })
        .collect::<Vec<_>>();
    (bounded, count, truncated)
}

fn add_semantic_overlap_comparison(
    object: &mut serde_json::Map<String, Value>,
    scan: &ScanResult,
    state_dir: &Path,
) -> Result<()> {
    if object.get("title").and_then(Value::as_str)
        != Some(crate::query::SEMANTIC_OVERLAP_FINDING_TITLE)
    {
        return Ok(());
    }
    let affected_skill_ids = object
        .get("affected_skill_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if affected_skill_ids.len() != 2 {
        return Ok(());
    }
    let Some(left) = scan
        .skills
        .iter()
        .find(|skill| skill.id == affected_skill_ids[0])
    else {
        return Ok(());
    };
    let Some(right) = scan
        .skills
        .iter()
        .find(|skill| skill.id == affected_skill_ids[1])
    else {
        return Ok(());
    };
    let Some(basis) = crate::query::semantic_overlap_basis(left, right) else {
        return Ok(());
    };
    let skills = [left, right]
        .into_iter()
        .map(|skill| -> Result<Value> {
            let placements = scan
                .placements
                .iter()
                .filter(|placement| placement.skill_id == skill.id)
                .collect::<Vec<_>>();
            let agents = placements
                .iter()
                .filter_map(|placement| placement.agent.map(AgentKind::id))
                .collect::<BTreeSet<_>>();
            let providers = placements
                .iter()
                .filter_map(|placement| placement.provider.clone())
                .collect::<BTreeSet<_>>();
            let governable = placements.iter().any(|placement| placement.is_mutable());
            let authority = placement_authority_summary(&placements);
            let (name, name_truncated) =
                bounded_semantic_text(&skill.name, SEMANTIC_COMPARISON_NAME_LIMIT);
            let (description, description_truncated) = bounded_semantic_optional_text(
                skill.metadata.description.as_deref(),
                SEMANTIC_COMPARISON_DESCRIPTION_LIMIT,
            );
            let (triggers, trigger_count, triggers_truncated) = bounded_semantic_list(
                skill.metadata.triggers.iter().map(String::as_str),
                SEMANTIC_COMPARISON_LIST_LIMIT,
                SEMANTIC_COMPARISON_LIST_ITEM_LIMIT,
            );
            let (source, source_truncated) = bounded_semantic_optional_text(
                skill.metadata.source.as_deref(),
                SEMANTIC_COMPARISON_METADATA_LIMIT,
            );
            let (version, version_truncated) = bounded_semantic_optional_text(
                skill.metadata.version.as_deref(),
                SEMANTIC_COMPARISON_METADATA_LIMIT,
            );
            let (revision, revision_truncated) = bounded_semantic_optional_text(
                skill.metadata.revision.as_deref(),
                SEMANTIC_COMPARISON_METADATA_LIMIT,
            );
            let (agents, agent_count, agents_truncated) = bounded_semantic_list(
                agents.iter().copied(),
                SEMANTIC_COMPARISON_LIST_LIMIT,
                SEMANTIC_COMPARISON_LIST_ITEM_LIMIT,
            );
            let (providers, provider_count, providers_truncated) = bounded_semantic_list(
                providers.iter().map(String::as_str),
                SEMANTIC_COMPARISON_LIST_LIMIT,
                SEMANTIC_COMPARISON_LIST_ITEM_LIMIT,
            );
            let mut current_paths = current_readable_skill_paths(scan, state_dir, &skill.id)?;
            let readable_path_count = current_paths.paths.len();
            let visible_paths = current_paths
                .paths
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            current_paths.paths.truncate(5);
            let mut readable_placements = placements
                .iter()
                .filter(|placement| {
                    let path = placement.entrypoint.display().to_string();
                    visible_paths.contains(&path)
                })
                .map(|placement| {
                    let (provider, provider_truncated) = bounded_semantic_optional_text(
                        placement.provider.as_deref(),
                        SEMANTIC_COMPARISON_LIST_ITEM_LIMIT,
                    );
                    json!({
                        "placement_id": placement.id,
                        "path": placement.entrypoint,
                        "agent": placement.agent.map(AgentKind::id),
                        "provider": provider,
                        "provider_truncated": provider_truncated,
                        "owned_by_agent": placement.owned_by_agent,
                        "mutation_scope": placement.mutation_scope,
                        "governable": placement.is_mutable(),
                        "default_exposed": placement.default_exposed,
                        "link_status": placement.link_status,
                        "fingerprint_completeness": placement.fingerprint_completeness,
                        "fingerprint_detail": placement.fingerprint_detail
                    })
                })
                .collect::<Vec<_>>();
            readable_placements
                .sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
            let readable_placement_count = readable_placements.len();
            readable_placements.truncate(SEMANTIC_COMPARISON_LIST_LIMIT);
            Ok(json!({
                "skill_id": skill.id,
                "name": name,
                "name_truncated": name_truncated,
                "description": description,
                "description_truncated": description_truncated,
                "triggers": triggers,
                "trigger_count": trigger_count,
                "triggers_truncated": triggers_truncated,
                "summary": skill.summary,
                "source": source,
                "source_truncated": source_truncated,
                "version": version,
                "version_truncated": version_truncated,
                "revision": revision,
                "revision_truncated": revision_truncated,
                "content_digest": skill.content_digest,
                "digest_algorithm": skill.digest_algorithm,
                "agents": agents,
                "agent_count": agent_count,
                "agents_truncated": agents_truncated,
                "providers": providers,
                "provider_count": provider_count,
                "providers_truncated": providers_truncated,
                "governable": governable,
                "owned_by_agent": authority.owned_by_agent,
                "mutation_scopes": authority.mutation_scopes,
                "placement_count": placements.len(),
                "readable_path_count": readable_path_count,
                "readable_paths_truncated": readable_path_count > current_paths.paths.len(),
                "current_content_available": !current_paths.drifted,
                "readable_paths": current_paths.paths,
                "readable_placement_count": readable_placement_count,
                "readable_placements_truncated": readable_placement_count > readable_placements.len(),
                "readable_placements": readable_placements
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    object.insert(
        "comparison".into(),
        json!({
            "decision": "compare_skill_meaning",
            "automatic_change_supported": false,
            "semantic_conclusion_owner": "agent_or_user",
            "basis": basis,
            "skill_count": skills.len(),
            "skills": skills
        }),
    );
    Ok(())
}

fn compact_finding_detail(mut details: Value) -> Value {
    let Some(object) = details.as_object_mut() else {
        return details;
    };
    let items = object
        .remove("evidence")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|evidence| {
            let mut facts = evidence
                .get("details")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if let Some(facts) = facts.as_object_mut() {
                facts.remove("root_id");
                facts.retain(|_, value| !value.is_null());
            }
            json!({
                "evidence_id": evidence["id"],
                "kind": evidence["kind"],
                "quality": evidence["quality"],
                "subject_type": evidence["subject_type"],
                "subject_id": evidence["subject_id"],
                "path": evidence["path"],
                "facts": facts
            })
        })
        .collect::<Vec<_>>();
    object.remove("affected_skill_ids");
    object.remove("affected_placement_ids");
    object.remove("evidence_ids");
    object.remove("placements");
    if let Some(page) = object.get_mut("page").and_then(Value::as_object_mut) {
        if let Some(returned) = page.get_mut("returned").and_then(Value::as_object_mut) {
            returned.insert("items".into(), json!(items.len()));
        }
    }
    object.insert("items".into(), json!(items));
    details
}

fn report_actions(result: &Value, request: ReportRequest<'_>) -> Vec<SuggestedAction> {
    match request {
        ReportRequest::Summary => {
            let total = result["finding_count"].as_u64().unwrap_or_default();
            let findings = result["findings"].as_array();
            let returned = findings.map_or(0, |findings| findings.len() as u64);
            let mut actions = Vec::new();
            if total > returned {
                actions.push(action(
                    "list_findings",
                    &[
                        "report",
                        "--findings",
                        "--limit",
                        "20",
                        "--offset",
                        "0",
                        "--json",
                    ],
                    false,
                    false,
                    "more_findings_available",
                ));
            }
            actions.extend(
                findings
                    .into_iter()
                    .flatten()
                    .filter_map(|finding| finding["id"].as_str())
                    .take(3)
                    .map(|id| {
                        action(
                            "view_finding",
                            &["report", "--finding", id, "--json"],
                            false,
                            false,
                            "top_finding_selected",
                        )
                    }),
            );
            actions
        }
        ReportRequest::Findings {
            category,
            severity,
            limit,
            ..
        } => {
            let Some(next_offset) = result["page"]["next_offset"].as_u64() else {
                return Vec::new();
            };
            let mut argv = vec!["report".to_owned(), "--findings".to_owned()];
            if let Some(category) = category {
                argv.extend(["--category".to_owned(), category.id().to_owned()]);
            }
            if let Some(severity) = severity {
                argv.extend(["--severity".to_owned(), severity.id().to_owned()]);
            }
            argv.extend([
                "--limit".to_owned(),
                limit.to_string(),
                "--offset".to_owned(),
                next_offset.to_string(),
                "--json".to_owned(),
            ]);
            let argv = argv.iter().map(String::as_str).collect::<Vec<_>>();
            vec![action(
                "list_more_findings",
                &argv,
                false,
                false,
                "more_filtered_findings_available",
            )]
        }
        ReportRequest::Finding {
            id,
            full,
            limit,
            offset,
        } => {
            let requires_trust_decision =
                result["resolution"]["decision"].as_str() == Some("confirm_trusted_source_roots");
            let requires_variant_decision =
                result["resolution"]["decision"].as_str() == Some("choose_same_name_variant");
            let planning_supported = result["planning"]["supported"].as_bool() == Some(true);
            let mut actions = Vec::new();
            if let Some(next_offset) = result["page"]["next_offset"].as_u64() {
                let mut argv = vec!["report".to_owned(), "--finding".to_owned(), id.to_owned()];
                if full {
                    argv.push("--full".to_owned());
                }
                argv.extend([
                    "--limit".to_owned(),
                    limit.to_string(),
                    "--offset".to_owned(),
                    next_offset.to_string(),
                    "--json".to_owned(),
                ]);
                let argv = argv.iter().map(String::as_str).collect::<Vec<_>>();
                actions.push(action(
                    "list_more_finding_detail",
                    &argv,
                    false,
                    false,
                    "more_finding_detail_available",
                ));
            }
            if !requires_trust_decision && !requires_variant_decision && planning_supported {
                actions.push(action(
                    "plan",
                    &["plan", "--stdin", "--json"],
                    false,
                    false,
                    "finding_action_available",
                ));
            }
            if requires_trust_decision {
                for path in result["resolution"]["observed_link_targets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .take(10)
                {
                    actions.push(action(
                        "confirm_source_root_read_permission",
                        &[
                            "source-root",
                            "confirm",
                            "--finding",
                            id,
                            "--path",
                            path,
                            "--json",
                        ],
                        true,
                        true,
                        "exact_local_source_read_permission_required",
                    ));
                }
            }
            if !full {
                let limit = limit.to_string();
                let offset = offset.to_string();
                actions.push(action(
                    "show_full_finding",
                    &[
                        "report",
                        "--finding",
                        id,
                        "--full",
                        "--limit",
                        &limit,
                        "--offset",
                        &offset,
                        "--json",
                    ],
                    false,
                    false,
                    "finding_full_detail_available",
                ));
            }
            actions
        }
        ReportRequest::Exhaustive => Vec::new(),
    }
}

fn placement_owns_physical_source(placement: &scan::SkillPlacement) -> bool {
    if placement.link_target.is_some() {
        return false;
    }
    let Some(physical_directory) = placement.physical_directory.as_ref() else {
        return false;
    };
    let Some(root_parent) = placement.root.parent() else {
        return false;
    };
    let Some(root_name) = placement.root.file_name() else {
        return false;
    };
    let Ok(relative_directory) = placement.directory.strip_prefix(&placement.root) else {
        return false;
    };
    let Ok(physical_root_parent) = std::fs::canonicalize(root_parent) else {
        return false;
    };
    physical_root_parent
        .join(root_name)
        .join(relative_directory)
        == *physical_directory
}

fn finding_library_planning(
    finding: &FindingRecord,
    scan_id: &ScanId,
    latest_scan_id: &ScanId,
    scan: &ScanResult,
) -> Option<Value> {
    let (_, placement_ids) = exact_duplicate_finding_scope(finding, scan).ok()?;
    if scan_id != latest_scan_id {
        return Some(json!({
            "supported": false,
            "reason": "stale_finding",
            "snapshot_id": scan_id,
            "latest_snapshot_id": latest_scan_id
        }));
    }
    let affected = placement_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let protected = scan
        .placements
        .iter()
        .filter(|placement| affected.contains(placement.id.as_str()) && !placement.is_mutable())
        .collect::<Vec<_>>();
    let protected_count = protected.len();
    if !protected.is_empty() {
        let (reason, next_step) = read_only_placement_reason(&protected);
        let authority = placement_authority_summary(&protected);
        let blocked_placements = protected
            .iter()
            .take(5)
            .map(|placement| {
                json!({
                    "placement_id": placement.id,
                    "owned_by_agent": placement.owned_by_agent,
                    "mutation_scope": placement.mutation_scope,
                    "provider": placement.provider
                })
            })
            .collect::<Vec<_>>();
        return Some(json!({
            "supported": false,
            "reason": reason,
            "snapshot_id": scan_id,
            "protected_placement_count": protected_count,
            "blocked_placements": blocked_placements,
            "blocked_placements_truncated": protected_count > 5,
            "owned_by_agent": authority.owned_by_agent,
            "mutation_scopes": authority.mutation_scopes,
            "next_step": next_step
        }));
    }
    let mut physical_groups =
        std::collections::BTreeMap::<PathBuf, Vec<&scan::SkillPlacement>>::new();
    for placement in scan
        .placements
        .iter()
        .filter(|placement| affected.contains(placement.id.as_str()) && placement.is_mutable())
    {
        physical_groups
            .entry(placement.physical_directory_or_logical().to_path_buf())
            .or_default()
            .push(placement);
    }
    let mut candidates = physical_groups
        .into_values()
        .filter_map(|placements| {
            let placement = placements
                .into_iter()
                .find(|placement| placement_owns_physical_source(placement))?;
            let (rank, reason) = if placement.agent.is_none() && !placement.default_exposed {
                (0_u8, "non_exposed_source")
            } else if !placement.default_exposed {
                (1_u8, "non_exposed_owned_placement")
            } else {
                (2_u8, "agent_owned_placement")
            };
            Some((
                rank,
                placement.entrypoint.display().to_string(),
                json!({
                    "placement_id": placement.id,
                    "path": placement.entrypoint,
                    "agent": placement.agent.map(AgentKind::id),
                    "default_exposed": placement.default_exposed,
                    "governable": placement.is_mutable(),
                    "owned_by_agent": placement.owned_by_agent,
                    "mutation_scope": placement.mutation_scope,
                    "reason": reason
                }),
            ))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    let candidate_count = candidates.len();
    let candidates = candidates
        .into_iter()
        .take(5)
        .map(|(_, _, value)| value)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    Some(json!({
        "supported": true,
        "decision": "consolidate_exact_duplicates",
        "snapshot_id": scan_id,
        "request_field": "finding_library_changes",
        "allowed_requested_states": ["managed", "hosted"],
        "canonical_candidate_count": candidate_count,
        "canonical_candidates_truncated": candidate_count > 5,
        "canonical_candidates": candidates,
        "plan_request_template": {
            "schema_version": 1,
            "finding_library_changes": [{
                "finding_id": finding.id,
                "canonical_placement_id": "<choose a canonical_candidates placement_id>",
                "requested_state": "<managed|hosted>"
            }]
        }
    }))
}

#[derive(Default)]
struct BlockedSkillFacts {
    name: String,
    agents: BTreeSet<String>,
    reasons: BTreeSet<&'static str>,
    observed_source_targets: BTreeSet<String>,
    dependent_source_paths: BTreeSet<String>,
    dependent_placement_ids: BTreeSet<String>,
}

struct BlockedSkillPlanning {
    count: usize,
    items: Vec<Value>,
    truncated: bool,
    displayed_skill_ids: Vec<String>,
    displayed_dependent_source_paths: Vec<String>,
}

fn blocked_skill_planning(
    exclusions: &[crate::roster_plan::RosterChangeExclusion],
    full: bool,
) -> BlockedSkillPlanning {
    let mut by_skill = BTreeMap::<String, BlockedSkillFacts>::new();
    for exclusion in exclusions {
        let facts = by_skill.entry(exclusion.skill_id.clone()).or_default();
        facts.name = exclusion.name.clone();
        facts.agents.insert(exclusion.agent.clone());
        facts.reasons.insert(exclusion.reason);
        if let Some(target) = &exclusion.observed_source_target {
            facts
                .observed_source_targets
                .insert(target.display().to_string());
        }
        if let Some(crate::roster_plan::RosterSafetyBlocker::DependentSource {
            placement_ids,
            paths,
            ..
        }) = &exclusion.safety_blocker
        {
            facts
                .dependent_placement_ids
                .extend(placement_ids.iter().cloned());
            facts
                .dependent_source_paths
                .extend(paths.iter().map(|path| path.display().to_string()));
        }
    }

    let count = by_skill.len();
    let limit = if full { usize::MAX } else { 5 };
    let selected = by_skill.into_iter().take(limit).collect::<Vec<_>>();
    let displayed_skill_ids = selected
        .iter()
        .map(|(skill_id, _)| skill_id.clone())
        .collect::<Vec<_>>();
    let displayed_dependent_source_paths = selected
        .iter()
        .flat_map(|(_, facts)| facts.dependent_source_paths.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let items = selected
        .into_iter()
        .map(|(skill_id, facts)| {
            json!({
                "skill_id": skill_id,
                "name": facts.name,
                "agents": facts.agents,
                "reasons": facts.reasons,
                "observed_source_targets": facts.observed_source_targets,
                "dependent_source_paths": facts.dependent_source_paths,
                "dependent_placement_ids": facts.dependent_placement_ids
            })
        })
        .collect();
    BlockedSkillPlanning {
        count,
        items,
        truncated: count > limit,
        displayed_skill_ids,
        displayed_dependent_source_paths,
    }
}

fn finding_roster_planning(
    store: &StateStore,
    finding: &FindingRecord,
    scan_id: &ScanId,
    latest_scan_id: &ScanId,
    scan: &ScanResult,
    full: bool,
) -> Result<Option<Value>> {
    if !crate::roster_recommendation::is_large_roster_finding(finding) {
        return Ok(None);
    }
    if scan_id != latest_scan_id {
        return Ok(Some(json!({
            "supported": false,
            "reason": "stale_finding",
            "snapshot_id": scan_id,
            "latest_snapshot_id": latest_scan_id
        })));
    }
    let declared_core = declared_core_pairs(store, scan)?;
    let recommendation = match crate::roster_recommendation::recommend(
        finding,
        scan,
        &declared_core,
        &crate::roster_recommendation::RecommendationRequest {
            core_budget: crate::roster_recommendation::MAX_CORE_BUDGET,
            protected_skill_ids: BTreeSet::new(),
        },
    ) {
        Ok(recommendation) => recommendation,
        Err(error) => {
            return Ok(Some(json!({
                "supported": false,
                "reason": "automatic_roster_selection_unavailable",
                "detail": error.to_string(),
                "snapshot_id": scan_id
            })));
        }
    };
    let selection_evidence = roster_selection_evidence(&recommendation);
    let supported =
        crate::roster_plan::exclude_unpreservable_demotions(scan, recommendation.changes.clone())?;
    let agents = recommendation
        .agents
        .iter()
        .map(|agent| {
            let unchanged_blocked_count = supported
                .exclusions
                .iter()
                .filter(|exclusion| exclusion.agent == agent.agent.id())
                .count();
            let preview = agent
                .core_selections
                .iter()
                .take(5)
                .map(|selection| {
                    json!({
                        "skill_id": selection.skill_id,
                        "name": selection.name,
                        "reason": selection.reason,
                        "evidence_scope": selection.evidence_scope,
                        "evidence_agents": selection.evidence_agents
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "agent": agent.agent.id(),
                "before_default_exposure": agent.before_default_exposure,
                "unique_skill_count": agent.unique_skill_count,
                "proposed_core_count": agent.core_count,
                "proposed_on_demand_count": agent.on_demand_count.saturating_sub(unchanged_blocked_count),
                "unchanged_blocked_count": unchanged_blocked_count,
                "positive_signal_count": agent.positive_signal_count,
                "direct_signal_count": agent.direct_signal_count,
                "cross_agent_signal_count": agent.cross_agent_signal_count,
                "fallback_core_count": agent.fallback_core_count,
                "core_preview": preview,
                "core_preview_truncated": agent.core_selections.len() > 5
            })
        })
        .collect::<Vec<_>>();
    let blocked_change_count = supported.exclusions.len();
    let blocked_changes = supported
        .exclusions
        .iter()
        .take(5)
        .map(crate::roster_plan::blocked_change_json)
        .collect::<Vec<_>>();
    if blocked_change_count > 0 {
        let blocked_skills = blocked_skill_planning(&supported.exclusions, full);
        let source_dependency = supported
            .exclusions
            .iter()
            .any(|exclusion| exclusion.reason == "non_agent_source_link_depends_on_removal");
        let blocked_skill_ids = supported
            .exclusions
            .iter()
            .map(|exclusion| exclusion.skill_id.as_str())
            .collect::<BTreeSet<_>>();
        let observed_link_targets = scan
            .placements
            .iter()
            .filter(|placement| blocked_skill_ids.contains(placement.skill_id.as_str()))
            .filter_map(|placement| placement.link_target.as_ref())
            .map(|path| path.display().to_string())
            .collect::<BTreeSet<_>>();
        if source_dependency {
            let protected_skill_ids = blocked_skills.displayed_skill_ids.clone();
            let (protection_available, protection_unavailable_reason, protection_detail) =
                if blocked_skills.truncated {
                    (false, Some("blocked_skill_set_incomplete"), None)
                } else {
                    match crate::roster_recommendation::recommend(
                        finding,
                        scan,
                        &declared_core,
                        &crate::roster_recommendation::RecommendationRequest {
                            core_budget: crate::roster_recommendation::MAX_CORE_BUDGET,
                            protected_skill_ids: protected_skill_ids.iter().cloned().collect(),
                        },
                    ) {
                        Ok(_) => (true, None, None),
                        Err(error) => (
                            false,
                            Some("protected_core_selection_unavailable"),
                            Some(error.to_string()),
                        ),
                    }
                };
            let mut protect_choice = json!({
                "choice": "protect_blocked_skills_as_core",
                "requires_confirmation": true,
                "available": protection_available,
                "unavailable_reason": protection_unavailable_reason,
                "unavailable_detail": protection_detail,
                "protected_skill_ids": protected_skill_ids,
                "protected_skill_ids_complete": !blocked_skills.truncated,
                "plan_request_template_available": protection_available,
                "next": if blocked_skills.truncated {
                    "open the full Finding before constructing a complete protected-Skill Plan request"
                } else if !protection_available {
                    "the production Recommendation constraints reject this Core protection set; use the source-link preservation choice"
                } else {
                    "after user confirmation, retry Plan with these protected Skill identities; another independent blocker may still fail closed"
                }
            });
            if protection_available {
                protect_choice["plan_request_template"] = json!({
                    "schema_version": 1,
                    "finding_roster_changes": [{
                        "finding_id": finding.id,
                        "core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
                        "protected_skill_ids": protected_skill_ids
                    }]
                });
            }
            let source_choice = json!({
                "choice": "preserve_or_retarget_dependent_sources",
                "requires_confirmation": true,
                "dependent_source_paths": blocked_skills.displayed_dependent_source_paths,
                "dependent_source_paths_complete": !blocked_skills.truncated,
                "next": if blocked_skills.truncated {
                    "open the full Finding before changing any dependent source link"
                } else {
                    "after user-approved manual preservation or retargeting, rescan and reopen the new large-Roster Finding; another independent blocker may still fail closed"
                }
            });
            return Ok(Some(json!({
                "supported": false,
                "reason": "source_dependency_blocks_roster_change",
                "decision": "resolve_source_dependency",
                "automatic_change_supported": false,
                "snapshot_id": scan_id,
                "request_field": "finding_roster_changes",
                "default_core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
                "absence_of_usage_evidence": "not_negative_evidence",
                "explicit_only_or_archive_decision_implied": false,
                "agent_count": agents.len(),
                "agents": agents,
                "blocked_change_count": blocked_change_count,
                "blocked_changes": blocked_changes,
                "blocked_changes_truncated": blocked_change_count > 5,
                "blocked_skill_count": blocked_skills.count,
                "blocked_skills": blocked_skills.items,
                "blocked_skills_truncated": blocked_skills.truncated,
                "dependent_link_targets": observed_link_targets,
                "resolution_choices": [protect_choice, source_choice],
                "after_resolution": {
                    "next": if protection_available {
                        "choose either explicit Core protection or user-approved source-link preservation, then retry from fresh evidence; another independent blocker may still fail closed"
                    } else {
                        "use user-approved source-link preservation, then retry from fresh evidence; another independent blocker may still fail closed"
                    }
                }
            })));
        }
        let mutation_scopes = supported
            .exclusions
            .iter()
            .flat_map(|exclusion| match exclusion.safety_blocker.as_ref() {
                Some(crate::roster_plan::RosterSafetyBlocker::ProviderManaged { .. }) => {
                    vec!["provider_read_only".to_owned()]
                }
                Some(crate::roster_plan::RosterSafetyBlocker::MutationScopeReadOnly {
                    mutation_scopes,
                    ..
                }) => mutation_scopes.clone(),
                _ => Vec::new(),
            })
            .collect::<BTreeSet<_>>();
        let requires_more_than_read_confirmation = mutation_scopes.is_empty()
            || mutation_scopes
                .iter()
                .any(|scope| scope != "untrusted_external");
        if requires_more_than_read_confirmation {
            return Ok(Some(json!({
                "supported": false,
                "reason": "mutation_scope_blocks_roster_change",
                "decision": "choose_mutable_placements_or_keep_unchanged",
                "automatic_change_supported": false,
                "snapshot_id": scan_id,
                "request_field": "finding_roster_changes",
                "mutation_scopes": mutation_scopes,
                "blocked_change_count": blocked_change_count,
                "blocked_changes": blocked_changes,
                "blocked_changes_truncated": blocked_change_count > 5,
                "next": "Provider and durable read permissions authorize inspection only; keep these placements unchanged or choose current mutable placements. Rescan when scope facts are missing."
            })));
        }
        return Ok(Some(json!({
            "supported": false,
            "reason": "trusted_canonical_sources_required",
            "decision": "confirm_trusted_source_roots",
            "decision_semantics": "confirm_exact_local_read_permission",
            "permission_scope": "exact_local_read_only",
            "content_endorsed": false,
            "evidence_quality_changed": false,
            "governance_authorized": false,
            "plan_apply_authorized": false,
            "automatic_change_supported": false,
            "snapshot_id": scan_id,
            "request_field": "finding_roster_changes",
            "default_core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
            "absence_of_usage_evidence": "not_negative_evidence",
            "explicit_only_or_archive_decision_implied": false,
            "agent_count": agents.len(),
            "agents": agents,
            "blocked_change_count": blocked_change_count,
            "blocked_changes": blocked_changes,
            "blocked_changes_truncated": blocked_change_count > 5,
            "blocked_skill_count": blocked_skills.count,
            "blocked_skills": blocked_skills.items,
            "blocked_skills_truncated": blocked_skills.truncated,
            "observed_link_targets": observed_link_targets,
            "after_confirmation": {
                "repeatable_option": "--source-root",
                "value": "absolute canonical source directory",
                "next": "rescan and reopen the new large-Roster Finding"
            }
        })));
    }
    Ok(Some(json!({
        "supported": true,
        "decision": "right_size_default_rosters",
        "snapshot_id": scan_id,
        "request_field": "finding_roster_changes",
        "default_core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
        "allowed_core_budget": {
            "minimum": 1,
            "maximum": crate::roster_recommendation::MAX_CORE_BUDGET,
            "unit": "skills_per_affected_agent"
        },
        "selection_policy": [
            "protected_by_request",
            "declared_core",
            "skillroster_bootstrap",
            "target_agent_usage_evidence",
            "cross_agent_same_skill_usage_evidence",
            "stable_fallback"
        ],
        "absence_of_usage_evidence": "not_negative_evidence",
        "automatic_target_states": ["core", "on_demand"],
        "explicit_only_or_archive_decision_implied": false,
        "selection_evidence": selection_evidence.summary,
        "uncertainty": selection_evidence.uncertainty,
        "blocked_change_count": blocked_change_count,
        "blocked_changes": blocked_changes,
        "blocked_changes_truncated": blocked_change_count > 5,
        "agent_count": agents.len(),
        "agents": agents,
        "plan_request_template": {
            "schema_version": 1,
            "finding_roster_changes": [{
                "finding_id": finding.id,
                "core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
                "protected_skill_ids": []
            }]
        }
    })))
}

fn select_report_view(report: &Value, request: ReportRequest<'_>) -> Value {
    match request {
        ReportRequest::Summary => compact_report(report),
        ReportRequest::Findings {
            category,
            severity,
            limit,
            offset,
        } => paged_finding_report(report, category, severity, limit, offset),
        ReportRequest::Exhaustive | ReportRequest::Finding { .. } => report.clone(),
    }
}

fn compact_finding_summary(finding: &Value) -> Value {
    json!({
        "id": finding["id"],
        "kind": finding["kind"],
        "category": finding["category"],
        "severity": finding["severity"],
        "title": finding["title"],
        "summary": finding["summary"],
        "evidence_quality": finding["evidence_quality"],
        "primary_evidence_id": finding["evidence_ids"].as_array().and_then(|ids| ids.first()),
        "affected_skill_count": finding["affected_skill_count"],
        "affected_placement_count": finding["affected_placement_count"],
        "impact": finding["impact"],
        "coverage": finding["coverage"]
    })
}

struct FindingRollup {
    category: String,
    severity: String,
    title: String,
    finding_count: usize,
    skill_ids: BTreeSet<String>,
    placement_ids: BTreeSet<String>,
}

fn finding_rollups(findings: &[Value]) -> Vec<Value> {
    let mut rollups = Vec::<FindingRollup>::new();
    for finding in findings {
        let category = finding["category"].as_str().unwrap_or("unknown");
        let severity = finding["severity"].as_str().unwrap_or("unknown");
        let title = finding["title"].as_str().unwrap_or("Unknown Finding");
        let index = rollups
            .iter()
            .position(|rollup| {
                rollup.category == category && rollup.severity == severity && rollup.title == title
            })
            .unwrap_or_else(|| {
                rollups.push(FindingRollup {
                    category: category.to_owned(),
                    severity: severity.to_owned(),
                    title: title.to_owned(),
                    finding_count: 0,
                    skill_ids: BTreeSet::new(),
                    placement_ids: BTreeSet::new(),
                });
                rollups.len() - 1
            });
        let rollup = &mut rollups[index];
        rollup.finding_count += 1;
        for id in finding["affected_skill_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            rollup.skill_ids.insert(id.to_owned());
        }
        for id in finding["affected_placement_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            rollup.placement_ids.insert(id.to_owned());
        }
    }
    rollups
        .into_iter()
        .map(|rollup| {
            json!({
                "category": rollup.category,
                "severity": rollup.severity,
                "title": rollup.title,
                "finding_count": rollup.finding_count,
                "affected_skill_count": rollup.skill_ids.len(),
                "affected_placement_count": rollup.placement_ids.len()
            })
        })
        .collect()
}

fn report_finding_rollups(report: &Value) -> Value {
    report.get("finding_rollups").cloned().unwrap_or_else(|| {
        json!(finding_rollups(
            report["findings"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
        ))
    })
}

fn paged_finding_report(
    report: &Value,
    category: Option<ReportCategory>,
    severity: Option<ReportSeverity>,
    limit: usize,
    offset: usize,
) -> Value {
    let findings = report["findings"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let matching = findings
        .iter()
        .filter(|finding| {
            category.is_none_or(|value| finding["category"].as_str() == Some(value.id()))
                && severity.is_none_or(|value| finding["severity"].as_str() == Some(value.id()))
        })
        .collect::<Vec<_>>();
    let matching = interleave_finding_families(matching);
    let total = matching.len();
    let items = matching
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(compact_finding_summary)
        .collect::<Vec<_>>();
    let end = offset.saturating_add(items.len());
    let next_offset = (end < total).then_some(end);
    json!({
        "view": "findings",
        "report_id": report["report_id"],
        "snapshot_id": report["snapshot_id"],
        "skill_count": report["skill_count"],
        "placement_count": report["placement_count"],
        "default_exposure": report["default_exposure"],
        "observed_use_agent_count": report["observed_use_agent_count"],
        "coverage_reliable_agent_count": report["coverage_reliable_agent_count"],
        "coverage_sampled_agent_count": report["coverage_sampled_agent_count"],
        "coverage_root_agent_count": report["coverage_root_agent_count"],
        "coverage_limited_agent_count": report["coverage_limited_agent_count"],
        "coverage_missing_agent_count": report["coverage_missing_agent_count"],
        "coverage_inaccessible_agent_count": report["coverage_inaccessible_agent_count"],
        "session_coverage": report["session_coverage"],
        "primary_metrics": report["primary_metrics"],
        "finding_count": findings.len(),
        "matched_finding_count": total,
        "finding_rollups": report_finding_rollups(report),
        "category_counts": report["category_counts"],
        "filters": {
            "category": category.map(ReportCategory::id),
            "severity": severity.map(ReportSeverity::id)
        },
        "page": {
            "offset": offset,
            "limit": limit,
            "returned": items.len(),
            "total": total,
            "next_offset": next_offset,
            "has_more": next_offset.is_some()
        },
        "items": items,
        "files_changed": false
    })
}

fn interleave_finding_families(findings: Vec<&Value>) -> Vec<&Value> {
    let mut families = Vec::<((&str, &str, &str), Vec<&Value>)>::new();
    let mut family_indices = BTreeMap::<(&str, &str, &str), usize>::new();
    for finding in findings {
        let key = (
            finding["category"].as_str().unwrap_or("unknown"),
            finding["severity"].as_str().unwrap_or("unknown"),
            finding["title"].as_str().unwrap_or("Unknown Finding"),
        );
        if let Some(index) = family_indices.get(&key).copied() {
            families[index].1.push(finding);
        } else {
            family_indices.insert(key, families.len());
            families.push((key, vec![finding]));
        }
    }

    let mut interleaved = Vec::new();
    let mut member_index = 0;
    loop {
        let before = interleaved.len();
        for (_, members) in &families {
            if let Some(finding) = members.get(member_index) {
                interleaved.push(*finding);
            }
        }
        if interleaved.len() == before {
            break;
        }
        member_index += 1;
    }
    interleaved
}

fn compact_report(report: &Value) -> Value {
    let findings = report["findings"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let compact_findings = findings
        .iter()
        .take(3)
        .map(compact_finding_summary)
        .collect::<Vec<_>>();
    json!({
        "report_id": report["report_id"],
        "snapshot_id": report["snapshot_id"],
        "skill_count": report["skill_count"],
        "placement_count": report["placement_count"],
        "default_exposure": report["default_exposure"],
        "observed_use_agent_count": report["observed_use_agent_count"],
        "coverage_reliable_agent_count": report["coverage_reliable_agent_count"],
        "coverage_sampled_agent_count": report["coverage_sampled_agent_count"],
        "coverage_root_agent_count": report["coverage_root_agent_count"],
        "coverage_limited_agent_count": report["coverage_limited_agent_count"],
        "coverage_missing_agent_count": report["coverage_missing_agent_count"],
        "coverage_inaccessible_agent_count": report["coverage_inaccessible_agent_count"],
        "session_coverage": report["session_coverage"],
        "primary_metrics": report["primary_metrics"],
        "finding_count": findings.len(),
        "findings": compact_findings,
        "finding_rollups": report_finding_rollups(report),
        "category_counts": report["category_counts"],
        "files_changed": false
    })
}

fn finding_json(
    id: &FindingId,
    finding: &crate::query::Finding,
    evidence_ids: &[EvidenceId],
    scan: &ScanResult,
) -> Value {
    let category = finding_category(finding.category);
    let kind =
        crate::roster_recommendation::finding_kind(&category, &finding.title).or_else(|| {
            (finding.category == crate::query::FindingCategory::Layout
                && finding.title == crate::query::SAME_NAME_DIVERGENT_FINDING_TITLE)
                .then_some(crate::query::SAME_NAME_DIVERGENT_FINDING_KIND)
        });
    let mut value = json!({
        "id": id,
        "kind": kind,
        "category": finding.category,
        "severity": finding.severity,
        "title": finding.title,
        "summary": finding.summary,
        "evidence_quality": finding.evidence_quality,
        "evidence_ids": evidence_ids,
        "affected_skill_ids": finding.affected_skill_ids,
        "affected_placement_ids": finding.affected_placement_ids,
        "impact": finding_impact(finding),
        "coverage": finding_coverage(finding, scan),
        "files_changed": false
    });
    if finding.category == crate::query::FindingCategory::Usage
        && finding.title == "Five-stage usage evidence"
    {
        value["usage_overview"] = json!(crate::query::usage_overview(scan));
    }
    value
}

fn finding_impact(finding: &crate::query::Finding) -> Value {
    json!({
        "affected_skill_count": finding.affected_skill_ids.len(),
        "affected_placement_count": finding.affected_placement_ids.len(),
        "scope": if finding.affected_placement_ids.is_empty() {
            "library_or_evidence"
        } else {
            "agent_placements"
        }
    })
}

fn finding_coverage(finding: &crate::query::Finding, scan: &ScanResult) -> Value {
    finding_coverage_facts(
        finding.coverage_basis,
        json!(finding.evidence_quality),
        scan,
    )
}

fn stored_finding_coverage_basis(
    finding: &FindingRecord,
    details: &serde_json::Map<String, Value>,
) -> Result<crate::query::FindingCoverageBasis> {
    let stored_basis = match details.get("coverage") {
        None => None,
        Some(Value::Object(coverage)) => coverage.get("basis"),
        Some(_) => {
            return Err(StoredFindingCoverageInvalid {
                finding_id: finding.id.clone(),
                reason: "malformed_coverage",
            }
            .into());
        }
    };
    match stored_basis {
        Some(Value::String(basis)) if basis == "skill_root_scan" => {
            return Ok(crate::query::FindingCoverageBasis::SkillRootScan);
        }
        Some(Value::String(basis)) if basis == "session_usage" => {
            return Ok(crate::query::FindingCoverageBasis::SessionUsage);
        }
        Some(_) => {
            return Err(StoredFindingCoverageInvalid {
                finding_id: finding.id.clone(),
                reason: "unsupported_coverage_basis",
            }
            .into());
        }
        None => {}
    }

    // Records written before coverage dimensions were introduced have no typed basis.
    // Reconstruct only those legacy records from the stable Finding identity.
    Ok(match finding.category {
        FindingCategory::Usage => crate::query::FindingCoverageBasis::SessionUsage,
        FindingCategory::Lifecycle
            if matches!(
                finding.title.as_str(),
                crate::query::STALE_ARCHIVE_FINDING_TITLE
                    | crate::query::UNKNOWN_ARCHIVE_FINDING_TITLE
            ) =>
        {
            crate::query::FindingCoverageBasis::SessionUsage
        }
        _ => crate::query::FindingCoverageBasis::SkillRootScan,
    })
}

fn finding_coverage_facts(
    basis: crate::query::FindingCoverageBasis,
    evidence_quality: Value,
    scan: &ScanResult,
) -> Value {
    match basis {
        crate::query::FindingCoverageBasis::SkillRootScan => {
            structural_finding_coverage(basis, evidence_quality, scan)
        }
        crate::query::FindingCoverageBasis::SessionUsage => {
            session_finding_coverage(basis, evidence_quality, scan)
        }
    }
}

fn structural_finding_coverage(
    basis: crate::query::FindingCoverageBasis,
    evidence_quality: Value,
    scan: &ScanResult,
) -> Value {
    let roots = scan
        .roots
        .iter()
        .filter(|root| root.kind == RootKind::Skills)
        .collect::<Vec<_>>();
    let agents_for_status = |status: scan::RootStatus| {
        roots
            .iter()
            .filter(|root| root.status == status)
            .filter_map(|root| root.agent.map(AgentKind::id))
            .collect::<BTreeSet<_>>()
    };
    let included_agents = agents_for_status(scan::RootStatus::Included);
    let missing_agents = agents_for_status(scan::RootStatus::Missing);
    let inaccessible_agents = agents_for_status(scan::RootStatus::Inaccessible);
    let bounded_agents = roots
        .iter()
        .filter(|root| root.status == scan::RootStatus::Included && !root.discovery_complete)
        .filter_map(|root| root.agent.map(AgentKind::id))
        .collect::<BTreeSet<_>>();
    let limited_agents = inaccessible_agents
        .union(&bounded_agents)
        .copied()
        .collect::<BTreeSet<_>>();
    let reliable_agents = AgentKind::ALL
        .iter()
        .copied()
        .filter(|agent| !limited_agents.contains(agent.id()))
        .map(AgentKind::id)
        .collect::<Vec<_>>();
    let root_count = |status| roots.iter().filter(|root| root.status == status).count();
    let inaccessible_root_count = root_count(scan::RootStatus::Inaccessible);
    let bounded_root_count = roots
        .iter()
        .filter(|root| root.status == scan::RootStatus::Included && !root.discovery_complete)
        .count();
    json!({
        "basis": basis,
        "evidence_quality": evidence_quality,
        "scope": "configured_and_discovered_skill_roots",
        "reliable_agents": reliable_agents,
        "limited_agents": limited_agents,
        "included_agents": included_agents,
        "missing_agents": missing_agents,
        "inaccessible_agents": inaccessible_agents,
        "bounded_agents": bounded_agents,
        "supported_agent_count": AgentKind::ALL.len(),
        "included_root_count": root_count(scan::RootStatus::Included),
        "missing_root_count": root_count(scan::RootStatus::Missing),
        "inaccessible_root_count": inaccessible_root_count,
        "bounded_root_count": bounded_root_count,
        "denominator_reliable": inaccessible_root_count == 0 && bounded_root_count == 0
    })
}

fn session_finding_coverage(
    basis: crate::query::FindingCoverageBasis,
    evidence_quality: Value,
    scan: &ScanResult,
) -> Value {
    let session_roots = scan
        .roots
        .iter()
        .filter(|root| root.kind == RootKind::Sessions)
        .collect::<Vec<_>>();
    let mut reliable_agents = Vec::new();
    let mut limited_agents = Vec::new();
    let mut missing_agents = Vec::new();
    let mut excluded_agents = Vec::new();
    let mut inaccessible_agents = Vec::new();
    for agent in AgentKind::ALL {
        let has_status = |status| {
            session_roots
                .iter()
                .any(|root| root.agent == Some(agent) && root.status == status)
        };
        if has_status(scan::RootStatus::Inaccessible) {
            inaccessible_agents.push(agent.id());
        } else if has_status(scan::RootStatus::Excluded) {
            excluded_agents.push(agent.id());
        } else if has_status(scan::RootStatus::Missing) {
            missing_agents.push(agent.id());
        } else {
            match scan
                .coverage
                .iter()
                .find(|coverage| coverage.agent == agent)
            {
                Some(coverage) if coverage.denominator_reliable => {
                    reliable_agents.push(agent.id());
                }
                Some(_) => limited_agents.push(agent.id()),
                None => missing_agents.push(agent.id()),
            }
        }
    }
    let denominator_reliable = reliable_agents.len() == AgentKind::ALL.len();
    let root_count = |status| {
        session_roots
            .iter()
            .filter(|root| root.status == status)
            .count()
    };
    json!({
        "basis": basis,
        "evidence_quality": evidence_quality,
        "reliable_agents": reliable_agents,
        "limited_agents": limited_agents,
        "missing_agents": missing_agents,
        "excluded_agents": excluded_agents,
        "inaccessible_agents": inaccessible_agents,
        "supported_agent_count": AgentKind::ALL.len(),
        "included_root_count": root_count(scan::RootStatus::Included),
        "missing_root_count": root_count(scan::RootStatus::Missing),
        "excluded_root_count": root_count(scan::RootStatus::Excluded),
        "inaccessible_root_count": root_count(scan::RootStatus::Inaccessible),
        "denominator_reliable": denominator_reliable
    })
}

fn find_command(
    store: &StateStore,
    state_dir: &Path,
    task: &str,
    hints: &[String],
    limit: usize,
    load: bool,
    variant_skill_id: Option<&str>,
) -> Result<(Value, Vec<SuggestedAction>)> {
    if variant_skill_id.is_some() && !load {
        return Err(SkillLoadBlocked {
            reason: "variant_selector_requires_load",
            skill_id: variant_skill_id.unwrap_or_default().to_owned(),
            skill_name: "Top-1".into(),
            path: None,
            roster_state: "unknown".into(),
            mutation_scopes: Vec::new(),
            expected_digest: None,
            actual_digest: None,
        }
        .into());
    }
    let (scan_id, scan) = latest_scan(store)?;
    require_content_identity(&scan)?;
    let retrieval_hints = normalize_retrieval_hints(hints);
    let retrieval_query = crate::query::RetrievalQuery::from_parts(
        std::iter::once(task).chain(retrieval_hints.iter().map(String::as_str)),
    );
    let candidate_search_text = crate::query::candidate_search_text(retrieval_query.text());
    let mut candidate_ids = store
        .search_skill_ids(&candidate_search_text, scan.skills.len())?
        .into_iter()
        .map(|id| id.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    let mut routable_ids = scan
        .skills
        .iter()
        .map(|skill| skill.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut archived_ids = BTreeSet::new();
    for skill_id in routable_ids.clone() {
        let id = SkillId::parse(skill_id.clone())?;
        let states = store.roster_states_for_skill(&id)?;
        if !states.is_empty() && states.iter().all(|state| *state == RosterState::Archived) {
            routable_ids.remove(&skill_id);
            archived_ids.insert(skill_id);
        }
    }
    if crate::query::contains_cjk(retrieval_query.text()) {
        candidate_ids.extend(routable_ids.iter().cloned());
    }
    candidate_ids.retain(|skill_id| routable_ids.contains(skill_id));
    let pool_limit = if retrieval_hints.is_empty() {
        limit
    } else {
        usize::from(crate::cli::MAX_FIND_RESULTS)
    };
    let augmented_matches = crate::query::find_matching(
        &scan,
        &retrieval_query,
        pool_limit,
        Some(&candidate_ids),
        Some(&routable_ids),
    );
    let ranking_strategy = if retrieval_hints.is_empty() {
        "single_lexical_channel"
    } else {
        "task_hint_reciprocal_rank_fusion"
    };
    let mut matches = if retrieval_hints.is_empty() {
        augmented_matches
    } else {
        let task_query = crate::query::RetrievalQuery::from_parts([task]);
        let task_matches = crate::query::find_matching(
            &scan,
            &task_query,
            pool_limit,
            Some(&candidate_ids),
            Some(&routable_ids),
        );
        crate::query::fuse_retrieval_channels(task_matches, augmented_matches, &task_query, limit)
    };
    let mut warnings = Vec::new();
    let mut rescan_required = false;
    for found in &mut matches {
        let mut drifted_variants = 0_usize;
        if found.variants.is_empty() {
            let skill_id = SkillId::parse(found.skill_id.clone())?;
            found.roster_state = current_roster_state(&store.roster_states_for_skill(&skill_id)?);
            let current = current_readable_skill_paths(&scan, state_dir, &found.skill_id)?;
            found.paths = current.paths;
            drifted_variants += usize::from(current.drifted);
        } else {
            for variant in &mut found.variants {
                let skill_id = SkillId::parse(variant.skill_id.clone())?;
                variant.roster_state =
                    current_roster_state(&store.roster_states_for_skill(&skill_id)?);
                let current = current_readable_skill_paths(&scan, state_dir, &variant.skill_id)?;
                variant.paths = current.paths;
                drifted_variants += usize::from(current.drifted);
            }
            if let Some(representative) = found
                .variants
                .iter()
                .find(|variant| variant.skill_id == found.skill_id)
            {
                found.paths = representative.paths.clone();
                found.roster_state = representative.roster_state.clone();
            }
        }
        if drifted_variants > 0 {
            rescan_required = true;
            let variant_count = found.variants.len().max(1);
            warnings.push(if drifted_variants == variant_count {
                format!(
                    "{} has no current path that matches the latest Snapshot identity and fingerprint; run skillroster scan",
                    found.name
                )
            } else {
                format!(
                    "{} has {drifted_variants} same-name variant(s) with path drift; run skillroster scan",
                    found.name
                )
            });
        }
    }
    let (mut actions, variant_rescan_required) = bind_variant_findings(
        store,
        state_dir,
        &scan_id,
        &scan,
        &routable_ids,
        &mut matches,
    )?;
    rescan_required |= variant_rescan_required;
    if load && matches.is_empty() {
        let archived_match = crate::query::find_matching(
            &scan,
            &retrieval_query,
            1,
            Some(&archived_ids),
            Some(&archived_ids),
        )
        .into_iter()
        .next();
        return Err(SkillLoadBlocked {
            reason: if archived_match.is_some() {
                "archived_skill_not_routable"
            } else {
                "no_routable_match"
            },
            skill_id: archived_match
                .as_ref()
                .map(|matched| matched.skill_id.clone())
                .unwrap_or_default(),
            skill_name: archived_match
                .as_ref()
                .map(|matched| matched.name.clone())
                .unwrap_or_else(|| "Top-1".into()),
            path: None,
            roster_state: archived_match
                .map(|_| "archived".to_owned())
                .unwrap_or_else(|| "unassigned".into()),
            mutation_scopes: Vec::new(),
            expected_digest: None,
            actual_digest: None,
        }
        .into());
    }
    let loaded_skill = if load && !matches.is_empty() {
        let ranked = &matches[0];
        let selected = select_explicit_variant(ranked, variant_skill_id)?;
        let mut loaded = verified_top_skill_load(&scan_id, &scan, state_dir, &selected)?;
        if let Some(skill_id) = variant_skill_id {
            loaded["selection"]["ranking_evidence_scope"] = json!("ranked_capability_group");
            loaded["selection"]["variant_selection"] = json!({
                "mode": "explicit_skill_id",
                "requested_skill_id": skill_id,
                "ranked_capability_name": ranked.name,
                "ranked_group_representative_skill_id": ranked.skill_id,
                "ranked_variant_count": ranked.variant_count,
            });
        }
        Some(loaded)
    } else {
        None
    };
    for found in matches
        .iter()
        .take(3)
        .filter(|found| found.variant_count > 1)
    {
        let next = found.variant_finding.as_ref();
        warnings.push(match next.and_then(|reference| reference.finding_id.as_deref()) {
            Some(finding_id) => format!(
                "{} represents {} same-name Skill variants; inspect Finding {finding_id} before choosing content",
                found.name, found.variant_count
            ),
            None if matches!(
                next.map(|reference| reference.state),
                Some(crate::query::VariantFindingState::ReportRequired)
            ) => format!(
                "{} represents {} same-name Skill variants; materialize the current Report before choosing content",
                found.name, found.variant_count
            ),
            None if matches!(
                next.map(|reference| reference.state),
                Some(crate::query::VariantFindingState::RescanRequired)
            ) => format!(
                "{} represents {} same-name Skill variants; refresh the drifted Snapshot before choosing content",
                found.name, found.variant_count
            ),
            _ => format!(
                "{} represents {} same-name Skill variants, but no matching current divergent-content Finding is available",
                found.name, found.variant_count
            ),
        });
    }
    if !load {
        if let Some(found) = matches.first().filter(|found| {
            found.variant_count > 1
                && !matches!(
                    found
                        .variant_finding
                        .as_ref()
                        .map(|reference| reference.state),
                    Some(crate::query::VariantFindingState::RescanRequired)
                )
        }) {
            actions.extend(explicit_variant_load_actions(task, &retrieval_hints, found));
        }
    }
    let cjk_hint_required = retrieval_hints.is_empty() && crate::query::contains_cjk(task);
    if cjk_hint_required {
        warnings.push(
            "Find is lexical and the task contains CJK text; retry with one concise English capability paraphrase via --hint if relevant Skills use English metadata"
                .into(),
        );
    }
    if matches.is_empty() && !cjk_hint_required {
        warnings.push(
            "No lexical Skill match was found; retry once with concrete capability, tool, or operation terms via --hint"
                .into(),
        );
    }
    warnings.sort();
    warnings.dedup();
    let mut result = json!({
        "snapshot_id": scan_id,
        "task": task,
        "retrieval_hints": retrieval_hints,
        "ranking_strategy": ranking_strategy,
        "matches": matches,
        "rescan_required": rescan_required,
        "warnings": warnings,
        "files_changed": false
    });
    if let Some(loaded_skill) = loaded_skill {
        result["loaded_skill"] = loaded_skill;
    }
    Ok((result, actions))
}

fn select_explicit_variant(
    ranked: &crate::query::FindMatch,
    requested_skill_id: Option<&str>,
) -> Result<crate::query::FindMatch> {
    let Some(requested_skill_id) = requested_skill_id else {
        return Ok(ranked.clone());
    };
    let blocked = |reason| SkillLoadBlocked {
        reason,
        skill_id: requested_skill_id.to_owned(),
        skill_name: ranked.name.clone(),
        path: None,
        roster_state: "unknown".into(),
        mutation_scopes: Vec::new(),
        expected_digest: None,
        actual_digest: None,
    };
    if ranked.variant_count <= 1 {
        return Err(blocked("variant_selector_requires_ambiguous_top_match").into());
    }
    let Some(variant) = ranked
        .variants
        .iter()
        .find(|variant| variant.skill_id == requested_skill_id)
    else {
        return Err(blocked("variant_not_in_top_match").into());
    };
    let mut selected = ranked.clone();
    selected.skill_id = variant.skill_id.clone();
    selected.paths = variant.paths.clone();
    selected.agents = variant.agents.clone();
    selected.roster_state = variant.roster_state.clone();
    selected.source = variant.source.clone();
    selected.providers = variant.providers.clone();
    selected.governable = variant.governable;
    selected.owned_by_agent = variant.owned_by_agent;
    selected.mutation_scopes = variant.mutation_scopes.clone();
    selected.variant_skill_ids.clear();
    selected.variants.clear();
    selected.variant_count = 1;
    selected.variants_truncated = false;
    selected.variant_finding = None;
    Ok(selected)
}

fn explicit_variant_load_actions(
    task: &str,
    hints: &[String],
    ranked: &crate::query::FindMatch,
) -> Vec<SuggestedAction> {
    ranked
        .variants
        .iter()
        .map(|variant| {
            let mut argv = vec!["skillroster".to_owned(), "find".to_owned(), task.to_owned()];
            for hint in hints {
                argv.extend(["--hint".to_owned(), hint.clone()]);
            }
            argv.extend([
                "--load".to_owned(),
                "--limit".to_owned(),
                "1".to_owned(),
                "--variant-skill-id".to_owned(),
                variant.skill_id.clone(),
                "--json".to_owned(),
            ]);
            SuggestedAction {
                action: "load_exact_variant_for_comparison".into(),
                description: "load_exact_variant_for_comparison".into(),
                argv,
                mutates: false,
                requires_confirmation: false,
                reason_code: "same_name_variant_explicit_read_available".into(),
            }
        })
        .collect()
}

fn bind_variant_findings(
    store: &StateStore,
    state_dir: &Path,
    scan_id: &ScanId,
    scan: &ScanResult,
    routable_ids: &BTreeSet<String>,
    matches: &mut [crate::query::FindMatch],
) -> Result<(Vec<SuggestedAction>, bool)> {
    let report = store
        .latest_report()?
        .filter(|report| report.scan_id == *scan_id);
    let mut actions = Vec::new();
    let mut action_argvs = BTreeSet::new();
    let mut rescan_required = false;

    for (index, found) in matches
        .iter_mut()
        .enumerate()
        .filter(|(_, found)| found.variant_count > 1)
    {
        let capability_name = found.name.trim().to_lowercase();
        let variant_ids = scan
            .skills
            .iter()
            .filter(|skill| skill.name.trim().to_lowercase() == capability_name)
            .filter(|skill| routable_ids.contains(&skill.id))
            .map(|skill| skill.id.clone())
            .collect::<BTreeSet<_>>();
        let mut variant_drifted = false;
        for skill_id in &variant_ids {
            variant_drifted |= current_readable_skill_paths(scan, state_dir, skill_id)?.drifted;
        }
        rescan_required |= variant_drifted;
        let (reference, suggested) = if variant_drifted {
            let suggested = action(
                "refresh_drifted_snapshot",
                &["scan", "--json"],
                false,
                false,
                "routable_variant_drift_detected",
            );
            (
                crate::query::VariantFindingReference {
                    state: crate::query::VariantFindingState::RescanRequired,
                    reason_code: crate::query::VariantFindingReason::RoutableVariantDriftDetected,
                    snapshot_id: scan_id.to_string(),
                    report_id: None,
                    finding_id: None,
                    resolution: None,
                    argv: suggested.argv.clone(),
                },
                suggested,
            )
        } else if let Some(report) = report.as_ref() {
            if let Some(finding_id) = matching_variant_finding_id(store, report, &variant_ids)? {
                let suggested = action(
                    "inspect_variant_finding",
                    &["report", "--finding", &finding_id, "--json"],
                    false,
                    false,
                    "same_snapshot_variant_finding_available",
                );
                (
                    crate::query::VariantFindingReference {
                        state: crate::query::VariantFindingState::Available,
                        reason_code:
                            crate::query::VariantFindingReason::SameSnapshotVariantSetMatched,
                        snapshot_id: scan_id.to_string(),
                        report_id: Some(report.id.to_string()),
                        finding_id: Some(finding_id),
                        resolution: Some("choose_same_name_variant".into()),
                        argv: suggested.argv.clone(),
                    },
                    suggested,
                )
            } else {
                let suggested = action(
                    "inspect_layout_findings",
                    &["report", "--findings", "--category", "layout", "--json"],
                    false,
                    false,
                    "matching_divergent_content_finding_missing",
                );
                (
                    crate::query::VariantFindingReference {
                        state: crate::query::VariantFindingState::FindingUnavailable,
                        reason_code: crate::query::VariantFindingReason::MatchingDivergentContentFindingMissing,
                        snapshot_id: scan_id.to_string(),
                        report_id: Some(report.id.to_string()),
                        finding_id: None,
                        resolution: None,
                        argv: suggested.argv.clone(),
                    },
                    suggested,
                )
            }
        } else {
            let suggested = action(
                "materialize_report",
                &["report", "--summary", "--json"],
                false,
                false,
                "current_snapshot_report_missing",
            );
            (
                crate::query::VariantFindingReference {
                    state: crate::query::VariantFindingState::ReportRequired,
                    reason_code: crate::query::VariantFindingReason::CurrentSnapshotReportMissing,
                    snapshot_id: scan_id.to_string(),
                    report_id: None,
                    finding_id: None,
                    resolution: None,
                    argv: suggested.argv.clone(),
                },
                suggested,
            )
        };
        found.variant_finding = Some(reference);
        if index < 3 && action_argvs.insert(suggested.argv.clone()) {
            actions.push(suggested);
        }
    }
    Ok((actions, rescan_required))
}

fn matching_variant_finding_id(
    store: &StateStore,
    report: &ReportRecord,
    variant_ids: &BTreeSet<String>,
) -> Result<Option<String>> {
    let Some(candidate) = report.summary["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|finding| {
            if finding["kind"] != crate::query::SAME_NAME_DIVERGENT_FINDING_KIND {
                return None;
            }
            let affected = finding["affected_skill_ids"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<BTreeSet<_>>();
            (affected == *variant_ids)
                .then(|| finding["id"].as_str())
                .flatten()
        })
    else {
        return Ok(None);
    };
    let id = FindingId::parse(candidate.to_owned())?;
    let Some(stored) = store.get_finding(&id)? else {
        return Ok(None);
    };
    let stored_variant_ids = stored.details["affected_skill_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    Ok((stored.report_id == report.id
        && stored.category == FindingCategory::Layout
        && stored.details["kind"] == crate::query::SAME_NAME_DIVERGENT_FINDING_KIND
        && stored_variant_ids == *variant_ids)
        .then(|| id.to_string()))
}

fn normalize_retrieval_hints(hints: &[String]) -> Vec<String> {
    let mut normalized = hints
        .iter()
        .map(|hint| hint.trim())
        .filter(|hint| !hint.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn current_roster_state(states: &[RosterState]) -> String {
    let mut names = states
        .iter()
        .map(|state| match state {
            RosterState::Core => "core",
            RosterState::OnDemand => "on_demand",
            RosterState::ExplicitOnly => "explicit_only",
            RosterState::Archived => "archived",
        })
        .collect::<std::collections::BTreeSet<_>>();
    match names.len() {
        0 => "unassigned".into(),
        1 => names.pop_first().unwrap_or("unassigned").into(),
        _ => "mixed".into(),
    }
}

fn current_readable_skill_paths(
    scan: &ScanResult,
    state_dir: &Path,
    skill_id: &str,
) -> Result<CurrentSkillPaths> {
    let skill = scan
        .skills
        .iter()
        .find(|skill| skill.id == skill_id)
        .ok_or_else(|| anyhow!("Skill {skill_id} is not in the latest Snapshot"))?;
    let directory_name = safe_skill_directory_name(&skill.name).ok();
    let mut candidates = directory_name
        .as_ref()
        .map(|directory_name| {
            state_dir
                .join("library")
                .join(directory_name)
                .join("SKILL.md")
        })
        .into_iter()
        .collect::<Vec<_>>();
    candidates.extend(
        scan.placements
            .iter()
            .filter(|placement| placement.skill_id == skill_id)
            .map(|placement| placement.entrypoint.clone()),
    );
    if let Some(directory_name) = directory_name {
        candidates.extend(
            scan.roots
                .iter()
                .filter(|root| {
                    root.kind == RootKind::Skills && root.status == scan::RootStatus::Included
                })
                .map(|root| root.path.join(&directory_name).join("SKILL.md")),
        );
    }
    let mut approved_roots = scan
        .roots
        .iter()
        .filter(|root| root.kind == RootKind::Skills && root.status == scan::RootStatus::Included)
        .filter_map(|root| std::fs::canonicalize(&root.path).ok())
        .collect::<Vec<_>>();
    if let Ok(library) = std::fs::canonicalize(state_dir.join("library")) {
        approved_roots.push(library);
    }
    approved_roots.sort();
    approved_roots.dedup();

    let mut fingerprints_by_resolved_path =
        std::collections::BTreeMap::<PathBuf, std::collections::BTreeSet<&str>>::new();
    let mut known_fingerprints = std::collections::BTreeSet::new();
    for placement in scan
        .placements
        .iter()
        .filter(|placement| placement.skill_id == skill_id)
    {
        known_fingerprints.insert(placement.content_digest.as_str());
        if let Ok(resolved) = std::fs::canonicalize(&placement.entrypoint) {
            fingerprints_by_resolved_path
                .entry(resolved)
                .or_default()
                .insert(placement.content_digest.as_str());
        }
    }

    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in candidates {
        if !seen.insert(path.clone()) || !path.exists() {
            continue;
        }
        let Ok(resolved) = std::fs::canonicalize(&path) else {
            continue;
        };
        if !resolved.is_file() || !approved_roots.iter().any(|root| resolved.starts_with(root)) {
            continue;
        }
        let Ok((actual_id, actual_fingerprint)) = scan::inspect_skill_identity(&path) else {
            continue;
        };
        let expected_fingerprints = fingerprints_by_resolved_path
            .get(&resolved)
            .unwrap_or(&known_fingerprints);
        if actual_id != skill.id || !expected_fingerprints.contains(actual_fingerprint.as_str()) {
            continue;
        }
        paths.push(path.display().to_string());
    }
    paths.sort();
    paths.dedup();
    Ok(CurrentSkillPaths {
        drifted: paths.is_empty(),
        paths,
    })
}

struct CurrentSkillPaths {
    paths: Vec<String>,
    drifted: bool,
}

#[derive(Debug)]
struct SkillLoadBlocked {
    reason: &'static str,
    skill_id: String,
    skill_name: String,
    path: Option<PathBuf>,
    roster_state: String,
    mutation_scopes: Vec<String>,
    expected_digest: Option<String>,
    actual_digest: Option<String>,
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

fn verified_top_skill_load(
    scan_id: &ScanId,
    scan: &ScanResult,
    state_dir: &Path,
    matched: &crate::query::FindMatch,
) -> Result<Value> {
    let blocked = |reason, path, expected_digest, actual_digest| SkillLoadBlocked {
        reason,
        skill_id: matched.skill_id.clone(),
        skill_name: matched.name.clone(),
        path,
        roster_state: matched.roster_state.clone(),
        mutation_scopes: matched
            .mutation_scopes
            .iter()
            .map(|scope| scope.id().to_owned())
            .collect(),
        expected_digest,
        actual_digest,
    };

    if matched.variant_count != 1 {
        return Err(blocked("same_name_variants_ambiguous", None, None, None).into());
    }
    if matched.roster_state == "archived" {
        return Err(blocked("archived_skill_not_routable", None, None, None).into());
    }

    let mut placements = scan
        .placements
        .iter()
        .filter(|placement| placement.skill_id == matched.skill_id)
        .collect::<Vec<_>>();
    placements.sort_by(|left, right| left.entrypoint.cmp(&right.entrypoint));
    if placements.is_empty() {
        return Err(blocked("placement_missing_from_snapshot", None, None, None).into());
    }
    if placements.iter().all(|placement| {
        placement
            .mutation_scope
            .is_none_or(|scope| scope == scan::MutationScope::UntrustedExternal)
    }) {
        return Err(blocked("untrusted_external_source", None, None, None).into());
    }
    if placements.iter().all(|placement| {
        placement.fingerprint_completeness != scan::FingerprintCompleteness::Complete
    }) {
        return Err(blocked("package_fingerprint_incomplete", None, None, None).into());
    }
    if placements
        .iter()
        .filter(|placement| {
            placement.mutation_scope != Some(scan::MutationScope::UntrustedExternal)
                && placement.fingerprint_completeness == scan::FingerprintCompleteness::Complete
        })
        .all(|placement| placement.entrypoint_digest.is_none())
    {
        return Err(blocked("legacy_snapshot_requires_rescan", None, None, None).into());
    }

    let placement = placements
        .into_iter()
        .find(|placement| {
            placement.mutation_scope != Some(scan::MutationScope::UntrustedExternal)
                && placement.fingerprint_completeness == scan::FingerprintCompleteness::Complete
                && placement.entrypoint_digest.is_some()
        })
        .ok_or_else(|| blocked("eligible_placement_missing", None, None, None))?;
    let path = placement.entrypoint.clone();
    let expected_entrypoint_digest = placement.entrypoint_digest.clone().ok_or_else(|| {
        blocked(
            "legacy_snapshot_requires_rescan",
            Some(path.clone()),
            None,
            None,
        )
    })?;
    let before = std::fs::canonicalize(&path).map_err(|_| {
        blocked(
            "entrypoint_unreadable",
            Some(path.clone()),
            Some(expected_entrypoint_digest.clone()),
            None,
        )
    })?;
    let mut allowed_roots = scan
        .roots
        .iter()
        .filter(|root| root.kind == RootKind::Skills && root.status == scan::RootStatus::Included)
        .filter_map(|root| std::fs::canonicalize(&root.path).ok())
        .collect::<Vec<_>>();
    if let Ok(library) = std::fs::canonicalize(state_dir.join("library")) {
        allowed_roots.push(library);
    }
    if !before.is_file() || !allowed_roots.iter().any(|root| before.starts_with(root)) {
        return Err(blocked(
            "entrypoint_escapes_approved_roots",
            Some(path),
            Some(expected_entrypoint_digest),
            None,
        )
        .into());
    }
    let metadata = std::fs::metadata(&before).map_err(|_| {
        blocked(
            "entrypoint_unreadable",
            Some(path.clone()),
            Some(expected_entrypoint_digest.clone()),
            None,
        )
    })?;
    if metadata.len() > MAX_AGENT_LOADED_SKILL_BYTES {
        return Err(blocked(
            "entrypoint_exceeds_content_limit",
            Some(path),
            Some(expected_entrypoint_digest),
            None,
        )
        .into());
    }
    let bytes = std::fs::read(&before).map_err(|_| {
        blocked(
            "entrypoint_unreadable",
            Some(path.clone()),
            Some(expected_entrypoint_digest.clone()),
            None,
        )
    })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_AGENT_LOADED_SKILL_BYTES {
        return Err(blocked(
            "entrypoint_exceeds_content_limit",
            Some(path),
            Some(expected_entrypoint_digest),
            None,
        )
        .into());
    }
    let actual_entrypoint_digest = content_digest(&bytes);
    if actual_entrypoint_digest != expected_entrypoint_digest {
        return Err(blocked(
            "entrypoint_content_drift",
            Some(path),
            Some(expected_entrypoint_digest),
            Some(actual_entrypoint_digest),
        )
        .into());
    }
    let content = String::from_utf8(bytes).map_err(|_| {
        blocked(
            "entrypoint_not_utf8",
            Some(path.clone()),
            Some(expected_entrypoint_digest.clone()),
            Some(actual_entrypoint_digest.clone()),
        )
    })?;
    let (actual_id, actual_package_digest) =
        scan::inspect_skill_identity(&before).map_err(|_| {
            blocked(
                "package_identity_unreadable",
                Some(path.clone()),
                Some(placement.content_digest.clone()),
                None,
            )
        })?;
    let after = std::fs::canonicalize(&path).map_err(|_| {
        blocked(
            "entrypoint_unreadable",
            Some(path.clone()),
            Some(expected_entrypoint_digest.clone()),
            None,
        )
    })?;
    if before != after
        || actual_id != matched.skill_id
        || actual_package_digest != placement.content_digest
    {
        return Err(blocked(
            "package_identity_drift",
            Some(path),
            Some(placement.content_digest.clone()),
            Some(actual_package_digest),
        )
        .into());
    }

    let read_authority = match placement.mutation_scope {
        Some(scan::MutationScope::Mutable) => "agent_or_managed_root",
        Some(scan::MutationScope::ProviderReadOnly) => "provider_read_only",
        Some(scan::MutationScope::DurableReadOnly) => "confirmed_durable_read_only",
        Some(scan::MutationScope::UntrustedExternal) | None => unreachable!("filtered above"),
    };
    Ok(json!({
        "selection": {
            "rank": matched.rank,
            "skill_id": matched.skill_id,
            "name": matched.name,
            "snapshot_id": scan_id,
            "ranking_evidence": matched.match_reasons,
            "ranking_evidence_scope": "loaded_identity",
        },
        "content": {
            "path": placement.entrypoint,
            "media_type": "text/markdown; charset=utf-8",
            "byte_length": content.len(),
            "sha256": actual_entrypoint_digest,
            "complete": true,
            "text": content,
        },
        "governance": {
            "roster_state": matched.roster_state,
            "read_authority": read_authority,
            "content_endorsed": false,
            "owned_by_agent": placement.owned_by_agent,
            "mutation_scope": placement.mutation_scope,
            "governable": placement.is_mutable(),
            "provider": placement.provider,
        },
        "verification": {
            "identity_matches_snapshot": true,
            "entrypoint_digest_matches_snapshot": true,
            "package_fingerprint_matches_snapshot": true,
            "package_fingerprint_complete": true,
            "package_fingerprint": actual_package_digest,
            "content_limit_bytes": MAX_AGENT_LOADED_SKILL_BYTES,
        },
        "task_success": "not_evaluated",
    }))
}

fn plan_command(
    store: &StateStore,
    state_dir: &Path,
    action_argv_prefix: &[String],
) -> Result<Value> {
    if store.recovery_required()? {
        bail!("recovery is required before another Plan can be prepared");
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    prepare_plan(
        store,
        state_dir,
        serde_json::from_str(&input)?,
        PlanOrigin::Agent,
        None,
        action_argv_prefix,
    )
}

fn plan_detail_command(store: &StateStore, id: &str) -> Result<Value> {
    let id = PlanId::parse(id.to_owned())?;
    let record = store
        .get_plan(&id)?
        .ok_or_else(|| anyhow!("Plan {id} does not exist"))?;
    let prepared: PreparedPlan = serde_json::from_value(record.input["prepared"].clone())?;
    let mut result = record
        .input
        .get("summary")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let object = result
        .as_object_mut()
        .expect("stored Plan summary is an object");
    object.remove("detail");
    object.insert("detail_level".into(), json!("full"));
    object.insert("plan_id".into(), json!(prepared.id));
    object.insert("snapshot_id".into(), json!(prepared.scan_id));
    object.insert("report_id".into(), json!(record.report_id));
    object.insert("digest".into(), json!(prepared.digest));
    object.insert("state".into(), json!(record.status));
    object.insert("operations".into(), json!(prepared.operations));
    object.insert("roster_changes".into(), json!(prepared.roster_changes));
    object.insert("evidence_ids".into(), json!(prepared.evidence_ids));
    object.insert(
        "finding_ids".into(),
        record
            .input
            .get("finding_ids")
            .cloned()
            .unwrap_or_else(|| json!([])),
    );
    object.insert("source_updates".into(), json!(prepared.source_updates));
    object.insert("library_changes".into(), json!(prepared.library_changes));
    object.insert(
        "roster_before".into(),
        record.input["roster_before"].clone(),
    );
    object.insert(
        "library_before".into(),
        record.input["library_before"].clone(),
    );
    if let Some(selection_evidence) = record
        .input
        .get("selection_evidence_full")
        .filter(|value| value.is_object())
    {
        object.insert("selection_evidence".into(), selection_evidence.clone());
    }
    object.insert("risk".into(), json!(plan_risk(&prepared)));
    object.insert("reversible".into(), json!(true));
    object.insert("canonical_deletion_count".into(), json!(0));
    object.insert("confirmation_required".into(), json!(true));
    object.insert("files_changed".into(), json!(false));
    Ok(result)
}

fn plan_risk(prepared: &PreparedPlan) -> &'static str {
    if !prepared.source_updates.is_empty() {
        "source_update"
    } else if !prepared.roster_changes.is_empty() {
        "roster_change"
    } else if !prepared.library_changes.is_empty() {
        "library_governance"
    } else if prepared.operations.is_empty() {
        "roster_change"
    } else {
        "filesystem_change"
    }
}

fn operation_groups(operations: &[Operation]) -> Value {
    let mut groups = std::collections::BTreeMap::<&str, usize>::new();
    for operation in operations {
        let kind = match operation {
            Operation::CreateDirectory { .. } => "create_directory",
            Operation::CreateSymlink { .. } => "create_symlink",
            Operation::WriteFile { .. } => "write_file",
            Operation::ReplaceFile { .. } => "replace_file",
            Operation::RemoveSymlink { .. } => "remove_symlink",
            Operation::Copy { .. } => "copy",
            Operation::MoveRecoverable { .. } => "move_recoverable",
        };
        *groups.entry(kind).or_default() += 1;
    }
    json!(groups)
}

fn affected_summary(prepared: &PreparedPlan, scan: &ScanResult, impact: &Value) -> Value {
    let mut agents = prepared
        .roster_changes
        .iter()
        .map(|change| change.agent.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let skills = prepared
        .roster_changes
        .iter()
        .map(|change| change.skill_id.clone())
        .chain(
            prepared
                .library_changes
                .iter()
                .map(|change| change.skill_id.clone()),
        )
        .chain(
            prepared
                .source_updates
                .iter()
                .map(|change| change.skill_id.clone()),
        )
        .collect::<std::collections::BTreeSet<_>>();
    let roster_pairs = prepared
        .roster_changes
        .iter()
        .map(|change| (change.agent.as_str(), change.skill_id.as_str()))
        .collect::<BTreeSet<_>>();
    let mut placements = scan
        .placements
        .iter()
        .filter(|placement| {
            placement.agent.is_some_and(|agent| {
                roster_pairs.contains(&(agent.id(), placement.skill_id.as_str()))
            })
        })
        .map(|placement| placement.id.clone())
        .collect::<BTreeSet<_>>();
    placements.extend(
        prepared
            .library_changes
            .iter()
            .flat_map(|change| change.placement_ids.iter().cloned())
            .chain(
                prepared
                    .source_updates
                    .iter()
                    .map(|change| change.placement_id.clone()),
            ),
    );
    for placement in &scan.placements {
        if placements.contains(&placement.id) {
            if let Some(agent) = placement.agent {
                agents.insert(agent.id().to_owned());
            }
        }
    }
    agents.extend(
        impact
            .get("affected_agents")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
    let skill_count = skills.len();
    let skill_ids = skills.iter().take(10).cloned().collect::<Vec<_>>();
    json!({
        "agent_count": agents.len(),
        "agents": agents,
        "skill_count": skill_count,
        "skill_ids": skill_ids,
        "skill_ids_truncated": skill_count > 10,
        "placement_count": placements.len()
    })
}

fn bounded_ids(ids: Vec<String>) -> Value {
    json!({
        "count": ids.len(),
        "ids": ids.iter().take(10).collect::<Vec<_>>(),
        "ids_truncated": ids.len() > 10
    })
}

fn state_counts(values: &[Value], key: &str) -> Value {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for value in values {
        let state = value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("unassigned");
        *counts.entry(state.to_owned()).or_default() += 1;
    }
    json!(counts)
}

fn transition_change_count(before: &[Value], after: &[Value], after_state_key: &str) -> usize {
    before
        .iter()
        .zip(after)
        .filter(|(before, after)| before.get("state") != after.get(after_state_key))
        .count()
        + before.len().abs_diff(after.len())
}

fn bound_transition(value: &mut Value, after_state_key: &str) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let before = object
        .remove("before")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let after = object
        .remove("after")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    object.insert(
        "change_count".into(),
        json!(transition_change_count(&before, &after, after_state_key)),
    );
    object.insert("before_state_counts".into(), state_counts(&before, "state"));
    object.insert(
        "after_state_counts".into(),
        state_counts(&after, after_state_key),
    );
}

fn library_impact_totals(items: &[Value]) -> Option<Value> {
    if items.is_empty() {
        return None;
    }
    let sum = |pointer: &str| {
        items.iter().try_fold(0_u64, |total, item| {
            total.checked_add(item.pointer(pointer)?.as_u64()?)
        })
    };
    let states = |pointer: &str| {
        items.iter().try_fold(
            std::collections::BTreeMap::<String, usize>::new(),
            |mut counts, item| {
                let state = item.pointer(pointer)?.as_str()?;
                *counts.entry(state.to_owned()).or_default() += 1;
                Some(counts)
            },
        )
    };
    let before_sources = sum("/before/physical_source_count")?;
    let after_sources = sum("/after/physical_source_count")?;
    let before_placements = sum("/before/placement_count")?;
    let after_placements = sum("/after/placement_count")?;
    let before_exposed = sum("/before/default_exposed_placement_count")?;
    let after_exposed = sum("/after/default_exposed_placement_count")?;
    let relinked = sum("/after/relinked_placement_count")?;
    let delta = |after: u64, before: u64| {
        i64::try_from(after)
            .ok()?
            .checked_sub(i64::try_from(before).ok()?)
    };
    Some(json!({
        "before": {
            "physical_source_count": before_sources,
            "placement_count": before_placements,
            "default_exposed_placement_count": before_exposed,
            "governance_state_counts": states("/before/governance_state")?
        },
        "after": {
            "physical_source_count": after_sources,
            "placement_count": after_placements,
            "default_exposed_placement_count": after_exposed,
            "governance_state_counts": states("/after/governance_state")?
        },
        "delta": {
            "physical_source_count": delta(after_sources, before_sources)?,
            "placement_count": delta(after_placements, before_placements)?,
            "default_exposed_placement_count": delta(after_exposed, before_exposed)?
        },
        "relinked_placement_count": relinked
    }))
}

fn bounded_plan_impact(mut impact: Value) -> Value {
    let Some(object) = impact.as_object_mut() else {
        let items = impact.as_array().cloned().unwrap_or_default();
        let mut bounded = json!({
            "item_count": items.len(),
            "items": items.iter().take(10).collect::<Vec<_>>(),
            "items_truncated": items.len() > 10
        });
        if let Some(totals) = library_impact_totals(&items) {
            bounded["totals"] = totals;
        }
        return bounded;
    };
    if let Some(percent) = object
        .get("exposure_reduction_percent")
        .and_then(Value::as_f64)
    {
        object.insert(
            "exposure_reduction_percent".into(),
            json!((percent * 100.0).round() / 100.0),
        );
    }
    for (ids_key, count_key) in [
        ("affected_skill_ids", "affected_skill_count"),
        ("affected_placement_ids", "affected_placement_count"),
    ] {
        if let Some(ids) = object
            .remove(ids_key)
            .and_then(|value| value.as_array().cloned())
        {
            object.insert(count_key.into(), json!(ids.len()));
        }
    }
    if let Some(exclusions) = object
        .remove("exclusions")
        .and_then(|value| value.as_array().cloned())
    {
        object.insert("exclusion_count".into(), json!(exclusions.len()));
        object.insert(
            "exclusions".into(),
            json!(exclusions.iter().take(5).collect::<Vec<_>>()),
        );
        object.insert("exclusions_truncated".into(), json!(exclusions.len() > 5));
    }
    if let Some(blocked) = object
        .remove("blocked_preconditions")
        .and_then(|value| value.as_array().cloned())
    {
        object.insert("blocked_precondition_count".into(), json!(blocked.len()));
        object.insert(
            "blocked_preconditions".into(),
            json!(blocked.iter().take(5).collect::<Vec<_>>()),
        );
        object.insert(
            "blocked_preconditions_truncated".into(),
            json!(blocked.len() > 5),
        );
    }
    if let Some(roster) = object.get_mut("roster") {
        bound_transition(roster, "state");
    }
    if let Some(library) = object.get_mut("library") {
        bound_transition(library, "requested_state");
    }
    impact
}

fn plan_diff_summary(
    origin: PlanOrigin,
    source_update_diffs: &[Value],
    change_summary: &Value,
    operation_groups: &Value,
    impact: &Value,
) -> Value {
    if matches!(origin, PlanOrigin::SourceUpdate) {
        return json!({
            "item_count": source_update_diffs.len(),
            "items": source_update_diffs.iter().take(10).collect::<Vec<_>>(),
            "items_truncated": source_update_diffs.len() > 10
        });
    }

    let mut items = Vec::new();
    if change_summary["roster_change_count"]
        .as_u64()
        .is_some_and(|count| count > 0)
    {
        items.push(json!({
            "kind": "roster",
            "change_count": change_summary["roster_change_count"],
            "before_state_counts": impact.pointer("/roster/before_state_counts"),
            "after_state_counts": impact.pointer("/roster/after_state_counts"),
            "before_default_exposure": impact.get("before_default_exposure"),
            "after_default_exposure": impact.get("after_default_exposure")
        }));
    }
    if change_summary["library_change_count"]
        .as_u64()
        .is_some_and(|count| count > 0)
    {
        items.push(json!({
            "kind": "library",
            "change_count": change_summary["library_change_count"],
            "before_state_counts": impact.pointer("/library/before_state_counts"),
            "after_state_counts": impact.pointer("/library/after_state_counts")
        }));
    }
    if change_summary["operation_count"]
        .as_u64()
        .is_some_and(|count| count > 0)
    {
        items.push(json!({
            "kind": "filesystem",
            "operation_count": change_summary["operation_count"],
            "operation_groups": operation_groups
        }));
    }
    json!({
        "item_count": items.len(),
        "items": items,
        "items_truncated": false
    })
}

#[derive(Clone, Copy)]
enum PlanOrigin {
    Agent,
    RosterGovernance,
    BootstrapSetup,
    SourceUpdate,
    LibraryGovernance,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceUpdateChoice {
    RetainLocal,
    AdoptUpstream,
    PreserveBoth,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceUpdateRequest {
    skill_id: String,
    placement_id: String,
    source: String,
    current_revision: String,
    current_fingerprint: String,
    base_digest: Option<String>,
    upstream_revision: String,
    upstream_content: String,
    upstream_digest: String,
    choice: Option<SourceUpdateChoice>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum RequestedGovernanceState {
    Managed,
    Hosted,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LibraryChangeRequest {
    skill_id: String,
    canonical_placement_id: String,
    placement_ids: Vec<String>,
    requested_state: RequestedGovernanceState,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FindingLibraryChangeRequest {
    finding_id: String,
    canonical_placement_id: String,
    requested_state: RequestedGovernanceState,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FindingRosterChangeRequest {
    finding_id: String,
    core_budget: usize,
    #[serde(default)]
    protected_skill_ids: Vec<String>,
}

struct FindingPlanProvenance {
    report_id: ReportId,
    finding_ids: Vec<FindingId>,
    selection_evidence_summary: Option<Value>,
    selection_evidence_full: Option<Value>,
    uncertainty: Option<Value>,
}

struct SelectionEvidenceViews {
    summary: Value,
    full: Value,
    uncertainty: Option<Value>,
}

fn roster_selection_evidence(
    recommendation: &crate::roster_recommendation::RosterRecommendation,
) -> SelectionEvidenceViews {
    let mut core_selection_count = 0_usize;
    let mut forced_core_count = 0_usize;
    let mut positive_signal_core_count = 0_usize;
    let mut direct_signal_core_count = 0_usize;
    let mut cross_agent_signal_core_count = 0_usize;
    let mut stable_fallback_core_count = 0_usize;
    let mut fallback_dominated_agent_count = 0_usize;
    let mut cross_agent_dominated_agent_count = 0_usize;
    let mut reason_counts = std::collections::BTreeMap::<&str, usize>::new();
    let mut summary_agents = Vec::new();
    let mut full_agents = Vec::new();
    for agent in &recommendation.agents {
        let mut forced = 0_usize;
        let mut direct = 0_usize;
        let mut cross_agent = 0_usize;
        let mut fallback = 0_usize;
        for selection in &agent.core_selections {
            *reason_counts.entry(selection.reason).or_default() += 1;
            match selection.evidence_scope {
                "forced" => forced += 1,
                "target_agent" => direct += 1,
                "cross_agent" => cross_agent += 1,
                "fallback" => fallback += 1,
                _ => {}
            }
        }
        let core_count = agent.core_selections.len();
        let positive = direct + cross_agent;
        let fallback_dominated = fallback > core_count / 2;
        let cross_agent_dominated = cross_agent > core_count / 2;
        core_selection_count += core_count;
        forced_core_count += forced;
        positive_signal_core_count += positive;
        direct_signal_core_count += direct;
        cross_agent_signal_core_count += cross_agent;
        stable_fallback_core_count += fallback;
        fallback_dominated_agent_count += usize::from(fallback_dominated);
        cross_agent_dominated_agent_count += usize::from(cross_agent_dominated);
        let selections = agent
            .core_selections
            .iter()
            .map(|selection| {
                json!({
                    "skill_id": selection.skill_id,
                    "name": selection.name,
                    "reason": selection.reason,
                    "evidence_scope": selection.evidence_scope,
                    "evidence_agents": selection.evidence_agents
                })
            })
            .collect::<Vec<_>>();
        let common = json!({
            "agent": agent.agent.id(),
            "core_selection_count": core_count,
            "forced_core_count": forced,
            "positive_signal_core_count": positive,
            "direct_signal_core_count": direct,
            "cross_agent_signal_core_count": cross_agent,
            "stable_fallback_core_count": fallback,
            "fallback_dominated": fallback_dominated,
            "cross_agent_dominated": cross_agent_dominated,
            "core_preview": selections.iter().take(5).cloned().collect::<Vec<_>>(),
            "core_preview_truncated": selections.len() > 5
        });
        summary_agents.push(common.clone());
        let mut full = common;
        full.as_object_mut()
            .expect("selection evidence Agent is an object")
            .insert("core_selections".into(), json!(selections));
        full_agents.push(full);
    }
    let fallback_dominated = fallback_dominated_agent_count > 0;
    let cross_agent_dominated = cross_agent_dominated_agent_count > 0;
    let evidence = |detail_level: &str, agents: Vec<Value>| {
        json!({
            "detail_level": detail_level,
            "selection_policy": [
                "protected_by_request",
                "declared_core",
                "skillroster_bootstrap",
                "target_agent_usage_evidence",
                "cross_agent_same_skill_usage_evidence",
                "stable_fallback"
            ],
            "core_selection_count": core_selection_count,
            "forced_core_count": forced_core_count,
            "positive_signal_core_count": positive_signal_core_count,
            "direct_signal_core_count": direct_signal_core_count,
            "cross_agent_signal_core_count": cross_agent_signal_core_count,
            "stable_fallback_core_count": stable_fallback_core_count,
            "fallback_dominated": fallback_dominated,
            "fallback_dominated_agent_count": fallback_dominated_agent_count,
            "cross_agent_dominated": cross_agent_dominated,
            "cross_agent_dominated_agent_count": cross_agent_dominated_agent_count,
            "reason_counts": reason_counts,
            "agents": agents,
            "absence_of_usage_evidence": "not_negative_evidence"
        })
    };
    let uncertainty = if fallback_dominated && cross_agent_dominated {
        Some(json!({
            "code": "mixed_evidence_dominated_core_selection",
            "dominance_codes": [
                "fallback_dominated_core_selection",
                "cross_agent_dominated_core_selection"
            ],
            "review_required": true,
            "core_selection_count": core_selection_count,
            "direct_signal_core_count": direct_signal_core_count,
            "cross_agent_signal_core_count": cross_agent_signal_core_count,
            "stable_fallback_core_count": stable_fallback_core_count,
            "fallback_dominated_agent_count": fallback_dominated_agent_count,
            "cross_agent_dominated_agent_count": cross_agent_dominated_agent_count,
            "absence_of_usage_evidence": "not_negative_evidence"
        }))
    } else if fallback_dominated {
        Some(json!({
            "code": "fallback_dominated_core_selection",
            "review_required": true,
            "core_selection_count": core_selection_count,
            "direct_signal_core_count": direct_signal_core_count,
            "cross_agent_signal_core_count": cross_agent_signal_core_count,
            "stable_fallback_core_count": stable_fallback_core_count,
            "fallback_dominated_agent_count": fallback_dominated_agent_count,
            "absence_of_usage_evidence": "not_negative_evidence"
        }))
    } else if cross_agent_dominated {
        Some(json!({
            "code": "cross_agent_dominated_core_selection",
            "review_required": true,
            "core_selection_count": core_selection_count,
            "direct_signal_core_count": direct_signal_core_count,
            "cross_agent_signal_core_count": cross_agent_signal_core_count,
            "stable_fallback_core_count": stable_fallback_core_count,
            "cross_agent_dominated_agent_count": cross_agent_dominated_agent_count,
            "absence_of_usage_evidence": "not_negative_evidence"
        }))
    } else {
        None
    };
    SelectionEvidenceViews {
        summary: evidence("summary", summary_agents),
        full: evidence("full", full_agents),
        uncertainty,
    }
}

fn declared_core_pairs(
    store: &StateStore,
    scan: &ScanResult,
) -> Result<BTreeSet<(AgentKind, String)>> {
    let mut pairs = BTreeSet::new();
    let unique = scan
        .placements
        .iter()
        .filter_map(|placement| {
            placement
                .agent
                .map(|agent| (agent, placement.skill_id.as_str()))
        })
        .collect::<BTreeSet<_>>();
    for (agent, skill_id) in unique {
        let Some(agent_id) = store.agent_id(&model_agent(agent))? else {
            continue;
        };
        let skill_id = SkillId::parse(skill_id.to_owned())?;
        if store
            .roster_entry(&agent_id, &skill_id)?
            .is_some_and(|entry| entry.state == RosterState::Core)
        {
            pairs.insert((agent, skill_id.as_str().to_owned()));
        }
    }
    Ok(pairs)
}

fn expand_finding_roster_changes(
    store: &StateStore,
    mut input: Value,
    latest_scan_id: &ScanId,
    scan: &ScanResult,
    state_dir: &Path,
    action_argv_prefix: &[String],
) -> Result<(Value, Option<FindingPlanProvenance>)> {
    let object = input
        .as_object_mut()
        .ok_or_else(|| anyhow!("Plan input must be a JSON object"))?;
    let raw = object
        .remove("finding_roster_changes")
        .unwrap_or_else(|| json!([]));
    let requests: Vec<FindingRosterChangeRequest> = serde_json::from_value(raw)?;
    if requests.is_empty() {
        return Ok((input, None));
    }
    if requests.len() != 1 {
        bail!("one finding_roster_changes request is allowed per Plan");
    }
    if [
        "scan_id",
        "evidence_ids",
        "roster_changes",
        "source_updates",
        "library_changes",
        "finding_library_changes",
    ]
    .iter()
    .any(|key| object.contains_key(*key))
    {
        bail!(
            "finding_roster_changes cannot be mixed with CLI-derived IDs or other governance changes"
        );
    }
    let request = requests.into_iter().next().expect("length checked");
    let protected_skill_ids = request
        .protected_skill_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if protected_skill_ids.len() != request.protected_skill_ids.len() {
        bail!("finding_roster_changes contains duplicate protected Skill IDs");
    }
    let finding_id = FindingId::parse(request.finding_id)?;
    let finding = store
        .get_finding(&finding_id)?
        .ok_or_else(|| anyhow!("Finding {finding_id} does not exist"))?;
    let report = store
        .get_report(&finding.report_id)?
        .ok_or_else(|| anyhow!("Report {} does not exist", finding.report_id))?;
    if report.scan_id != *latest_scan_id {
        bail!("Finding {finding_id} does not belong to the latest Snapshot {latest_scan_id}");
    }
    let recommendation = crate::roster_recommendation::recommend(
        &finding,
        scan,
        &declared_core_pairs(store, scan)?,
        &crate::roster_recommendation::RecommendationRequest {
            core_budget: request.core_budget,
            protected_skill_ids,
        },
    )?;
    let selection_evidence = roster_selection_evidence(&recommendation);
    let supported =
        crate::roster_plan::exclude_unpreservable_demotions(scan, recommendation.changes.clone())?;
    if !supported.exclusions.is_empty() {
        if let Some(blocker) = finding_roster_safety_blocker(&supported.exclusions) {
            return Err(blocker.into());
        }
        if let Some(exclusion) = supported.exclusions.iter().find(|exclusion| {
            exclusion.reason == "multiple_package_fingerprints_require_explicit_preservation"
        }) {
            let placements = scan
                .placements
                .iter()
                .filter(|placement| placement.skill_id == exclusion.skill_id)
                .collect::<Vec<_>>();
            let fingerprint_count = placements
                .iter()
                .map(|placement| placement.content_digest.as_str())
                .collect::<BTreeSet<_>>()
                .len();
            return Err(crate::roster_plan::RosterPackageFingerprintVariants {
                skill_id: exclusion.skill_id.clone(),
                placement_ids: placements
                    .iter()
                    .map(|placement| placement.id.clone())
                    .collect(),
                fingerprint_count,
            }
            .into());
        }
        return Err(crate::roster_plan::source_confirmation_block(
            finding_id.as_str(),
            request.core_budget,
            &supported.exclusions,
            state_dir,
            action_argv_prefix,
        )?
        .into());
    }
    if finding.evidence_ids.is_empty() {
        bail!("Finding {finding_id} has no Evidence");
    }
    input["scan_id"] = json!(latest_scan_id);
    input["evidence_ids"] = json!(finding.evidence_ids);
    input["roster_changes"] = serde_json::to_value(supported.changes)?;
    Ok((
        input,
        Some(FindingPlanProvenance {
            report_id: finding.report_id,
            finding_ids: vec![finding_id],
            selection_evidence_summary: Some(selection_evidence.summary),
            selection_evidence_full: Some(selection_evidence.full),
            uncertainty: selection_evidence.uncertainty,
        }),
    ))
}

fn finding_roster_safety_blocker(
    exclusions: &[crate::roster_plan::RosterChangeExclusion],
) -> Option<crate::roster_plan::RosterSafetyBlocker> {
    exclusions
        .iter()
        .filter_map(|exclusion| exclusion.safety_blocker.clone())
        .find(|blocker| match blocker {
            crate::roster_plan::RosterSafetyBlocker::MutationScopeReadOnly {
                mutation_scopes,
                ..
            } => mutation_scopes.len() != 1 || mutation_scopes[0].as_str() != "untrusted_external",
            _ => true,
        })
}

fn exact_duplicate_finding_scope(
    finding: &FindingRecord,
    scan: &ScanResult,
) -> Result<(String, Vec<String>)> {
    if finding.category != FindingCategory::Overlap {
        bail!("Finding {} is not an exact-duplicate Finding", finding.id);
    }
    let object = finding
        .details
        .as_object()
        .ok_or_else(|| anyhow!("Finding {} has invalid stored details", finding.id))?;
    let ids = |key: &str| -> Result<Vec<String>> {
        object
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Finding {} has no {key}", finding.id))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow!("Finding {} has an invalid {key}", finding.id))
            })
            .collect()
    };
    let skill_ids = ids("affected_skill_ids")?;
    let placement_ids = ids("affected_placement_ids")?;
    if skill_ids.len() != 1 || placement_ids.len() < 2 {
        bail!("Finding {} is not an exact-duplicate Finding", finding.id);
    }
    let unique_ids = placement_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    if unique_ids.len() != placement_ids.len() {
        bail!(
            "Finding {} contains duplicate placement references",
            finding.id
        );
    }
    let skill_id = &skill_ids[0];
    let all_placements = scan
        .placements
        .iter()
        .filter(|placement| placement.skill_id == *skill_id)
        .collect::<Vec<_>>();
    if let Some(blocker) =
        incomplete_fingerprint_blocker(all_placements.iter().copied(), "finding_plan")
    {
        return Err(blocker.into());
    }
    let all_ids = all_placements
        .iter()
        .map(|placement| placement.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if unique_ids != all_ids {
        bail!(
            "Finding {} does not cover every placement of Skill {}",
            finding.id,
            skill_id
        );
    }
    let digests = all_placements
        .iter()
        .map(|placement| placement.content_digest.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if digests.len() != 1 {
        bail!("Finding {} does not contain exact duplicates", finding.id);
    }
    Ok((skill_id.clone(), placement_ids))
}

fn expand_finding_library_changes(
    store: &StateStore,
    mut input: Value,
    latest_scan_id: &ScanId,
    scan: &ScanResult,
) -> Result<(Value, Option<FindingPlanProvenance>)> {
    let object = input
        .as_object_mut()
        .ok_or_else(|| anyhow!("Plan input must be a JSON object"))?;
    let raw = object
        .remove("finding_library_changes")
        .unwrap_or_else(|| json!([]));
    let requests: Vec<FindingLibraryChangeRequest> = serde_json::from_value(raw)?;
    if requests.is_empty() {
        return Ok((input, None));
    }
    if [
        "scan_id",
        "evidence_ids",
        "roster_changes",
        "source_updates",
        "library_changes",
        "finding_roster_changes",
    ]
    .iter()
    .any(|key| object.contains_key(*key))
    {
        bail!(
            "finding_library_changes cannot be mixed with CLI-derived IDs or other governance changes"
        );
    }

    let mut evidence_ids = std::collections::BTreeSet::new();
    let mut library_changes = Vec::with_capacity(requests.len());
    let mut report_id = None;
    let mut finding_ids = Vec::with_capacity(requests.len());
    for request in requests {
        let finding_id = FindingId::parse(request.finding_id)?;
        let finding = store
            .get_finding(&finding_id)?
            .ok_or_else(|| anyhow!("Finding {finding_id} does not exist"))?;
        let report = store
            .get_report(&finding.report_id)?
            .ok_or_else(|| anyhow!("Report {} does not exist", finding.report_id))?;
        if report.scan_id != *latest_scan_id {
            bail!("Finding {finding_id} does not belong to the latest Snapshot {latest_scan_id}");
        }
        if report_id
            .as_ref()
            .is_some_and(|existing| existing != &finding.report_id)
        {
            bail!("finding_library_changes must come from one Report");
        }
        report_id.get_or_insert_with(|| finding.report_id.clone());
        let (skill_id, placement_ids) = exact_duplicate_finding_scope(&finding, scan)?;
        if !placement_ids.contains(&request.canonical_placement_id) {
            bail!("canonical placement is not part of Finding {finding_id}");
        }
        let evidence_id = finding
            .evidence_ids
            .first()
            .ok_or_else(|| anyhow!("Finding {finding_id} has no Evidence"))?;
        evidence_ids.insert(evidence_id.as_str().to_owned());
        library_changes.push(LibraryChangeRequest {
            skill_id,
            canonical_placement_id: request.canonical_placement_id,
            placement_ids,
            requested_state: request.requested_state,
        });
        finding_ids.push(finding_id);
    }
    input["scan_id"] = json!(latest_scan_id);
    input["evidence_ids"] = json!(evidence_ids);
    input["library_changes"] = serde_json::to_value(library_changes)?;
    Ok((
        input,
        Some(FindingPlanProvenance {
            report_id: report_id.expect("non-empty Finding requests have a Report"),
            finding_ids,
            selection_evidence_summary: None,
            selection_evidence_full: None,
            uncertainty: None,
        }),
    ))
}

fn normalize_agent_plan(
    store: &StateStore,
    mut input: Value,
    scan: &ScanResult,
    state_dir: &Path,
) -> Result<(Value, Vec<Value>, PlanOrigin)> {
    if input["operations"]
        .as_array()
        .is_some_and(|operations| !operations.is_empty())
    {
        bail!(
            "Agent-authored Plans may not contain filesystem operations; use declarative source_updates"
        );
    }
    let raw_updates = input
        .as_object_mut()
        .and_then(|object| object.remove("source_updates"))
        .unwrap_or_else(|| json!([]));
    let requests: Vec<SourceUpdateRequest> = serde_json::from_value(raw_updates)?;
    let raw_library_changes = input
        .as_object_mut()
        .and_then(|object| object.remove("library_changes"))
        .unwrap_or_else(|| json!([]));
    let library_requests: Vec<LibraryChangeRequest> = serde_json::from_value(raw_library_changes)?;
    let roster_changes: Vec<change::RosterChange> = serde_json::from_value(
        input
            .get("roster_changes")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )?;
    let request_group_count = usize::from(!requests.is_empty())
        + usize::from(!library_requests.is_empty())
        + usize::from(!roster_changes.is_empty());
    if request_group_count > 1 {
        bail!("Roster, source, and Library governance must be prepared as separate Plans");
    }
    if !library_requests.is_empty() {
        return normalize_library_plan(input, scan, state_dir, library_requests);
    }
    if requests.is_empty() {
        if !roster_changes.is_empty() {
            let derived = crate::roster_plan::derive(scan, state_dir, &roster_changes)?;
            input["operations"] = Value::Array(derived.operations);
            input["library_changes"] = serde_json::to_value(derived.implicit_library_changes)?;
            return Ok((input, vec![derived.impact], PlanOrigin::RosterGovernance));
        }
        return Ok((input, Vec::new(), PlanOrigin::Agent));
    }

    let mut operations = Vec::new();
    let mut actions = Vec::new();
    let mut diffs = Vec::new();
    let mut placements = std::collections::HashSet::new();
    for request in requests {
        if !placements.insert(request.placement_id.clone()) {
            bail!(
                "Plan contains duplicate source updates for placement {}",
                request.placement_id
            );
        }
        let skill = scan
            .skills
            .iter()
            .find(|skill| skill.id == request.skill_id)
            .ok_or_else(|| anyhow!("Skill {} is not in the latest Snapshot", request.skill_id))?;
        let placement = scan
            .placements
            .iter()
            .find(|placement| placement.id == request.placement_id)
            .ok_or_else(|| {
                anyhow!(
                    "Placement {} is not in the latest Snapshot",
                    request.placement_id
                )
            })?;
        if placement.skill_id != request.skill_id {
            bail!(
                "Placement {} does not belong to Skill {}",
                request.placement_id,
                request.skill_id
            );
        }
        if !placement.is_mutable() {
            bail!(
                "Placement {} has read-only mutation_scope {}; source updates require mutable",
                request.placement_id,
                placement
                    .mutation_scope
                    .map(scan::MutationScope::id)
                    .unwrap_or("unknown")
            );
        }
        if placement.content_digest != request.current_fingerprint {
            bail!(
                "Placement {} fingerprint drifted from the submitted current_fingerprint",
                request.placement_id
            );
        }
        if skill.metadata.source.as_deref() != Some(request.source.as_str()) {
            bail!("Skill {} source metadata does not match", request.skill_id);
        }
        let current_revision = skill
            .metadata
            .version
            .as_deref()
            .or(skill.metadata.revision.as_deref());
        if current_revision != Some(request.current_revision.as_str()) {
            bail!(
                "Skill {} revision metadata does not match",
                request.skill_id
            );
        }
        if request.upstream_revision.trim().is_empty()
            || request.upstream_revision == request.current_revision
        {
            bail!("upstream_revision must identify a different revision");
        }
        if request.upstream_content.len() > 2 * 1024 * 1024 {
            bail!("upstream_content exceeds the 2 MiB source-update limit");
        }
        if placement.link_target.is_some()
            || std::fs::symlink_metadata(&placement.entrypoint)?
                .file_type()
                .is_symlink()
        {
            bail!("source update must target an owned regular-file placement");
        }
        let current_content = std::fs::read_to_string(&placement.entrypoint)?;
        let actual_file_digest = content_digest(current_content.as_bytes());
        let baseline = store.source_baseline(&request.source, &request.current_revision)?;
        let trusted_digest = baseline
            .as_ref()
            .and_then(|baseline| baseline.trusted_digest.as_deref());
        if let Some(submitted) = request.base_digest.as_deref() {
            let submitted = normalized_digest(submitted)?;
            let stored = baseline.as_ref().ok_or_else(|| {
                anyhow!(
                    "no stored source baseline exists for {}@{}; submitted base_digest cannot be trusted",
                    request.source,
                    request.current_revision
                )
            })?;
            let expected = stored
                .trusted_digest
                .as_deref()
                .unwrap_or(&stored.entrypoint_digest);
            if submitted != expected {
                bail!(
                    "submitted base_digest does not match the immutable source baseline for {}@{}",
                    request.source,
                    request.current_revision
                );
            }
        }
        let upstream_digest = normalized_digest(&request.upstream_digest)?;
        if content_digest(request.upstream_content.as_bytes()) != upstream_digest {
            bail!("upstream_digest does not match upstream_content");
        }
        let upstream_metadata = scan::parse_skill_markdown(&request.upstream_content);
        if upstream_metadata.source.as_deref() != Some(request.source.as_str())
            || upstream_metadata
                .version
                .as_deref()
                .or(upstream_metadata.revision.as_deref())
                != Some(request.upstream_revision.as_str())
        {
            bail!("upstream_content metadata does not match source and upstream_revision");
        }
        if actual_file_digest == upstream_digest {
            bail!("placement already has the submitted upstream content");
        }
        if let Some(existing_upstream) =
            store.source_baseline(&request.source, &request.upstream_revision)?
        {
            if let Some(trusted_upstream) = existing_upstream.trusted_digest {
                if trusted_upstream != upstream_digest {
                    bail!(
                        "submitted upstream content conflicts with the trusted source baseline for {}@{}",
                        request.source,
                        request.upstream_revision
                    );
                }
            }
        }
        let baseline_trusted = trusted_digest.is_some();
        let local_modified = trusted_digest.is_none_or(|digest| actual_file_digest != digest);
        if (!baseline_trusted || local_modified) && request.choice.is_none() {
            if baseline_trusted {
                bail!(
                    "local modification detected for placement {}; choice is required",
                    request.placement_id
                );
            }
            bail!(
                "source baseline for {}@{} is first-observed and untrusted; an explicit choice is required",
                request.source,
                request.current_revision
            );
        }
        let choice = request.choice.unwrap_or(SourceUpdateChoice::AdoptUpstream);
        let choice_name = match choice {
            SourceUpdateChoice::RetainLocal => "retain_local",
            SourceUpdateChoice::AdoptUpstream => "adopt_upstream",
            SourceUpdateChoice::PreserveBoth => "preserve_both",
        };
        let expected_file_fingerprint = change::fingerprint(&placement.entrypoint)?.to_string();
        let action_target = placement.entrypoint.clone();
        match choice {
            SourceUpdateChoice::RetainLocal => {}
            SourceUpdateChoice::AdoptUpstream => operations.push(json!({
                "kind": "replace_file",
                "target": placement.entrypoint,
                "content": request.upstream_content,
                "expected_fingerprint": expected_file_fingerprint
            })),
            SourceUpdateChoice::PreserveBoth => {
                let sibling =
                    source_update_sibling(&placement.entrypoint, &request.upstream_revision)?;
                operations.push(json!({
                    "kind": "write_file",
                    "target": sibling,
                    "content": request.upstream_content,
                    "expected_fingerprint": "missing"
                }));
            }
        }
        actions.push(json!({
            "skill_id": request.skill_id,
            "placement_id": request.placement_id,
            "choice": choice_name,
            "source": request.source,
            "from_revision": request.current_revision,
            "to_revision": request.upstream_revision,
            "current_digest": request.current_fingerprint,
            "expected_file_fingerprint": expected_file_fingerprint,
            "upstream_digest": upstream_digest,
            "baseline_trusted": baseline_trusted,
            "choice_reason": if baseline_trusted {
                if local_modified { "trusted_baseline_local_modification" } else { "trusted_baseline_clean" }
            } else {
                "first_observed_baseline_untrusted"
            },
            "target": action_target
        }));
        let mut diff = diff_summary(
            &current_content,
            &request.upstream_content,
            local_modified,
            choice_name,
            &request.placement_id,
        );
        diff["baseline_trusted"] = json!(baseline_trusted);
        diff["choice_reason"] = json!(if baseline_trusted {
            if local_modified {
                "trusted_baseline_local_modification"
            } else {
                "trusted_baseline_clean"
            }
        } else {
            "first_observed_baseline_untrusted"
        });
        diffs.push(diff);
    }
    input["operations"] = Value::Array(operations);
    input["source_updates"] = Value::Array(actions);
    Ok((input, diffs, PlanOrigin::SourceUpdate))
}

fn normalized_digest(value: &str) -> Result<String> {
    let digest = value.strip_prefix("sha256:").unwrap_or(value);
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("digest must be a 64-character SHA-256 hex value");
    }
    Ok(digest.to_ascii_lowercase())
}

fn resolve_path_with_missing_tail(path: &Path) -> PathBuf {
    let mut ancestor = path;
    let mut missing = Vec::<std::ffi::OsString>::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            return path.to_path_buf();
        };
        missing.push(name.to_owned());
        let Some(parent) = ancestor.parent() else {
            return path.to_path_buf();
        };
        ancestor = parent;
    }
    let mut resolved = std::fs::canonicalize(ancestor).unwrap_or_else(|_| ancestor.to_path_buf());
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    resolved
}

fn overlapping_agent_skill_roots(scan: &ScanResult, library_root: &Path) -> Vec<PathBuf> {
    let library_root = resolve_path_with_missing_tail(library_root);
    let mut roots = scan
        .roots
        .iter()
        .filter(|root| root.kind == RootKind::Skills && root.agent.is_some())
        .map(|root| resolve_path_with_missing_tail(&root.path))
        .filter(|root| library_root.starts_with(root) || root.starts_with(&library_root))
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

fn normalize_library_plan(
    mut input: Value,
    scan: &ScanResult,
    state_dir: &Path,
    requests: Vec<LibraryChangeRequest>,
) -> Result<(Value, Vec<Value>, PlanOrigin)> {
    let library_root = state_dir.join("library");
    let backup_root = state_dir.join("plan-backups");
    let nonce = ulid::Ulid::new().to_string();
    let mut operations = Vec::new();
    let mut actions = Vec::new();
    let mut impacts = Vec::new();
    let mut touched_skills = std::collections::HashSet::new();
    let mut needs_library_root = false;
    let mut needs_backup_root = false;

    if requests
        .iter()
        .any(|request| matches!(request.requested_state, RequestedGovernanceState::Hosted))
    {
        let agent_roots = overlapping_agent_skill_roots(scan, &library_root);
        if !agent_roots.is_empty() {
            return Err(LibraryRootConflict {
                library_root,
                agent_roots,
            }
            .into());
        }
    }

    for request in requests {
        if !touched_skills.insert(request.skill_id.clone()) {
            bail!(
                "Plan contains duplicate Library changes for {}",
                request.skill_id
            );
        }
        let skill = scan
            .skills
            .iter()
            .find(|skill| skill.id == request.skill_id)
            .ok_or_else(|| anyhow!("Skill {} is not in the latest Snapshot", request.skill_id))?;
        let all_placements = scan
            .placements
            .iter()
            .filter(|placement| placement.skill_id == request.skill_id)
            .collect::<Vec<_>>();
        if let Some(blocker) =
            incomplete_fingerprint_blocker(all_placements.iter().copied(), "plan")
        {
            return Err(blocker.into());
        }
        if all_placements
            .iter()
            .any(|placement| !placement.is_mutable())
        {
            let scopes = all_placements
                .iter()
                .filter(|placement| !placement.is_mutable())
                .map(|placement| {
                    placement
                        .mutation_scope
                        .map(scan::MutationScope::id)
                        .unwrap_or("unknown")
                })
                .collect::<BTreeSet<_>>();
            bail!(
                "Library change for {} includes read-only placements with mutation_scope {:?}",
                request.skill_id,
                scopes
            );
        }
        let expected_ids = all_placements
            .iter()
            .map(|placement| placement.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let submitted_ids = request
            .placement_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        if expected_ids != submitted_ids || request.placement_ids.len() != submitted_ids.len() {
            bail!(
                "Library change for {} must include every placement exactly once",
                request.skill_id
            );
        }
        let canonical = all_placements
            .iter()
            .find(|placement| placement.id == request.canonical_placement_id)
            .copied()
            .ok_or_else(|| anyhow!("canonical placement is not part of the requested Skill"))?;
        if canonical.link_target.is_some()
            || !std::fs::symlink_metadata(&canonical.directory)?.is_dir()
        {
            bail!("canonical placement must be an owned real directory");
        }
        if all_placements
            .iter()
            .any(|placement| placement.content_digest != canonical.content_digest)
        {
            bail!("all placements must have the canonical placement's exact digest");
        }
        let mut physical_groups =
            std::collections::BTreeMap::<PathBuf, Vec<&scan::SkillPlacement>>::new();
        for placement in &all_placements {
            physical_groups
                .entry(placement.validated_physical_directory()?)
                .or_default()
                .push(*placement);
        }
        let before_physical_source_count = physical_groups.len();
        let before_placement_count = all_placements.len();
        let default_exposed_placement_count = all_placements
            .iter()
            .filter(|placement| placement.default_exposed)
            .count();
        let canonical_physical = canonical.validated_physical_directory()?;
        if !placement_owns_physical_source(canonical) {
            bail!("canonical placement must be the owned physical source directory");
        }
        let state_name = match request.requested_state {
            RequestedGovernanceState::Managed => "managed",
            RequestedGovernanceState::Hosted => "hosted",
        };
        let safe_name = safe_skill_directory_name(&skill.name)?;
        let library_path = library_root.join(safe_name);
        let adds_hosted_library_placement =
            matches!(request.requested_state, RequestedGovernanceState::Hosted)
                && canonical.directory != library_path;
        let after_placement_count =
            before_placement_count + usize::from(adds_hosted_library_placement);
        let canonical_fingerprint = change::fingerprint(&canonical.directory)?;
        let link_source = match request.requested_state {
            RequestedGovernanceState::Managed => canonical.directory.clone(),
            RequestedGovernanceState::Hosted => {
                needs_library_root |= !library_root.exists();
                if canonical.directory != library_path {
                    if library_path.exists() {
                        bail!("Library target {} already exists", library_path.display());
                    }
                    operations.push(json!({
                        "kind": "move_recoverable",
                        "source": canonical.directory,
                        "target": library_path,
                        "expected_fingerprint": canonical_fingerprint
                    }));
                    operations.push(json!({
                        "kind": "create_symlink",
                        "source": library_path,
                        "target": canonical.directory,
                        "expected_fingerprint": "missing",
                        "expected_source_fingerprint": canonical_fingerprint
                    }));
                }
                library_path.clone()
            }
        };

        let mut relinked = usize::from(adds_hosted_library_placement);
        for (physical_source, mut placements) in physical_groups {
            if physical_source == canonical_physical {
                continue;
            }
            placements.sort_by(|left, right| left.directory.cmp(&right.directory));
            let placement = placements
                .iter()
                .copied()
                .find(|placement| placement_owns_physical_source(placement))
                .ok_or_else(|| {
                    anyhow!(
                        "Physical source {} has no owned real placement; state may have drifted, run skillroster scan",
                        physical_source.display()
                    )
                })?;
            if !std::fs::symlink_metadata(&placement.directory)?.is_dir() {
                bail!(
                    "Physical source {} drifted from an owned real directory; run skillroster scan",
                    physical_source.display()
                );
            }
            needs_backup_root |= !backup_root.exists();
            let backup = backup_root.join(format!("{nonce}-{}", placement.id));
            operations.push(json!({
                "kind": "move_recoverable",
                "source": placement.directory,
                "target": backup,
                "expected_fingerprint": change::fingerprint(&placement.directory)?
            }));
            operations.push(json!({
                "kind": "create_symlink",
                "source": link_source,
                "target": placement.directory,
                "expected_fingerprint": "missing",
                "expected_source_fingerprint": canonical_fingerprint
            }));
            relinked += placements.len();
        }
        actions.push(json!({
            "skill_id": request.skill_id,
            "canonical_placement_id": request.canonical_placement_id,
            "placement_ids": request.placement_ids,
            "requested_state": state_name,
            "canonical_path": canonical.directory,
            "library_path": if matches!(request.requested_state, RequestedGovernanceState::Hosted) {
                Some(library_path)
            } else {
                None
            }
        }));
        impacts.push(json!({
            "skill_id": request.skill_id,
            "before": {
                "placement_count": before_placement_count,
                "physical_source_count": before_physical_source_count,
                "default_exposed_placement_count": default_exposed_placement_count,
                "canonical_path": canonical.directory,
                "governance_state": "observed"
            },
            "after": {
                "placement_count": after_placement_count,
                "physical_source_count": 1,
                "default_exposed_placement_count": default_exposed_placement_count,
                "governance_state": state_name,
                "canonical_path": link_source,
                "relinked_placement_count": relinked
            },
            "delta": {
                "placement_count": after_placement_count as i64 - before_placement_count as i64,
                "physical_source_count": 1_i64 - before_physical_source_count as i64,
                "default_exposed_placement_count": 0
            }
        }));
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
    }
    if operations.is_empty() {
        bail!("Library change has no effective filesystem change");
    }
    input["operations"] = Value::Array(operations);
    input["library_changes"] = Value::Array(actions);
    Ok((input, impacts, PlanOrigin::LibraryGovernance))
}

fn safe_skill_directory_name(name: &str) -> Result<String> {
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
        bail!("Skill name cannot form a safe Library directory");
    }
    Ok(safe)
}

fn content_digest(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn source_update_sibling(entrypoint: &Path, revision: &str) -> Result<PathBuf> {
    let safe_revision = revision
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if safe_revision.is_empty() {
        bail!("upstream_revision cannot form a safe sibling name");
    }
    Ok(entrypoint.with_file_name(format!("SKILL.upstream-{safe_revision}.md")))
}

fn diff_summary(
    current: &str,
    upstream: &str,
    local_modified: bool,
    choice: &str,
    placement_id: &str,
) -> Value {
    let current_lines = current.lines().collect::<Vec<_>>();
    let upstream_lines = upstream.lines().collect::<Vec<_>>();
    let prefix = current_lines
        .iter()
        .zip(&upstream_lines)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = current_lines[prefix..]
        .iter()
        .rev()
        .zip(upstream_lines[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    json!({
        "placement_id": placement_id,
        "choice": choice,
        "local_modification_detected": local_modified,
        "current_lines": current_lines.len(),
        "upstream_lines": upstream_lines.len(),
        "removed_lines": current_lines.len().saturating_sub(prefix + suffix),
        "added_lines": upstream_lines.len().saturating_sub(prefix + suffix)
    })
}

fn prepare_plan(
    store: &StateStore,
    state_dir: &Path,
    input: Value,
    origin: PlanOrigin,
    reuse_identity: Option<Value>,
    action_argv_prefix: &[String],
) -> Result<Value> {
    if store.recovery_required()? {
        bail!("recovery is required before another Plan can be prepared");
    }
    let (scan_id, scan) = latest_scan(store)?;
    require_content_identity(&scan)?;
    let (input, finding_provenance) = if matches!(origin, PlanOrigin::Agent) {
        let (input, roster_provenance) = expand_finding_roster_changes(
            store,
            input,
            &scan_id,
            &scan,
            state_dir,
            action_argv_prefix,
        )?;
        let (input, library_provenance) =
            expand_finding_library_changes(store, input, &scan_id, &scan)?;
        (input, roster_provenance.or(library_provenance))
    } else {
        (input, None)
    };
    let canonical_state_dir = std::fs::canonicalize(state_dir)?;
    let requested_scan_id = input["scan_id"]
        .as_str()
        .ok_or_else(|| anyhow!("Plan scan_id is required"))?;
    if requested_scan_id != scan_id.as_str() {
        bail!("Plan references {requested_scan_id}, latest Snapshot is {scan_id}");
    }
    if matches!(origin, PlanOrigin::Agent)
        && ["roster_changes", "source_updates", "library_changes"]
            .iter()
            .any(|key| {
                input
                    .get(*key)
                    .and_then(Value::as_array)
                    .is_some_and(|values| !values.is_empty())
            })
        && input
            .get("evidence_ids")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    {
        bail!("missing_evidence: governance Plans must cite Evidence from the latest Snapshot");
    }
    let (input, source_update_diffs, effective_origin) = match origin {
        PlanOrigin::Agent => normalize_agent_plan(store, input, &scan, &canonical_state_dir)?,
        _ => (input, Vec::new(), origin),
    };
    let encoded = serde_json::to_string(&input)?;
    let prepared = change::prepare(
        &encoded,
        &PrepareContext {
            approved_roots: {
                let mut roots = approved_roots(&scan);
                roots.push(canonical_state_dir.join("library"));
                roots.push(canonical_state_dir.join("plan-backups"));
                roots
            },
            state_dir: state_dir.to_path_buf(),
            operation_policy: match effective_origin {
                PlanOrigin::Agent => OperationPolicy::GovernanceOnly,
                PlanOrigin::RosterGovernance => OperationPolicy::LibraryGovernance,
                PlanOrigin::BootstrapSetup => OperationPolicy::BootstrapSetup,
                PlanOrigin::SourceUpdate => OperationPolicy::SourceUpdate,
                PlanOrigin::LibraryGovernance => OperationPolicy::LibraryGovernance,
            },
        },
    )?;
    if prepared.scan_id != scan_id.as_str() {
        bail!(
            "Plan references {}, latest Snapshot is {scan_id}",
            prepared.scan_id
        );
    }
    validate_roster_changes(store, &prepared, &scan)?;
    validate_source_update_preconditions(&prepared, &scan)?;
    validate_plan_evidence(store, &prepared, &scan_id)?;
    if matches!(effective_origin, PlanOrigin::BootstrapSetup) {
        let prepared_scan_id = ScanId::parse(prepared.scan_id.clone())?;
        if let Some(existing) = store.ready_plan_with_reuse_identity(
            &prepared_scan_id,
            &prepared.digest,
            &input,
            reuse_identity.as_ref(),
        )? {
            if let Some(summary) = existing.input.get("summary") {
                return Ok(summary.clone());
            }
        }
    }
    let roster_before = capture_roster_state(store, &prepared)?;
    let library_before = capture_library_state(store, &prepared)?;
    let roster_after = serde_json::to_value(&prepared.roster_changes)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    let library_after = serde_json::to_value(&prepared.library_changes)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    let effective_roster_change_count =
        transition_change_count(&roster_before, &roster_after, "state");
    let effective_library_change_count =
        transition_change_count(&library_before, &library_after, "requested_state");
    if matches!(effective_origin, PlanOrigin::Agent)
        && prepared.source_updates.is_empty()
        && effective_roster_change_count == 0
    {
        bail!("Plan has no effective Roster state change");
    }
    let change_summary = json!({
        "operation_count": prepared.operations.len(),
        "roster_change_count": effective_roster_change_count,
        "library_change_count": effective_library_change_count,
        "source_update_count": prepared.source_updates.len()
    });
    let mut impact = if matches!(effective_origin, PlanOrigin::RosterGovernance) {
        source_update_diffs
            .first()
            .cloned()
            .unwrap_or_else(|| json!({}))
    } else {
        json!(source_update_diffs)
    };
    if let Some(object) = impact.as_object_mut() {
        object.insert(
            "roster".into(),
            json!({
                "before": roster_before,
                "after": prepared.roster_changes
            }),
        );
        object.insert(
            "library".into(),
            json!({
                "before": library_before,
                "after": prepared.library_changes
            }),
        );
        object.insert("operation_count".into(), json!(prepared.operations.len()));
    }
    let finding_ids = finding_provenance
        .as_ref()
        .map(|provenance| provenance.finding_ids.clone())
        .unwrap_or_default();
    let report_id = finding_provenance
        .as_ref()
        .map(|provenance| provenance.report_id.clone());
    let selection_evidence_summary = finding_provenance
        .as_ref()
        .and_then(|provenance| provenance.selection_evidence_summary.clone());
    let selection_evidence_full = finding_provenance
        .as_ref()
        .and_then(|provenance| provenance.selection_evidence_full.clone());
    let uncertainty = finding_provenance
        .as_ref()
        .and_then(|provenance| provenance.uncertainty.clone());
    let risk = plan_risk(&prepared);
    let operations_by_kind = operation_groups(&prepared.operations);
    let impact = bounded_plan_impact(impact);
    let affected = affected_summary(&prepared, &scan, &impact);
    let diff_summary = plan_diff_summary(
        effective_origin,
        &source_update_diffs,
        &change_summary,
        &operations_by_kind,
        &impact,
    );
    let mut detail_contains = vec![
        "operations",
        "roster_changes",
        "source_updates",
        "library_changes",
        "before_state",
    ];
    if selection_evidence_full.is_some() {
        detail_contains.push("complete_core_selections");
    }
    let mut summary = json!({
        "detail_level": "summary",
        "plan_id": prepared.id,
        "snapshot_id": prepared.scan_id,
        "digest": prepared.digest,
        "state": "ready",
        "evidence": bounded_ids(prepared.evidence_ids.clone()),
        "findings": bounded_ids(
            finding_ids.iter().map(ToString::to_string).collect::<Vec<_>>()
        ),
        "change_summary": change_summary,
        "operation_groups": operations_by_kind,
        "affected": affected,
        "diff_summary": diff_summary,
        "impact": impact,
        "detail": {
            "available": true,
            "command": ["plan", "--show", prepared.id, "--json"],
            "contains": detail_contains
        },
        "risk": risk,
        "reversible": true,
        "canonical_deletion_count": 0,
        "confirmation_required": true,
        "files_changed": false
    });
    let summary_object = summary
        .as_object_mut()
        .expect("Plan summary is always an object");
    if let Some(selection_evidence) = selection_evidence_summary {
        summary_object.insert("selection_evidence".into(), selection_evidence);
    }
    if let Some(uncertainty) = uncertainty {
        summary_object.insert("uncertainty".into(), uncertainty);
    }
    let physical_bindings = capture_physical_placement_bindings(&prepared, &scan);
    store.save_plan(&plan_record(
        &prepared,
        input,
        roster_before.clone(),
        library_before.clone(),
        report_id,
        finding_ids.clone(),
        summary.clone(),
        selection_evidence_full,
        reuse_identity,
        physical_bindings,
    )?)?;
    Ok(summary)
}

fn apply_command(store: &StateStore, id: &str) -> Result<Value> {
    if store.recovery_required()? {
        bail!("recovery is required before Apply can continue");
    }
    let id = PlanId::parse(id.to_string())?;
    let record = store
        .get_plan(&id)?
        .ok_or_else(|| anyhow!("Plan {id} does not exist"))?;
    if record.status != PlanStatus::Ready {
        bail!("Plan {id} is not ready");
    }
    let prepared: PreparedPlan = serde_json::from_value(record.input["prepared"].clone())?;
    require_clear_journals(store, &prepared.state_dir)?;
    let (latest_scan_id, scan) = latest_scan(store)?;
    if latest_scan_id != record.scan_id {
        return Err(PlanSnapshotDrift {
            plan_id: id,
            expected_snapshot_id: record.scan_id,
            current_snapshot_id: latest_scan_id,
        }
        .into());
    }
    validate_governance_fingerprint_completeness(&prepared, &scan)?;
    validate_physical_placement_bindings(&record, &prepared, &scan)?;
    validate_roster_changes(store, &prepared, &scan)?;
    validate_source_update_preconditions(&prepared, &scan)?;
    validate_plan_evidence(store, &prepared, &latest_scan_id)?;
    let roster_before = capture_roster_state(store, &prepared)?;
    if Value::Array(roster_before.clone()) != record.input["roster_before"] {
        bail!("Plan {id} is stale; Roster state has drifted");
    }
    let library_before = capture_library_state(store, &prepared)?;
    if Value::Array(library_before.clone()) != record.input["library_before"] {
        bail!("Plan {id} is stale; Library governance state has drifted");
    }
    store.update_plan_status(&id, PlanStatus::Applying)?;
    let mut outcome = match change::apply_locked(&prepared) {
        Ok(outcome) => outcome,
        Err(error) => {
            let next = if journal_issues(store, &prepared.state_dir)?.is_empty() {
                PlanStatus::Ready
            } else {
                PlanStatus::RecoveryRequired
            };
            store.update_plan_status(&id, next)?;
            return Err(error.into());
        }
    };
    let mut next = match outcome.receipt.status {
        change::ReceiptStatus::Applied => PlanStatus::Applied,
        change::ReceiptStatus::FailedRolledBack => PlanStatus::FailedRolledBack,
        _ => PlanStatus::RecoveryRequired,
    };
    if outcome.verification_passed {
        let metadata_result = apply_roster_changes(store, &prepared)
            .and_then(|()| apply_library_changes(store, &prepared));
        if let Err(error) = metadata_result {
            let roster_restored =
                restore_roster_state(store, &Value::Array(roster_before.clone())).is_ok();
            let library_restored =
                restore_library_state(store, &Value::Array(library_before.clone())).is_ok();
            let filesystem_rollback = change::rollback_apply_locked(&outcome.receipt)?;
            let recovered =
                roster_restored && library_restored && filesystem_rollback.verification_passed;
            outcome.receipt = filesystem_rollback.receipt;
            outcome.receipt.status = if recovered {
                change::ReceiptStatus::FailedRolledBack
            } else {
                change::ReceiptStatus::RecoveryRequired
            };
            outcome.receipt.error = Some(format!("Governance metadata update failed: {error}"));
            outcome.verification_passed = false;
            next = if recovered {
                PlanStatus::FailedRolledBack
            } else {
                PlanStatus::RecoveryRequired
            };
        }
    }
    change::persist_journal_state(&outcome.receipt)?;
    let receipt = receipt_record(
        &outcome.receipt,
        &record
            .operations
            .iter()
            .map(|operation| (operation.position, operation.id.clone()))
            .collect::<Vec<_>>(),
        None,
        outcome.verification_passed,
        json!({"before": roster_before, "after": prepared.roster_changes}),
        json!(prepared.source_updates),
        json!(prepared.evidence_ids),
        json!({"before": library_before, "after": prepared.library_changes}),
    )?;
    let trusted_sources = if outcome.verification_passed && next == PlanStatus::Applied {
        prepared
            .source_updates
            .iter()
            .filter(|update| matches!(update.choice.as_str(), "adopt_upstream" | "preserve_both"))
            .map(|update| crate::sqlite::TrustedSourceBaseline {
                source: update.source.clone(),
                revision: update.to_revision.clone(),
                digest: update.upstream_digest.clone(),
                scan_id: latest_scan_id.clone(),
                observed_at: Utc::now().timestamp(),
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    store
        .save_apply_receipt_with_trusted_sources(&id, next, &receipt, &trusted_sources)
        .with_context(|| {
            format!(
                "filesystem journal {} is durable but SQLite Apply finalization failed; run lifecycle recovery --json",
                receipt.id
            )
        })?;
    Ok(mutation_result(&outcome, &receipt, None))
}

fn undo_command(store: &StateStore, id: &str) -> Result<Value> {
    let id = ReceiptId::parse(id.to_string())?;
    let record = store
        .get_receipt(&id)?
        .ok_or_else(|| anyhow!("Receipt {id} does not exist"))?;
    if record.status != ReceiptStatus::Applied {
        bail!("Receipt {id} is not undoable");
    }
    if let Some(reverse) = store.reverse_receipt_for(&id)? {
        bail!("Receipt {id} was already undone by {reverse}");
    }
    let original: ChangeReceipt =
        serde_json::from_value(record.verification["change_receipt"].clone())?;
    require_clear_journals(store, &original.state_dir)?;
    let mut outcome = change::undo_locked(&original)?;
    if outcome.verification_passed {
        if let Err(error) =
            restore_roster_state(store, &record.verification["roster_state"]["before"]).and_then(
                |()| restore_library_state(store, &record.verification["library_state"]["before"]),
            )
        {
            outcome.receipt.status = change::ReceiptStatus::RecoveryRequired;
            outcome.receipt.error = Some(format!("Roster restoration failed: {error}"));
            outcome.verification_passed = false;
        }
    }
    change::persist_journal_state(&outcome.receipt)?;
    let receipt = receipt_record(
        &outcome.receipt,
        &record
            .operation_results
            .iter()
            .map(|operation| (operation.position, operation.operation_id.clone()))
            .collect::<Vec<_>>(),
        Some(id.clone()),
        outcome.verification_passed,
        json!({
            "before": record.verification["roster_state"]["after"],
            "after": record.verification["roster_state"]["before"]
        }),
        record.verification["source_updates"].clone(),
        record.verification["evidence_ids"].clone(),
        json!({
            "before": record.verification["library_state"]["after"],
            "after": record.verification["library_state"]["before"]
        }),
    )?;
    if outcome.verification_passed {
        store
            .save_undo_receipt(&id, &receipt)
            .with_context(|| {
                format!(
                    "Undo journal {} is durable but SQLite finalization failed; run lifecycle recovery --json",
                    receipt.id
                )
            })?;
    } else {
        store.save_receipt(&receipt)?;
    }
    Ok(mutation_result(&outcome, &receipt, Some(id)))
}

const LEGACY_SINGLE_FILE_BOOTSTRAPS: &[(&str, &str)] = &[
    (
        "1.0.0",
        "08440a4a3e10489eae2484d0a5bf8f4b0451d22c43edaa90c75fcd0b66fd4a74",
    ),
    (
        "1.0.1",
        "2e18b702cbe61660aafff0b4c01538b51561cd0ce979cc4cce914f539b8c755e",
    ),
    (
        "1.0.2",
        "4a6c9de673948dbccfe719973395b7712fe09706ae22b2119e4a8ed8c5446a73",
    ),
    (
        "1.0.3",
        "1c0ffe7e1ef65d88e5c42a8978d92de81353c69072d8e610dce0c33d3938290c",
    ),
    (
        "1.0.4",
        "629d2d959c5b4277398e48f7b10c29c6c2adb0c0415723938ad7bbe4f8fec1c8",
    ),
    (
        "1.1.0",
        "d463d28295055fde8012e5c2424a754bf93e8c84c9fd6b48b7194bbaff0b27c2",
    ),
    (
        "1.2.0",
        "a12e3bbf45e4a3d51b8075dd2b56a410f113fb27cd803676fb614bd88f278f9c",
    ),
    (
        "1.3.0",
        "0fe2e7b8aed1f6db1c57a3d5b6efc247b795e1af01d5422b9168e707bc8c7b47",
    ),
    (
        "1.4.0",
        "2161ba4834fb350afee75fd6ab90ecff49741c971bb2d670b8d3c9f6588f9b70",
    ),
    (
        "1.5.0",
        "0afb58572bf602024f78e0f8c7312cc3485abb184746898945d1f7501a5425b5",
    ),
    (
        "1.5.1",
        "abb33e147e1b092ff70a2b02c5a8f89fc70d0c73eed2d8ec5ea88bba1ae58221",
    ),
    (
        "1.6.0",
        "c5f05692527bb2b8c45012ba6a464268b6a91a683a429a11dbe67022bbcc2aa5",
    ),
    (
        "1.7.0",
        "8b503801efa6a5dd13c8767647654bb9fcd4dcfaf10cdbbb61429dfe29995129",
    ),
    (
        "1.7.1",
        "f742e5d1af14309e3fa7e95378a207fafed3d09e477b1df9c9e76d65e4eec304",
    ),
    (
        "1.8.0",
        "820b760c5f6cc8c2ad6695e211044882077af138014579236b8824039fd1f394",
    ),
    (
        "1.8.1",
        "996891144bca51a654fc51294cd6d63c41df4afb5796af7846480b168f69edc6",
    ),
    (
        "1.8.2",
        "d79be31fe878267d00f21cf8e198443ebf8b95eb6631fc2395f132f12289e3e5",
    ),
    (
        "1.8.3",
        "3eba54753cfe8cdf987a8a4fe1ab1337317aef9b72e55b9be49b3158117470b1",
    ),
    (
        "1.8.4",
        "2758bed89792128c47d01f613a4df277dd32c9641cb036e7ed1f03c0c94e8381",
    ),
    (
        "1.8.5",
        "9f25609e002ad63e750214c54d7138f6e877849e46626aef441e00eaca8738d1",
    ),
    (
        "1.8.6",
        "4412ddc1fa4006492c28f8813462504a68cc71f4025398c3d60d5c225b2b904b",
    ),
    (
        "1.8.7",
        "68007e9c80a942cf4afa581f90639bbd260b69a36cb72aebdbca63e92d787eed",
    ),
    (
        "1.8.8",
        "15dc6c0bd25dc7a6e97336124960b7d548ecea9a3a58b34d55a4d57dbc4d5aeb",
    ),
    (
        "1.8.9",
        "1087110d8f8f9bb2c1839b1eda11b4417ca7b57af4449113a6fb5b1394e3309d",
    ),
    (
        "1.8.10",
        "dbc604dbfc45db5bf766609b28513eacb1ba87a96fc0cba7bb11337561bddd01",
    ),
    (
        "1.8.11",
        "501c0ce97d677b1a3c68d25dd1ece99590895dd51e23e671ea96312925e98c59",
    ),
    (
        "1.8.12",
        "69bf20da3494b69b634aee5e8d7dead8d8e30222f9e380461640af02bc83331e",
    ),
    (
        "1.8.13",
        "3063f8913c22846640389f5a633697aea253ab0805e974bd19fa0f06f2b01682",
    ),
    (
        "1.8.14",
        "ebf48e298ece30cd850146362b3fa73950b4fa26e35f3ce774a5700ae8b3e3cb",
    ),
    (
        "1.8.15",
        "dd08e0f4140989cb2a1178f60e152ee7913377d743f4029b9699122589193d47",
    ),
    (
        "1.8.16",
        "4d46b9699db8490821c1f91be1444dca689cd4b9578a2b732b633de441f8ff59",
    ),
    (
        "1.8.17",
        "5c7cc974e0d83d82993ba1ca9227c0d542288ca3d7ef4e04165794cb41faec33",
    ),
    (
        "1.8.18",
        "130cb488305b53dc3123633da50df6d3b843b24663861f7cc6b63964621b8718",
    ),
    (
        "1.8.19",
        "78aa0dbdf53c3f9f08fb5bf9805e33b2d5b8ffe4c63bac05d88391f608fab16c",
    ),
    (
        "1.8.20",
        "fc1318f0a8587ce193ab1f54462374508026cf8c50a382b613dc18b48f9d09f2",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BootstrapContentStatus {
    Current,
    OfficialOutdated(&'static str),
    Modified,
}

fn bootstrap_content_status(digest: &str, current_digest: &str) -> BootstrapContentStatus {
    if digest == current_digest {
        BootstrapContentStatus::Current
    } else if let Some((version, _)) = LEGACY_SINGLE_FILE_BOOTSTRAPS
        .iter()
        .find(|(_, official_digest)| *official_digest == digest)
    {
        BootstrapContentStatus::OfficialOutdated(version)
    } else {
        BootstrapContentStatus::Modified
    }
}

fn normalized_bootstrap_content(content: &str) -> String {
    content.replace("\r\n", "\n")
}

/// Exact manifests for released Bootstrap packages that already contained all
/// managed files. Add one entry when a package release is superseded; setup
/// never infers an official package by mixing file digests from different
/// releases.
#[derive(Clone, Debug)]
struct BootstrapPackageManifest<'a> {
    version: &'a str,
    file_digests: &'a [(&'a str, &'a str)],
}

const LEGACY_COMPLETE_BOOTSTRAP_PACKAGES: &[BootstrapPackageManifest<'static>] = &[];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BootstrapFileStatus {
    Current,
    OfficialOutdated(&'static str),
    Missing,
    Modified,
    Unsupported,
}

impl BootstrapFileStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::OfficialOutdated(_) => "official_outdated",
            Self::Missing => "missing",
            Self::Modified => "modified",
            Self::Unsupported => "unsupported",
        }
    }
}

fn current_bootstrap_package() -> Vec<(&'static str, String, String)> {
    BOOTSTRAP_PACKAGE_FILES
        .iter()
        .map(|file| {
            let content = normalized_bootstrap_content(file.content);
            (
                file.relative_path,
                content_digest(content.as_bytes()),
                content,
            )
        })
        .collect()
}

fn legacy_bootstrap_package_version<'a>(
    observed_digests: &[Option<String>],
    complete_packages: &'a [BootstrapPackageManifest<'a>],
) -> Option<&'a str> {
    if observed_digests.len() != BOOTSTRAP_PACKAGE_FILES.len() {
        return None;
    }
    if let Some(package) = complete_packages.iter().find(|package| {
        package.file_digests.len() == BOOTSTRAP_PACKAGE_FILES.len()
            && BOOTSTRAP_PACKAGE_FILES
                .iter()
                .zip(observed_digests)
                .all(|(file, observed)| {
                    package
                        .file_digests
                        .iter()
                        .find(|(relative_path, _)| *relative_path == file.relative_path)
                        .is_some_and(|(_, expected)| observed.as_deref() == Some(*expected))
                })
    }) {
        return Some(package.version);
    }
    if observed_digests[1..].iter().all(Option::is_none) {
        let skill_digest = observed_digests[0].as_deref()?;
        return LEGACY_SINGLE_FILE_BOOTSTRAPS
            .iter()
            .find_map(|(version, digest)| (*digest == skill_digest).then_some(*version));
    }
    None
}

fn bootstrap_file_status(
    relative_path: &str,
    path: &Path,
    current_digest: &str,
) -> (BootstrapFileStatus, Option<String>) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_file() => {
            (BootstrapFileStatus::Unsupported, None)
        }
        Ok(_) => match std::fs::read(path) {
            Ok(content) => {
                let digest = bootstrap_content_digest(&content);
                let status = if digest == current_digest {
                    BootstrapFileStatus::Current
                } else if relative_path == "SKILL.md" {
                    match bootstrap_content_status(&digest, current_digest) {
                        BootstrapContentStatus::OfficialOutdated(version) => {
                            BootstrapFileStatus::OfficialOutdated(version)
                        }
                        BootstrapContentStatus::Current => BootstrapFileStatus::Current,
                        BootstrapContentStatus::Modified => BootstrapFileStatus::Modified,
                    }
                } else {
                    BootstrapFileStatus::Modified
                };
                (status, Some(digest))
            }
            Err(_) => (BootstrapFileStatus::Unsupported, None),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (BootstrapFileStatus::Missing, None)
        }
        Err(_) => (BootstrapFileStatus::Unsupported, None),
    }
}

fn bootstrap_content_digest(content: &[u8]) -> String {
    match std::str::from_utf8(content) {
        Ok(content) => content_digest(normalized_bootstrap_content(content).as_bytes()),
        Err(_) => content_digest(content),
    }
}

fn setup_command(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    modified_choice: Option<ModifiedBootstrapChoice>,
) -> Result<Value> {
    setup_command_with_manifests(
        store,
        home,
        state_dir,
        modified_choice,
        LEGACY_COMPLETE_BOOTSTRAP_PACKAGES,
    )
}

fn setup_command_with_manifests<'a>(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    modified_choice: Option<ModifiedBootstrapChoice>,
    legacy_complete_packages: &'a [BootstrapPackageManifest<'a>],
) -> Result<Value> {
    let bootstrap_content_version = content_version()
        .context("bundled Bootstrap Skill is missing metadata.bootstrap-version")?;
    let Some(snapshot) = store.latest_completed_scan()? else {
        return Ok(json!({
            "detected_agents": [],
            "targets": [],
            "plan_id": Value::Null,
            "state": "scan_required",
            "bootstrap_skill": "skillroster",
            "cli_version": env!("CARGO_PKG_VERSION"),
            "bootstrap_content_version": bootstrap_content_version,
            "bootstrap_version": bootstrap_content_version,
            "missing_count": 0,
            "current_count": 0,
            "outdated_count": 0,
            "modified_count": 0,
            "unsupported_count": 0,
            "replace_count": 0,
            "physical_target_count": 0,
            "canonical_deletion_count": 0,
            "files_changed": false,
            "next": "skillroster scan --json"
        }));
    };
    let current_package = current_bootstrap_package();
    let mut detected = Vec::new();
    let mut targets = Vec::new();
    let mut operations = Vec::new();
    let mut missing_count = 0_usize;
    let mut current_count = 0_usize;
    let mut outdated_count = 0_usize;
    let mut modified_count = 0_usize;
    let mut unsupported_count = 0_usize;
    let mut replace_count = 0_usize;
    let mut physical_targets = BTreeSet::new();
    let mut planned_directories = BTreeSet::new();
    let mut planned_files = BTreeSet::new();
    for roots in harness::known_agent_roots(home) {
        for root in roots.skill_roots.into_iter().filter(|path| path.is_dir()) {
            let directory = root.join("skillroster");
            let entrypoint = directory.join("SKILL.md");
            let physical_root = std::fs::canonicalize(&root).with_context(|| {
                format!("failed to resolve Agent Skill root {}", root.display())
            })?;
            let physical_directory = physical_root.join("skillroster");
            let physical_entrypoint = physical_directory.join("SKILL.md");
            physical_targets.insert(physical_directory.clone());
            detected.push(json!({"agent": roots.agent.id(), "target": entrypoint}));
            let directory_is_unsupported = match std::fs::symlink_metadata(&directory) {
                Ok(metadata) => metadata.file_type().is_symlink() || !metadata.file_type().is_dir(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => true,
            };
            let references = directory.join("references");
            let references_are_unsupported = match std::fs::symlink_metadata(&references) {
                Ok(metadata) => metadata.file_type().is_symlink() || !metadata.file_type().is_dir(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => true,
            };
            let package_missing = !directory.exists() && !directory_is_unsupported;
            let observed_files = current_package
                .iter()
                .map(|(relative_path, current_digest, _)| {
                    let (status, digest) = bootstrap_file_status(
                        relative_path,
                        &directory.join(relative_path),
                        current_digest,
                    );
                    (*relative_path, status, digest)
                })
                .collect::<Vec<_>>();
            let observed_digests = observed_files
                .iter()
                .map(|(_, _, digest)| digest.clone())
                .collect::<Vec<_>>();
            let legacy_version =
                legacy_bootstrap_package_version(&observed_digests, legacy_complete_packages);
            let all_managed_files_missing = observed_files
                .iter()
                .all(|(_, status, _)| matches!(status, BootstrapFileStatus::Missing));
            let package_status = if directory_is_unsupported
                || references_are_unsupported
                || observed_files
                    .iter()
                    .any(|(_, status, _)| matches!(status, BootstrapFileStatus::Unsupported))
            {
                "unsupported"
            } else if package_missing || all_managed_files_missing {
                "missing"
            } else if observed_files
                .iter()
                .all(|(_, status, _)| matches!(status, BootstrapFileStatus::Current))
            {
                "current"
            } else if legacy_version.is_some() {
                "official_outdated"
            } else {
                "modified"
            };
            match package_status {
                "unsupported" => unsupported_count += 1,
                "missing" => missing_count += 1,
                "current" => current_count += 1,
                "official_outdated" => {
                    outdated_count += 1;
                    replace_count += 1;
                }
                "modified" => {
                    modified_count += 1;
                    if matches!(modified_choice, Some(ModifiedBootstrapChoice::AdoptCurrent)) {
                        replace_count += 1;
                    }
                }
                _ => unreachable!("package status is exhaustive"),
            }
            let should_install = package_status == "missing"
                || package_status == "official_outdated"
                || (package_status == "modified"
                    && matches!(modified_choice, Some(ModifiedBootstrapChoice::AdoptCurrent)));
            if should_install {
                if package_missing && planned_directories.insert(physical_directory.clone()) {
                    operations.push(json!({
                        "kind": "create_directory",
                        "target": directory,
                        "expected_fingerprint": "missing"
                    }));
                }
                let physical_references = physical_directory.join("references");
                if !references.exists() && planned_directories.insert(physical_references) {
                    operations.push(json!({
                        "kind": "create_directory",
                        "target": references,
                        "expected_fingerprint": "missing"
                    }));
                }
                for ((relative_path, _, content), (_, status, _)) in
                    current_package.iter().zip(&observed_files)
                {
                    if matches!(status, BootstrapFileStatus::Current) {
                        continue;
                    }
                    let target = directory.join(relative_path);
                    let physical_target = physical_directory.join(relative_path);
                    if !planned_files.insert(physical_target) {
                        continue;
                    }
                    let kind = match status {
                        BootstrapFileStatus::Missing => "write_file",
                        BootstrapFileStatus::Modified
                        | BootstrapFileStatus::OfficialOutdated(_) => "replace_file",
                        BootstrapFileStatus::Current | BootstrapFileStatus::Unsupported => {
                            unreachable!("install only receives supported non-current files")
                        }
                    };
                    operations.push(json!({
                        "kind": kind,
                        "target": target,
                        "content": content,
                        "expected_fingerprint": change::fingerprint(&target)?
                    }));
                }
            }
            targets.push(json!({
                "agent": roots.agent.id(),
                "target": entrypoint,
                "physical_target": physical_entrypoint,
                "status": package_status,
                "installed_version": if package_status == "current" {
                    Some(bootstrap_content_version)
                } else if package_status == "official_outdated" {
                    legacy_version
                } else {
                    None
                },
                "managed_file_count": current_package.len(),
                "managed_files": observed_files.iter().map(|(relative_path, status, _)| json!({
                    "relative_path": relative_path,
                    "status": status.as_str()
                })).collect::<Vec<_>>()
            }));
        }
    }
    let base = json!({
        "detected_agents": detected,
        "targets": targets,
        "bootstrap_skill": "skillroster",
        "cli_version": env!("CARGO_PKG_VERSION"),
        "bootstrap_content_version": bootstrap_content_version,
        "bootstrap_version": bootstrap_content_version,
        "missing_count": missing_count,
        "current_count": current_count,
        "outdated_count": outdated_count,
        "modified_count": modified_count,
        "unsupported_count": unsupported_count,
        "replace_count": replace_count,
        "physical_target_count": physical_targets.len(),
        "canonical_deletion_count": 0,
        "files_changed": false
    });
    if unsupported_count > 0 {
        let mut result = base;
        result["plan_id"] = Value::Null;
        result["state"] = json!("unsupported_targets");
        return Ok(result);
    }
    if modified_count > 0 && modified_choice.is_none() {
        let mut result = base;
        result["plan_id"] = Value::Null;
        result["state"] = json!("modified_choice_required");
        result["allowed_modified_choices"] = json!(["retain-local", "adopt-current"]);
        return Ok(result);
    }
    if matches!(modified_choice, Some(ModifiedBootstrapChoice::RetainLocal)) {
        replace_count = outdated_count;
    }
    if operations.is_empty() {
        let mut result = base;
        result["replace_count"] = json!(replace_count);
        result["plan_id"] = Value::Null;
        result["state"] = json!(if modified_count > 0 {
            "local_modifications_retained"
        } else if current_count > 0 {
            "up_to_date"
        } else {
            "no_supported_agent"
        });
        return Ok(result);
    }
    let plan = prepare_plan(
        store,
        state_dir,
        json!({"schema_version": 1, "scan_id": snapshot.id, "operations": operations}),
        PlanOrigin::BootstrapSetup,
        Some(json!({
            "modified_choice": match modified_choice {
                None => Value::Null,
                Some(ModifiedBootstrapChoice::RetainLocal) => json!("retain-local"),
                Some(ModifiedBootstrapChoice::AdoptCurrent) => json!("adopt-current"),
            }
        })),
        &[],
    )?;
    let operation_count = plan["change_summary"]["operation_count"]
        .as_u64()
        .unwrap_or_default();
    let mut result = base;
    result["replace_count"] = json!(replace_count);
    result["plan_id"] = plan["plan_id"].clone();
    result["state"] = json!("preview_ready");
    result["operation_count"] = json!(operation_count);
    result["confirmation_required"] = json!(true);
    Ok(result)
}

fn persist_index(store: &StateStore, scan_id: &ScanId, scan: &ScanResult) -> Result<()> {
    let observed_at = Utc::now().timestamp();
    let mut agent_ids = std::collections::BTreeMap::new();
    for agent in AgentKind::ALL {
        let stored_id = store.save_agent(&AgentRecord {
            id: AgentId::new(),
            kind: model_agent(agent),
            display_name: agent.display_name().into(),
        })?;
        agent_ids.insert(agent, stored_id);
    }

    let mut root_ids = std::collections::BTreeMap::new();
    for root in &scan.roots {
        let path = root.path.to_string_lossy().into_owned();
        let root_id = stable_id(
            "root",
            &format!(
                "{}\0{}\0{path}",
                scan_id.as_str(),
                root.agent.map(AgentKind::id).unwrap_or("shared")
            ),
            RootId::parse,
        )?;
        store.save_root(&RootRecord {
            id: root_id.clone(),
            scan_id: scan_id.clone(),
            agent_id: root.agent.and_then(|agent| agent_ids.get(&agent).cloned()),
            path: path.clone(),
            kind: root_kind(root.kind).into(),
            status: root_status(root.status),
            explicit: root.explicit,
            detail: root.detail.clone(),
        })?;
        root_ids.insert((root.agent, path.clone()), root_id.clone());
        save_reference_evidence(
            store,
            scan_id,
            &format!(
                "path:{}:{path}",
                root.agent.map(AgentKind::id).unwrap_or("shared")
            ),
            EvidenceKind::Path,
            EvidenceQuality::Observed,
            "root",
            root_id.as_str(),
            Some(path),
            None,
            json!({
                "kind": root_kind(root.kind),
                "status": root.status,
                "explicit": root.explicit,
                "agent": root.agent.map(AgentKind::id),
                "detail": root.detail,
                "discovery_complete": root.discovery_complete,
            }),
            observed_at,
        )?;
    }

    let mut skill_ids = std::collections::BTreeMap::new();
    let mut skill_names = std::collections::BTreeMap::new();
    for skill in &scan.skills {
        let id = SkillId::parse(skill.id.clone())?;
        let canonical_path = scan
            .placements
            .iter()
            .find(|placement| placement.skill_id == skill.id)
            .map(|placement| placement.directory.to_string_lossy().into_owned());
        let declared_revision = skill
            .metadata
            .version
            .clone()
            .or_else(|| skill.metadata.revision.clone());
        let identity_key = persistence_identity_key(skill, declared_revision.as_deref());
        let stored_id = store.save_skill(&SkillRecord {
            id,
            identity_key,
            name: skill.name.clone(),
            description: skill.metadata.description.clone(),
            declared_source: skill.metadata.source.clone(),
            declared_revision: declared_revision.clone(),
            content_digest: skill.content_digest.clone(),
            digest_version: 1,
            governance_state: GovernanceState::Observed,
            canonical_path,
        })?;
        if let (Some(source), Some(revision)) =
            (skill.metadata.source.as_ref(), declared_revision.as_ref())
        {
            if let Some(placement) = scan
                .placements
                .iter()
                .find(|placement| placement.skill_id == skill.id)
            {
                let entrypoint_digest = content_digest(&std::fs::read(&placement.entrypoint)?);
                store.record_source_baseline(&crate::sqlite::SourceBaseline {
                    source: source.clone(),
                    revision: revision.clone(),
                    entrypoint_digest,
                    first_observed_scan_id: scan_id.clone(),
                    first_observed_at: observed_at,
                    trusted_digest: None,
                    trusted_by_receipt_id: None,
                })?;
            }
        }
        store.index_skill(
            &stored_id,
            &skill.name,
            skill.metadata.description.as_deref().unwrap_or_default(),
            &skill.metadata.triggers.join(" "),
            &skill.normalized_text,
        )?;
        skill_ids.insert(skill.id.clone(), stored_id.clone());
        skill_names.insert(skill.id.clone(), skill.name.clone());
        save_reference_evidence(
            store,
            scan_id,
            &format!("digest:{}", skill.content_digest),
            EvidenceKind::Digest,
            EvidenceQuality::Observed,
            "skill",
            stored_id.as_str(),
            None,
            Some(skill.content_digest.clone()),
            json!({"algorithm": skill.digest_algorithm, "name": skill.name}),
            observed_at,
        )?;
        if let Some(content_identity_digest) = skill.content_identity_digest.as_deref() {
            save_reference_evidence(
                store,
                scan_id,
                &format!("routing_content_digest:{content_identity_digest}"),
                EvidenceKind::Digest,
                EvidenceQuality::Observed,
                "skill",
                stored_id.as_str(),
                None,
                Some(content_identity_digest.to_owned()),
                json!({
                    "algorithm": scan::CONTENT_IDENTITY_ALGORITHM,
                    "name": skill.name,
                    "scope": "routing_content_identity"
                }),
                observed_at,
            )?;
        }
    }

    for placement in &scan.placements {
        let path = placement.entrypoint.to_string_lossy().into_owned();
        let root_path = placement.root.to_string_lossy().into_owned();
        let root_key = (placement.agent, root_path.clone());
        let root_id = root_ids
            .get(&root_key)
            .cloned()
            .ok_or_else(|| anyhow!("Placement root was not normalized: {root_path}"))?;
        let skill_id = skill_ids
            .get(&placement.skill_id)
            .cloned()
            .ok_or_else(|| anyhow!("Placement Skill was not normalized: {}", placement.skill_id))?;
        let placement_id = PlacementId::parse(placement.id.clone())?;
        store.save_placement(&PlacementRecord {
            id: placement_id.clone(),
            scan_id: scan_id.clone(),
            skill_id: skill_id.clone(),
            agent_id: placement
                .agent
                .and_then(|agent| agent_ids.get(&agent).cloned()),
            root_id,
            path: path.clone(),
            kind: if placement.link_target.is_some() {
                PlacementKind::Symlink
            } else {
                PlacementKind::Directory
            },
            symlink_target: placement
                .link_target
                .as_ref()
                .map(|target| target.to_string_lossy().into_owned()),
            fingerprint: placement.content_digest.clone(),
            exposed: placement.default_exposed,
        })?;
        save_reference_evidence(
            store,
            scan_id,
            &format!("path:{path}"),
            EvidenceKind::Path,
            EvidenceQuality::Observed,
            "placement",
            placement_id.as_str(),
            Some(path),
            Some(placement.content_digest.clone()),
            json!({
                "skill_id": skill_id,
                "agent": placement.agent.map(AgentKind::id),
                "root_id": root_ids[&root_key],
                "link_status": placement.link_status,
                "link_target": placement.link_target,
                "default_exposed": placement.default_exposed,
                "governable": placement.is_mutable(),
                "owned_by_agent": placement.owned_by_agent,
                "mutation_scope": placement.mutation_scope,
                "provider": placement.provider,
                "fingerprint_completeness": placement.fingerprint_completeness,
                "fingerprint_detail": placement.fingerprint_detail,
            }),
            observed_at,
        )?;
        save_reference_evidence(
            store,
            scan_id,
            &format!("digest:{}", placement.content_digest),
            EvidenceKind::Digest,
            EvidenceQuality::Observed,
            "placement",
            placement_id.as_str(),
            Some(placement.entrypoint.to_string_lossy().into_owned()),
            Some(placement.content_digest.clone()),
            json!({
                "algorithm": "sha256-v1",
                "skill_id": skill_id,
                "placement_id": placement_id,
                "completeness": placement.fingerprint_completeness,
                "detail": placement.fingerprint_detail,
            }),
            observed_at,
        )?;
    }

    for coverage in &scan.coverage {
        let agent_id = agent_ids[&coverage.agent].clone();
        save_reference_evidence(
            store,
            scan_id,
            &format!("coverage:{}", coverage.agent.id()),
            EvidenceKind::Coverage,
            if coverage.denominator_reliable {
                EvidenceQuality::Observed
            } else {
                EvidenceQuality::Unknown
            },
            "agent",
            agent_id.as_str(),
            None,
            None,
            serde_json::to_value(coverage)?,
            observed_at,
        )?;
    }

    for usage in &scan.usage {
        let Some(skill_id) = skill_ids.get(&usage.skill_id).cloned() else {
            continue;
        };
        let Some(skill_name) = skill_names.get(&usage.skill_id) else {
            continue;
        };
        let agent_id = agent_ids[&usage.agent].clone();
        let reference = usage.evidence_reference();
        let evidence_id = save_reference_evidence(
            store,
            scan_id,
            &reference,
            EvidenceKind::Usage,
            evidence_quality(usage.quality),
            "skill",
            skill_id.as_str(),
            None,
            Some(usage.source_path_digest.clone()),
            json!({
                "agent": usage.agent.id(),
                "skill_id": skill_id,
                "skill_name": skill_name,
                "stage": usage.stage,
                "quality": usage.quality,
                "event_count": usage.event_count,
                "first_seen_unix": usage.first_seen_unix,
                "last_seen_unix": usage.last_seen_unix,
                "month_start_unix": usage.month_start_unix,
                "source_path_digest": usage.source_path_digest,
            }),
            observed_at,
        )?;
        store.save_usage_event(&UsageEvent {
            evidence_id,
            skill_id,
            agent_id,
            stage: usage_stage(usage.stage),
            quality: evidence_quality(usage.quality),
            source_path_digest: usage.source_path_digest.clone(),
            observed_event_count: usage.event_count,
            occurred_at: usage
                .last_seen_unix
                .and_then(|value| i64::try_from(value).ok()),
            outcome: None,
        })?;
    }
    Ok(())
}

fn persistence_identity_key(skill: &scan::ScannedSkill, declared_revision: Option<&str>) -> String {
    match (
        skill.metadata.source.as_deref(),
        declared_revision,
        skill.content_identity_digest.as_deref(),
    ) {
        (Some(source), Some(revision), _) => format!("source:{source}@{revision}"),
        (_, _, Some(content_identity)) => format!("content:{content_identity}"),
        _ => format!("incomplete-snapshot-skill:{}", skill.id),
    }
}

#[allow(clippy::too_many_arguments)]
fn save_reference_evidence(
    store: &StateStore,
    scan_id: &ScanId,
    reference: &str,
    kind: EvidenceKind,
    quality: EvidenceQuality,
    subject_type: &str,
    subject_id: &str,
    path: Option<String>,
    digest: Option<String>,
    details: Value,
    observed_at: i64,
) -> Result<EvidenceId> {
    let id = evidence_id(scan_id, reference)?;
    store.save_evidence(&EvidenceRecord {
        id: id.clone(),
        scan_id: scan_id.clone(),
        kind,
        quality,
        subject_type: subject_type.into(),
        subject_id: subject_id.into(),
        path,
        digest,
        details,
        observed_at,
    })?;
    Ok(id)
}

fn evidence_id(scan_id: &ScanId, reference: &str) -> Result<EvidenceId> {
    stable_id(
        "evidence",
        &format!("{}\0{reference}", scan_id.as_str()),
        EvidenceId::parse,
    )
}

fn stable_id<T>(
    prefix: &str,
    basis: &str,
    parse: impl FnOnce(String) -> std::result::Result<T, crate::model::InvalidId>,
) -> Result<T> {
    parse(format!("{prefix}_{:x}", Sha256::digest(basis.as_bytes()))).map_err(Into::into)
}

fn validate_roster_changes(
    store: &StateStore,
    plan: &PreparedPlan,
    scan: &ScanResult,
) -> Result<()> {
    validate_governance_operation_mutation_scopes(plan, scan)?;
    let mut requested = std::collections::HashSet::new();
    for change in &plan.roster_changes {
        let agent = harness_agent(&change.agent)?;
        if store.agent_id(&model_agent(agent))?.is_none() {
            bail!("Agent {} is not indexed", change.agent);
        }
        let skill_id = SkillId::parse(change.skill_id.clone())?;
        if !scan.skills.iter().any(|skill| skill.id == change.skill_id) {
            bail!(
                "Skill {skill_id} is not present in Snapshot {}",
                plan.scan_id
            );
        }
        if !store.skill_exists(&skill_id)? {
            bail!("Skill {skill_id} is not present in the Library index");
        }
        let has_mutable_placement = scan
            .placements
            .iter()
            .any(|placement| placement.skill_id == change.skill_id && placement.is_mutable());
        let read_only_placement_remains_unchanged = change.state == "core"
            && scan.placements.iter().any(|placement| {
                placement.skill_id == change.skill_id
                    && placement.agent == Some(agent)
                    && placement.default_exposed
            });
        if !has_mutable_placement && !read_only_placement_remains_unchanged {
            let scopes = scan
                .placements
                .iter()
                .filter(|placement| placement.skill_id == change.skill_id)
                .map(|placement| {
                    placement
                        .mutation_scope
                        .map(scan::MutationScope::id)
                        .unwrap_or("unknown")
                })
                .collect::<BTreeSet<_>>();
            bail!("Skill {skill_id} has no mutable placement; read-only mutation_scope {scopes:?}");
        }
        if !requested.insert((change.agent.as_str(), change.skill_id.as_str())) {
            bail!(
                "Plan contains conflicting duplicate Roster changes for Agent {} and Skill {skill_id}",
                change.agent
            );
        }
        roster_state(&change.state)?;
    }
    Ok(())
}

fn validate_governance_operation_mutation_scopes(
    plan: &PreparedPlan,
    scan: &ScanResult,
) -> Result<()> {
    let has_governance_changes = !plan.roster_changes.is_empty()
        || !plan.source_updates.is_empty()
        || !plan.library_changes.is_empty();
    if !has_governance_changes || plan.operations.is_empty() {
        return Ok(());
    }
    for operation in &plan.operations {
        let mutation_paths = match operation {
            Operation::MoveRecoverable { source, target, .. } => {
                vec![source.as_path(), target.as_path()]
            }
            Operation::CreateSymlink { target, .. }
            | Operation::WriteFile { target, .. }
            | Operation::ReplaceFile { target, .. }
            | Operation::RemoveSymlink { target, .. }
            | Operation::Copy { target, .. } => vec![target.as_path()],
            Operation::CreateDirectory { .. } => Vec::new(),
        };
        for mutation_path in mutation_paths {
            let mut blocked_by_skill = BTreeMap::<&str, Vec<&scan::SkillPlacement>>::new();
            for placement in scan.placements.iter().filter(|placement| {
                !placement.is_mutable()
                    && (paths_overlap(mutation_path, &placement.directory)
                        || placement
                            .physical_directory
                            .as_ref()
                            .is_some_and(|physical| paths_overlap(mutation_path, physical)))
            }) {
                blocked_by_skill
                    .entry(&placement.skill_id)
                    .or_default()
                    .push(placement);
            }
            if let Some((skill_id, mut placements)) = blocked_by_skill.into_iter().next() {
                placements.sort_by(|left, right| left.id.cmp(&right.id));
                return Err(crate::roster_plan::read_only_blocker(skill_id, &placements).into());
            }
        }
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn validate_governance_fingerprint_completeness(
    plan: &PreparedPlan,
    scan: &ScanResult,
) -> Result<()> {
    if plan.operations.is_empty() {
        return Ok(());
    }
    let governed_skill_ids = plan
        .roster_changes
        .iter()
        .map(|change| change.skill_id.as_str())
        .chain(
            plan.library_changes
                .iter()
                .map(|change| change.skill_id.as_str()),
        )
        .collect::<BTreeSet<_>>();
    if governed_skill_ids.is_empty() {
        return Ok(());
    }
    if let Some(blocker) = incomplete_fingerprint_blocker(
        scan.placements
            .iter()
            .filter(|placement| governed_skill_ids.contains(placement.skill_id.as_str())),
        "apply",
    ) {
        return Err(blocker.into());
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct PhysicalPlacementBinding {
    placement_id: String,
    entrypoint: PathBuf,
    expected_physical_directory: PathBuf,
}

fn capture_physical_placement_bindings(
    plan: &PreparedPlan,
    scan: &ScanResult,
) -> Vec<PhysicalPlacementBinding> {
    let skill_ids = plan
        .roster_changes
        .iter()
        .map(|change| change.skill_id.as_str())
        .chain(
            plan.library_changes
                .iter()
                .map(|change| change.skill_id.as_str()),
        )
        .collect::<BTreeSet<_>>();
    let mut bindings = scan
        .placements
        .iter()
        .filter(|placement| skill_ids.contains(placement.skill_id.as_str()))
        .filter_map(|placement| {
            Some(PhysicalPlacementBinding {
                placement_id: placement.id.clone(),
                entrypoint: placement.entrypoint.clone(),
                expected_physical_directory: placement.physical_directory.clone()?,
            })
        })
        .collect::<Vec<_>>();
    bindings.sort_by(|left, right| left.placement_id.cmp(&right.placement_id));
    bindings
}

fn validate_physical_placement_bindings(
    record: &PlanRecord,
    plan: &PreparedPlan,
    scan: &ScanResult,
) -> Result<()> {
    let expected = capture_physical_placement_bindings(plan, scan);
    let Some(stored) = record.input.get("physical_placement_bindings").cloned() else {
        if expected.is_empty() {
            return Ok(());
        }
        bail!("Plan {} has no physical placement bindings", record.id);
    };
    let stored: Vec<PhysicalPlacementBinding> = serde_json::from_value(stored)?;
    if stored != expected {
        bail!(
            "Plan {} physical placement bindings are incomplete or stale",
            record.id
        );
    }
    for binding in stored {
        let placement = scan
            .placements
            .iter()
            .find(|placement| placement.id == binding.placement_id)
            .ok_or_else(|| anyhow!("Plan placement {} disappeared", binding.placement_id))?;
        if placement.entrypoint != binding.entrypoint
            || placement.physical_directory.as_ref() != Some(&binding.expected_physical_directory)
        {
            bail!(
                "Plan placement {} no longer matches its physical binding",
                binding.placement_id
            );
        }
        placement.validated_physical_directory()?;
    }
    Ok(())
}

fn validate_source_update_preconditions(plan: &PreparedPlan, scan: &ScanResult) -> Result<()> {
    for update in &plan.source_updates {
        let placement = scan
            .placements
            .iter()
            .find(|placement| placement.id == update.placement_id)
            .ok_or_else(|| {
                anyhow!(
                    "Source-update placement {} disappeared",
                    update.placement_id
                )
            })?;
        if placement.skill_id != update.skill_id
            || placement.content_digest != update.current_digest
            || placement.entrypoint != update.target
        {
            bail!(
                "Source-update preconditions drifted for {}",
                update.placement_id
            );
        }
        let actual = change::fingerprint(&update.target)?;
        if actual != update.expected_file_fingerprint {
            bail!("Source-update file drifted for {}", update.placement_id);
        }
    }
    Ok(())
}

fn validate_plan_evidence(store: &StateStore, plan: &PreparedPlan, scan_id: &ScanId) -> Result<()> {
    let mut unique = std::collections::HashSet::new();
    for raw_id in &plan.evidence_ids {
        let evidence_id = EvidenceId::parse(raw_id.clone())?;
        if !unique.insert(evidence_id.clone()) {
            bail!("Plan contains duplicate Evidence ID {evidence_id}");
        }
        if !store.evidence_belongs_to_scan(&evidence_id, scan_id)? {
            bail!("Evidence {evidence_id} does not belong to Snapshot {scan_id}");
        }
    }
    Ok(())
}

fn capture_roster_state(store: &StateStore, plan: &PreparedPlan) -> Result<Vec<Value>> {
    plan.roster_changes
        .iter()
        .map(|change| {
            let agent = harness_agent(&change.agent)?;
            let agent_id = store
                .agent_id(&model_agent(agent))?
                .ok_or_else(|| anyhow!("Agent {} is not indexed", change.agent))?;
            let skill_id = SkillId::parse(change.skill_id.clone())?;
            let existing = store.roster_entry(&agent_id, &skill_id)?;
            Ok(json!({
                "agent_id": agent_id,
                "skill_id": skill_id,
                "state": existing.map(|entry| entry.state)
            }))
        })
        .collect()
}

fn apply_roster_changes(store: &StateStore, plan: &PreparedPlan) -> Result<()> {
    for change in &plan.roster_changes {
        let agent_id = store
            .agent_id(&model_agent(harness_agent(&change.agent)?))?
            .ok_or_else(|| anyhow!("Agent {} is not indexed", change.agent))?;
        store.save_roster_entry(&RosterEntry {
            agent_id,
            skill_id: SkillId::parse(change.skill_id.clone())?,
            state: roster_state(&change.state)?,
            updated_at: Utc::now().timestamp(),
        })?;
    }
    Ok(())
}

fn restore_roster_state(store: &StateStore, before: &Value) -> Result<()> {
    for entry in before
        .as_array()
        .ok_or_else(|| anyhow!("Receipt Roster state is invalid"))?
    {
        let agent_id = AgentId::parse(
            entry["agent_id"]
                .as_str()
                .ok_or_else(|| anyhow!("Receipt Agent ID is missing"))?
                .to_string(),
        )?;
        let skill_id = SkillId::parse(
            entry["skill_id"]
                .as_str()
                .ok_or_else(|| anyhow!("Receipt Skill ID is missing"))?
                .to_string(),
        )?;
        if entry["state"].is_null() {
            store.delete_roster_entry(&agent_id, &skill_id)?;
        } else {
            let state: RosterState = serde_json::from_value(entry["state"].clone())?;
            store.save_roster_entry(&RosterEntry {
                agent_id,
                skill_id,
                state,
                updated_at: Utc::now().timestamp(),
            })?;
        }
    }
    Ok(())
}

fn capture_library_state(store: &StateStore, plan: &PreparedPlan) -> Result<Vec<Value>> {
    plan.library_changes
        .iter()
        .map(|change| {
            let skill_id = SkillId::parse(change.skill_id.clone())?;
            let state = store
                .skill_governance_state(&skill_id)?
                .ok_or_else(|| anyhow!("Skill {skill_id} is not indexed"))?;
            Ok(json!({"skill_id": skill_id, "state": state}))
        })
        .collect()
}

fn apply_library_changes(store: &StateStore, plan: &PreparedPlan) -> Result<()> {
    for change in &plan.library_changes {
        let state = match change.requested_state.as_str() {
            "managed" => GovernanceState::Managed,
            "hosted" => GovernanceState::Hosted,
            value => bail!("unsupported governance state: {value}"),
        };
        store.update_skill_governance_state(&SkillId::parse(change.skill_id.clone())?, state)?;
    }
    Ok(())
}

fn restore_library_state(store: &StateStore, before: &Value) -> Result<()> {
    for entry in before
        .as_array()
        .ok_or_else(|| anyhow!("Receipt Library state is invalid"))?
    {
        let skill_id = SkillId::parse(
            entry["skill_id"]
                .as_str()
                .ok_or_else(|| anyhow!("Receipt Skill ID is missing"))?
                .to_string(),
        )?;
        let state: GovernanceState = serde_json::from_value(entry["state"].clone())?;
        store.update_skill_governance_state(&skill_id, state)?;
    }
    Ok(())
}

fn latest_scan(store: &StateStore) -> Result<(ScanId, ScanResult)> {
    store
        .latest_scan_payload()?
        .ok_or_else(|| anyhow!("no completed Snapshot; run skillroster scan first"))
}

#[derive(Debug)]
struct ContentIdentityRescanRequired;

impl std::fmt::Display for ContentIdentityRescanRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "latest Snapshot has no {} content identity; run skillroster scan",
            scan::CONTENT_IDENTITY_ALGORITHM
        )
    }
}

impl std::error::Error for ContentIdentityRescanRequired {}

fn require_content_identity(scan: &ScanResult) -> Result<()> {
    if scan.content_identity_algorithm.as_deref() != Some(scan::CONTENT_IDENTITY_ALGORITHM) {
        return Err(ContentIdentityRescanRequired.into());
    }
    Ok(())
}

fn harness_agent(value: &str) -> Result<AgentKind> {
    AgentKind::ALL
        .into_iter()
        .find(|agent| agent.id() == value)
        .ok_or_else(|| anyhow!("unsupported Agent in Roster change: {value}"))
}

fn model_agent(value: AgentKind) -> crate::model::AgentKind {
    match value {
        AgentKind::Codex => crate::model::AgentKind::Codex,
        AgentKind::ClaudeCode => crate::model::AgentKind::ClaudeCode,
        AgentKind::Pi => crate::model::AgentKind::Pi,
        AgentKind::OpenCode => crate::model::AgentKind::OpenCode,
        AgentKind::Hermes => crate::model::AgentKind::Hermes,
        AgentKind::Cursor => crate::model::AgentKind::Cursor,
        AgentKind::GeminiCli => crate::model::AgentKind::GeminiCli,
        AgentKind::GitHubCopilot => crate::model::AgentKind::GithubCopilot,
    }
}

fn root_kind(value: RootKind) -> &'static str {
    match value {
        RootKind::Skills => "skills",
        RootKind::Sessions => "sessions",
    }
}

fn root_status(value: crate::scan::RootStatus) -> RootStatus {
    match value {
        crate::scan::RootStatus::Included => RootStatus::Included,
        crate::scan::RootStatus::Excluded => RootStatus::Excluded,
        crate::scan::RootStatus::Missing => RootStatus::Missing,
        crate::scan::RootStatus::Inaccessible => RootStatus::Inaccessible,
    }
}

fn evidence_quality(value: crate::scan::EvidenceQuality) -> EvidenceQuality {
    match value {
        crate::scan::EvidenceQuality::Observed => EvidenceQuality::Observed,
        crate::scan::EvidenceQuality::Inferred => EvidenceQuality::Inferred,
        crate::scan::EvidenceQuality::Unknown => EvidenceQuality::Unknown,
    }
}

fn usage_stage(value: crate::scan::UsageStage) -> UsageStage {
    match value {
        crate::scan::UsageStage::Exposed => UsageStage::Exposed,
        crate::scan::UsageStage::Matched => UsageStage::Matched,
        crate::scan::UsageStage::Loaded => UsageStage::Loaded,
        crate::scan::UsageStage::Applied => UsageStage::Applied,
        crate::scan::UsageStage::Outcome => UsageStage::Outcome,
    }
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

#[allow(clippy::too_many_arguments)]
fn plan_record(
    prepared: &PreparedPlan,
    raw: Value,
    roster_before: Vec<Value>,
    library_before: Vec<Value>,
    report_id: Option<ReportId>,
    finding_ids: Vec<FindingId>,
    summary: Value,
    selection_evidence_full: Option<Value>,
    reuse_identity: Option<Value>,
    physical_bindings: Vec<PhysicalPlacementBinding>,
) -> Result<PlanRecord> {
    let operations = prepared
        .operations
        .iter()
        .enumerate()
        .map(|(position, operation)| PlanOperation {
            id: OperationId::new(),
            position: position as u32,
            target_path: operation.target().to_string_lossy().into_owned(),
            expected_fingerprint: Some(expected_fingerprint(operation).into()),
            action: match operation {
                Operation::CreateDirectory { .. } => OperationAction::CreateDirectory,
                Operation::CreateSymlink {
                    source,
                    expected_source_fingerprint,
                    ..
                } => OperationAction::CreateSymlink {
                    source: source.to_string_lossy().into_owned(),
                    expected_source_fingerprint: expected_source_fingerprint.clone(),
                },
                Operation::WriteFile { content, .. } => OperationAction::WriteFile {
                    content: content.clone(),
                },
                Operation::ReplaceFile { content, .. } => OperationAction::ReplaceFile {
                    content: content.clone(),
                },
                Operation::RemoveSymlink { .. } => OperationAction::RemoveSymlink,
                Operation::Copy { source, .. } => OperationAction::Copy {
                    source: source.to_string_lossy().into_owned(),
                },
                Operation::MoveRecoverable { source, .. } => OperationAction::MoveRecoverable {
                    source: source.to_string_lossy().into_owned(),
                },
            },
        })
        .collect();
    Ok(PlanRecord {
        id: PlanId::parse(prepared.id.clone())?,
        scan_id: ScanId::parse(prepared.scan_id.clone())?,
        report_id,
        created_at: Utc::now().timestamp(),
        status: PlanStatus::Ready,
        input: json!({
            "raw": raw,
            "prepared": prepared,
            "finding_ids": finding_ids,
            "roster_before": roster_before,
            "library_before": library_before,
            "summary": summary,
            "selection_evidence_full": selection_evidence_full,
            "reuse_identity": reuse_identity,
            "physical_placement_bindings": physical_bindings
        }),
        fingerprint: prepared.digest.clone(),
        operations,
    })
}

#[allow(clippy::too_many_arguments)]
fn receipt_record(
    receipt: &ChangeReceipt,
    operation_ids: &[(u32, OperationId)],
    reverses: Option<ReceiptId>,
    verified: bool,
    roster_state: Value,
    source_updates: Value,
    evidence_ids: Value,
    library_state: Value,
) -> Result<ReceiptRecord> {
    Ok(ReceiptRecord {
        id: ReceiptId::parse(receipt.id.clone())?,
        plan_id: PlanId::parse(receipt.plan_id.clone())?,
        reverses_receipt_id: reverses,
        created_at: Utc::now().timestamp(),
        completed_at: Some(Utc::now().timestamp()),
        status: match receipt.status {
            change::ReceiptStatus::Applying => ReceiptStatus::Applying,
            change::ReceiptStatus::Applied => ReceiptStatus::Applied,
            change::ReceiptStatus::FailedRolledBack => ReceiptStatus::FailedRolledBack,
            change::ReceiptStatus::RecoveryRequired => ReceiptStatus::RecoveryRequired,
            change::ReceiptStatus::Undone => ReceiptStatus::Undone,
        },
        verification: json!({
            "passed": verified,
            "change_receipt": receipt,
            "roster_state": roster_state,
            "source_updates": source_updates,
            "evidence_ids": evidence_ids,
            "library_state": library_state
        }),
        operation_results: receipt
            .operation_results
            .iter()
            .map(|result| {
                let operation_id = operation_ids
                    .iter()
                    .find(|(position, _)| *position == result.position)
                    .map(|(_, id)| id.clone())
                    .ok_or_else(|| {
                        anyhow!(
                            "Receipt operation {} has no matching Plan operation",
                            result.position
                        )
                    })?;
                Ok(OperationResult {
                    operation_id,
                    position: result.position,
                    status: result.status.clone(),
                    before_state: json!({
                        "action": result.action,
                        "target": result.target,
                        "fingerprint": result.before_fingerprint,
                    }),
                    after_state: result.after_fingerprint.as_ref().map(|fingerprint| {
                        json!({
                            "target": result.target,
                            "fingerprint": fingerprint,
                        })
                    }),
                    error: result.error.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

fn mutation_result(
    outcome: &change::ApplyOutcome,
    receipt: &ReceiptRecord,
    reverses: Option<ReceiptId>,
) -> Value {
    json!({
        "plan_id": receipt.plan_id,
        "receipt_id": receipt.id,
        "reverses_receipt_id": reverses,
        "status": receipt.status,
        "changed_path_count": outcome.receipt.changed_paths.len(),
        "changed_paths": outcome.receipt.changed_paths,
        "verification": if outcome.verification_passed { "passed" } else { "failed" },
        "canonical_deletion_count": 0,
        "undo_available": outcome.receipt.status == change::ReceiptStatus::Applied,
        "error": outcome.receipt.error,
        "files_changed": !outcome.receipt.changed_paths.is_empty()
    })
}

fn approved_roots(scan: &ScanResult) -> Vec<PathBuf> {
    scan.roots
        .iter()
        .filter(|root| root.kind == RootKind::Skills && root.status == scan::RootStatus::Included)
        .map(|root| root.path.clone())
        .collect()
}

fn expected_fingerprint(operation: &Operation) -> &str {
    match operation {
        Operation::CreateDirectory {
            expected_fingerprint,
            ..
        }
        | Operation::CreateSymlink {
            expected_fingerprint,
            ..
        }
        | Operation::WriteFile {
            expected_fingerprint,
            ..
        }
        | Operation::ReplaceFile {
            expected_fingerprint,
            ..
        }
        | Operation::RemoveSymlink {
            expected_fingerprint,
            ..
        }
        | Operation::Copy {
            expected_fingerprint,
            ..
        }
        | Operation::MoveRecoverable {
            expected_fingerprint,
            ..
        } => expected_fingerprint,
    }
}

fn journal_issues(store: &StateStore, state_dir: &Path) -> Result<Vec<Value>> {
    let mut issues = Vec::new();
    for journal in change::journals(state_dir)? {
        let id = ReceiptId::parse(journal.id.clone())?;
        let tracked = store.receipt_exists(&id)?;
        let pending = matches!(
            journal.status,
            change::ReceiptStatus::Applying | change::ReceiptStatus::RecoveryRequired
        );
        if !tracked || pending {
            issues.push(json!({
                "receipt_id": journal.id,
                "plan_id": journal.plan_id,
                "journal_status": journal.status,
                "tracked_in_sqlite": tracked,
                "reverses_receipt_id": journal.reverses_receipt_id,
                "changed_paths": journal.changed_paths,
                "error": journal.error,
            }));
        }
    }
    Ok(issues)
}

fn require_clear_journals(store: &StateStore, state_dir: &Path) -> Result<()> {
    let issues = journal_issues(store, state_dir)?;
    if let Some(issue) = issues.first() {
        bail!(
            "filesystem journal {} requires recovery inspection before another write; run lifecycle recovery --json",
            issue["receipt_id"].as_str().unwrap_or("unknown")
        );
    }
    Ok(())
}

fn recovery_text(store: &StateStore, state_dir: &Path) -> Result<&'static str> {
    Ok(
        if store.recovery_required()? || !journal_issues(store, state_dir)?.is_empty() {
            "required"
        } else {
            "clear"
        },
    )
}

fn action(
    name: &str,
    argv: &[&str],
    mutates: bool,
    confirmation: bool,
    reason: &str,
) -> SuggestedAction {
    SuggestedAction {
        action: name.into(),
        description: name.into(),
        argv: std::iter::once("skillroster".into())
            .chain(argv.iter().map(|value| (*value).into()))
            .collect(),
        mutates,
        requires_confirmation: confirmation,
        reason_code: reason.into(),
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod recovery_tests {
    use super::*;
    use clap::Parser;
    use tempfile::TempDir;

    #[test]
    fn command_lock_mode_matches_state_production_boundary() {
        fn exclusive(args: &[&str]) -> bool {
            let cli =
                Cli::try_parse_from(std::iter::once("skillroster").chain(args.iter().copied()))
                    .unwrap();
            command_requires_exclusive_state_lock(cli.command.as_ref())
        }

        for args in [
            &["scan"][..],
            &["report"][..],
            &["plan"][..],
            &["apply", "plan_1"][..],
            &["undo", "receipt_1"][..],
            &["setup"][..],
            &[
                "source-root",
                "confirm",
                "--finding",
                "finding_1",
                "--path",
                "/tmp",
            ][..],
            &["source-root", "revoke", "permission_1"][..],
            &["lifecycle", "exclude", "codex"][..],
            &["lifecycle", "purge", "--raw-days", "180"][..],
            &["lifecycle", "recovery"][..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"][..],
        ] {
            assert!(exclusive(args), "expected exclusive lock for {args:?}");
        }
        for args in [
            &[][..],
            &["status"][..],
            &["find", "review a pull request"][..],
            &["report", "--finding", "finding_1"][..],
            &["plan", "--show", "plan_1"][..],
            &["source-root", "inspect"][..],
            &["lifecycle", "inspect"][..],
            &["lifecycle", "export", "--output", "/tmp/export.json"][..],
        ] {
            assert!(!exclusive(args), "expected shared lock for {args:?}");
        }
    }

    fn placement_with_scope(
        id: &str,
        skill_id: &str,
        directory: &str,
        mutation_scope: Option<scan::MutationScope>,
    ) -> scan::SkillPlacement {
        let directory = PathBuf::from(directory);
        scan::SkillPlacement {
            id: id.into(),
            skill_id: skill_id.into(),
            agent: Some(AgentKind::Codex),
            root: PathBuf::from("/fixture/root"),
            entrypoint: directory.join("SKILL.md"),
            directory,
            physical_directory: None,
            content_digest: "legacy-digest".into(),
            entrypoint_digest: None,
            fingerprint_completeness: scan::FingerprintCompleteness::Complete,
            fingerprint_detail: None,
            link_target: None,
            link_status: scan::LinkStatus::NotLink,
            default_exposed: true,
            owned_by_agent: mutation_scope.map(|_| true),
            mutation_scope,
            governable: mutation_scope == Some(scan::MutationScope::Mutable),
            provider: None,
            executable_files: vec![],
            declared_name_matches_directory: Some(true),
        }
    }

    #[test]
    fn scan_detected_source_root_drift_cannot_be_promoted_back_to_active() {
        let permission = crate::source_policy::SourceRootPermission {
            id: crate::source_policy::SourcePermissionId::parse("sroot_fixture").unwrap(),
            path: PathBuf::from("/fixture/source"),
            finding_id: "finding_fixture".into(),
            snapshot_id: "scan_fixture".into(),
            identity: crate::source_policy::RootIdentity::Unavailable,
            granted_at: 1,
            revoked_at: None,
        };
        let active = crate::source_policy::FrozenSourceRoot {
            permission,
            state: crate::source_policy::SourceRootState::Active,
            resolved_path: Some(PathBuf::from("/fixture/source")),
            drift_reason: None,
        };
        let facts = conservative_source_policy_facts(
            std::slice::from_ref(&active),
            std::slice::from_ref(&active),
            &BTreeSet::from(["sroot_fixture".into()]),
        );

        assert_eq!(
            facts[0].state,
            crate::source_policy::SourceRootState::Inaccessible
        );
        assert!(facts[0].resolved_path.is_none());
        assert!(
            facts[0]
                .drift_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("during bounded Scan"))
        );
    }

    fn coverage_finding(basis: crate::query::FindingCoverageBasis) -> crate::query::Finding {
        crate::query::Finding {
            id: "finding_coverage".into(),
            category: crate::query::FindingCategory::Overlap,
            severity: crate::query::Severity::Medium,
            title: "Exact duplicate Skill placements".into(),
            summary: String::new(),
            affected_skill_ids: vec!["skill_one".into()],
            affected_placement_ids: vec!["placement_one".into()],
            evidence: Vec::new(),
            evidence_quality: scan::EvidenceQuality::Observed,
            coverage_basis: basis,
        }
    }

    #[test]
    fn finding_coverage_keeps_skill_roots_separate_from_sessions() {
        let root = |agent, status, suffix: &str| scan::RootObservation {
            agent,
            kind: RootKind::Skills,
            path: PathBuf::from(format!("/fixture/{suffix}")),
            status,
            explicit: false,
            detail: None,
            discovery_complete: true,
        };
        let session_root = |agent, status, suffix: &str| scan::RootObservation {
            agent: Some(agent),
            kind: RootKind::Sessions,
            path: PathBuf::from(format!("/fixture/{suffix}")),
            status,
            explicit: false,
            detail: None,
            discovery_complete: true,
        };
        let session =
            |agent, roots_present, roots_missing, roots_inaccessible| scan::SessionCoverage {
                agent,
                roots_present,
                roots_missing,
                roots_inaccessible,
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
        let mut scan = ScanResult {
            roots: vec![
                root(Some(AgentKind::Codex), scan::RootStatus::Included, "codex"),
                root(Some(AgentKind::Cursor), scan::RootStatus::Missing, "cursor"),
                session_root(
                    AgentKind::Codex,
                    scan::RootStatus::Included,
                    "codex-sessions",
                ),
                session_root(
                    AgentKind::Cursor,
                    scan::RootStatus::Missing,
                    "cursor-sessions",
                ),
            ],
            coverage: vec![
                session(AgentKind::Codex, 1, 0, 0),
                session(AgentKind::Cursor, 0, 1, 0),
            ],
            ..ScanResult::default()
        };

        let structural = finding_coverage(
            &coverage_finding(crate::query::FindingCoverageBasis::SkillRootScan),
            &scan,
        );
        assert_eq!(structural["basis"], "skill_root_scan");
        assert_eq!(structural["denominator_reliable"], true);
        assert_eq!(structural["missing_root_count"], 1);
        assert_eq!(structural["limited_agents"], json!([]));

        scan.roots[0].discovery_complete = false;
        let bounded_structural = finding_coverage(
            &coverage_finding(crate::query::FindingCoverageBasis::SkillRootScan),
            &scan,
        );
        assert_eq!(bounded_structural["denominator_reliable"], false);
        assert_eq!(bounded_structural["bounded_root_count"], 1);
        assert_eq!(bounded_structural["limited_agents"], json!(["codex"]));
        let blocker = crate::roster_plan::RosterDiscoveryIncomplete {
            agent: "codex".into(),
            path: PathBuf::from("/fixture/codex"),
            detail: Some("Skill discovery was bounded at depth 5".into()),
        };
        let blocked: Value = serde_json::from_str(&error_json("plan", &blocker)).unwrap();
        assert_eq!(
            blocked["error"]["code"],
            "roster_skill_root_discovery_incomplete"
        );
        assert_eq!(blocked["error"]["details"]["files_changed"], false);
        scan.roots[0].discovery_complete = true;

        let usage = finding_coverage(
            &coverage_finding(crate::query::FindingCoverageBasis::SessionUsage),
            &scan,
        );
        assert_eq!(usage["basis"], "session_usage");
        assert_eq!(usage["denominator_reliable"], false);
        assert_eq!(usage["limited_agents"], json!(["codex"]));
        assert!(
            usage["missing_agents"]
                .as_array()
                .unwrap()
                .contains(&json!("cursor"))
        );

        scan.roots.extend([
            session_root(
                AgentKind::ClaudeCode,
                scan::RootStatus::Inaccessible,
                "claude-sessions",
            ),
            session_root(AgentKind::Pi, scan::RootStatus::Excluded, "pi-sessions"),
        ]);
        let mixed_sessions = finding_coverage(
            &coverage_finding(crate::query::FindingCoverageBasis::SessionUsage),
            &scan,
        );
        assert_eq!(
            mixed_sessions["inaccessible_agents"],
            json!(["claude-code"])
        );
        assert_eq!(mixed_sessions["excluded_agents"], json!(["pi"]));
        let classified_agents = [
            "reliable_agents",
            "limited_agents",
            "missing_agents",
            "excluded_agents",
            "inaccessible_agents",
        ]
        .into_iter()
        .flat_map(|key| mixed_sessions[key].as_array().unwrap())
        .collect::<Vec<_>>();
        assert_eq!(classified_agents.len(), AgentKind::ALL.len());
        assert_eq!(
            classified_agents
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect::<BTreeSet<_>>()
                .len(),
            AgentKind::ALL.len()
        );

        scan.roots.push(root(
            Some(AgentKind::ClaudeCode),
            scan::RootStatus::Inaccessible,
            "claude",
        ));
        let inaccessible = finding_coverage(
            &coverage_finding(crate::query::FindingCoverageBasis::SkillRootScan),
            &scan,
        );
        assert_eq!(inaccessible["denominator_reliable"], false);
        assert_eq!(inaccessible["inaccessible_root_count"], 1);
        assert_eq!(inaccessible["limited_agents"], json!(["claude-code"]));
    }

    #[test]
    fn stored_coverage_basis_prefers_typed_details_and_only_infers_legacy_records() {
        let finding = FindingRecord {
            id: FindingId::parse("finding_coverage-basis").unwrap(),
            report_id: ReportId::parse("report_coverage-basis").unwrap(),
            category: FindingCategory::Usage,
            severity: Severity::Warning,
            title: "Five-stage usage evidence".into(),
            summary: String::new(),
            details: Value::Null,
            evidence_ids: Vec::new(),
        };

        let typed = json!({"coverage": {"basis": "skill_root_scan"}});
        assert_eq!(
            stored_finding_coverage_basis(&finding, typed.as_object().unwrap()).unwrap(),
            crate::query::FindingCoverageBasis::SkillRootScan
        );

        let legacy = json!({});
        assert_eq!(
            stored_finding_coverage_basis(&finding, legacy.as_object().unwrap()).unwrap(),
            crate::query::FindingCoverageBasis::SessionUsage
        );

        for (invalid, reason) in [
            (
                json!({"coverage": {"basis": "future_basis"}}),
                "unsupported_coverage_basis",
            ),
            (
                json!({"coverage": {"basis": 7}}),
                "unsupported_coverage_basis",
            ),
            (json!({"coverage": []}), "malformed_coverage"),
        ] {
            let error =
                stored_finding_coverage_basis(&finding, invalid.as_object().unwrap()).unwrap_err();
            let output: Value =
                serde_json::from_str(&error_json("report", error.as_ref())).unwrap();
            assert_eq!(output["error"]["code"], "stored_finding_coverage_invalid");
            assert_eq!(
                output["error"]["details"]["finding_id"],
                finding.id.to_string()
            );
            assert_eq!(output["error"]["details"]["reason"], reason);
            assert_eq!(output["error"]["details"]["files_changed"], false);
            assert_eq!(output["error"]["details"]["next_action"], "scan");
        }
    }

    #[test]
    fn summary_actions_keep_direct_drilldowns_when_every_finding_fits() {
        let result = json!({
            "finding_count": 2,
            "findings": [{"id": "finding_one"}, {"id": "finding_two"}]
        });
        let actions = report_actions(&result, ReportRequest::Summary);
        assert_eq!(actions.len(), 2);
        assert!(actions.iter().all(|action| action.action == "view_finding"));
        assert_eq!(
            actions[0].argv,
            [
                "skillroster",
                "report",
                "--finding",
                "finding_one",
                "--json"
            ]
        );
        assert_eq!(
            actions[1].argv,
            [
                "skillroster",
                "report",
                "--finding",
                "finding_two",
                "--json"
            ]
        );

        let empty = report_actions(
            &json!({"finding_count": 0, "findings": []}),
            ReportRequest::Summary,
        );
        assert!(empty.is_empty());
    }

    #[test]
    fn action_context_absolutizes_a_relative_state_directory() {
        let cli = Cli {
            json: true,
            state_dir: Some(PathBuf::from("relative-state")),
            home: None,
            roots: vec![],
            source_roots: vec![],
            command: Some(Command::Status),
        };

        let context = ActionContext::from_cli(&cli).unwrap();

        assert_eq!(context.argv[0], "--state-dir");
        assert_eq!(
            PathBuf::from(&context.argv[1]),
            std::path::absolute("relative-state").unwrap()
        );
    }

    #[test]
    #[cfg(unix)]
    fn action_context_refuses_a_non_unicode_path() {
        use std::os::unix::ffi::OsStringExt;

        let cli = Cli {
            json: true,
            state_dir: Some(PathBuf::from(std::ffi::OsString::from_vec(vec![
                b'/', b't', b'm', b'p', b'/', 0xff,
            ]))),
            home: None,
            roots: vec![],
            source_roots: vec![],
            command: Some(Command::Status),
        };

        let error = ActionContext::from_cli(&cli).unwrap_err();

        assert!(error.to_string().contains("must be valid Unicode"));
    }

    #[test]
    fn finding_rollups_count_unique_affected_subjects() {
        let findings = vec![
            json!({
                "category": "overlap",
                "severity": "medium",
                "title": "Exact duplicate Skill placements",
                "affected_skill_ids": ["skill_a"],
                "affected_placement_ids": ["placement_a", "placement_b"]
            }),
            json!({
                "category": "overlap",
                "severity": "medium",
                "title": "Exact duplicate Skill placements",
                "affected_skill_ids": ["skill_a", "skill_b"],
                "affected_placement_ids": ["placement_b", "placement_c"]
            }),
        ];

        let rollups = finding_rollups(&findings);

        assert_eq!(rollups.len(), 1);
        assert_eq!(rollups[0]["finding_count"], 2);
        assert_eq!(rollups[0]["affected_skill_count"], 2);
        assert_eq!(rollups[0]["affected_placement_count"], 3);
    }

    #[test]
    fn finding_pages_interleave_families_without_losing_instances() {
        let finding = |id: &str, category: &str, title: &str| {
            json!({
                "id": id,
                "category": category,
                "severity": "medium",
                "title": title,
                "affected_skill_ids": [],
                "affected_placement_ids": []
            })
        };
        let report = json!({
            "findings": [
                finding("a1", "overlap", "family-a"),
                finding("a2", "overlap", "family-a"),
                finding("a3", "overlap", "family-a"),
                finding("b1", "overlap", "family-b"),
                finding("b2", "overlap", "family-b"),
                finding("c1", "usage", "family-c")
            ]
        });

        let first = paged_finding_report(&report, None, None, 4, 0);
        let second = paged_finding_report(&report, None, None, 4, 4);
        let ids = |page: &Value| {
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };

        assert_eq!(ids(&first), ["a1", "b1", "c1", "a2"]);
        assert_eq!(ids(&second), ["b2", "a3"]);
        assert_eq!(first["page"]["total"], 6);
        assert_eq!(first["page"]["next_offset"], 4);
        assert_eq!(second["page"]["has_more"], false);
        assert_eq!(first, paged_finding_report(&report, None, None, 4, 0));

        let filtered = paged_finding_report(
            &report,
            Some(ReportCategory::Overlap),
            Some(ReportSeverity::Medium),
            10,
            0,
        );
        assert_eq!(ids(&filtered), ["a1", "b1", "a2", "b2", "a3"]);
        assert_eq!(filtered["page"]["total"], 5);
    }

    #[test]
    fn blocked_skill_planning_groups_pairs_and_expands_full_detail() {
        let exclusions = (0..6)
            .flat_map(|index| {
                ["codex", "claude-code"].map(move |agent| {
                    crate::roster_plan::RosterChangeExclusion {
                        agent: agent.into(),
                        skill_id: format!("skill_{index}"),
                        name: format!("skill-name-{index}"),
                        reason: "non_agent_source_link_depends_on_removal",
                        observed_source_target: Some(PathBuf::from(format!(
                            "/agent/skill-{index}"
                        ))),
                        safety_blocker: Some(
                            crate::roster_plan::RosterSafetyBlocker::DependentSource {
                                skill_id: format!("skill_{index}"),
                                placement_ids: vec![format!("placement_{index}")],
                                paths: vec![PathBuf::from(format!("/source/skill-{index}"))],
                            },
                        ),
                    }
                })
            })
            .collect::<Vec<_>>();

        let compact = blocked_skill_planning(&exclusions, false);
        assert_eq!(compact.count, 6);
        assert_eq!(compact.items.len(), 5);
        assert!(compact.truncated);
        assert_eq!(compact.items[0]["name"], "skill-name-0");
        assert_eq!(compact.items[0]["agents"], json!(["claude-code", "codex"]));
        assert_eq!(
            compact.items[0]["dependent_source_paths"],
            json!(["/source/skill-0"])
        );

        let full = blocked_skill_planning(&exclusions, true);
        assert_eq!(full.count, 6);
        assert_eq!(full.items.len(), 6);
        assert!(!full.truncated);
        assert_eq!(full.displayed_skill_ids.len(), 6);
    }

    #[test]
    fn roster_selection_uncertainty_requires_a_fallback_majority() {
        let recommendation = crate::roster_recommendation::RosterRecommendation {
            changes: vec![],
            agents: vec![crate::roster_recommendation::AgentRecommendation {
                agent: AgentKind::Codex,
                before_default_exposure: 51,
                unique_skill_count: 51,
                core_count: 3,
                on_demand_count: 48,
                positive_signal_count: 1,
                direct_signal_count: 1,
                cross_agent_signal_count: 0,
                fallback_core_count: 1,
                core_selections: vec![
                    crate::roster_recommendation::CoreSelection {
                        skill_id: "skill_forced".into(),
                        name: "forced".into(),
                        reason: "protected_by_request",
                        evidence_scope: "forced",
                        evidence_agents: vec![],
                    },
                    crate::roster_recommendation::CoreSelection {
                        skill_id: "skill_observed".into(),
                        name: "observed".into(),
                        reason: "observed_loaded",
                        evidence_scope: "target_agent",
                        evidence_agents: vec!["codex".into()],
                    },
                    crate::roster_recommendation::CoreSelection {
                        skill_id: "skill_fallback".into(),
                        name: "fallback".into(),
                        reason: "stable_fallback",
                        evidence_scope: "fallback",
                        evidence_agents: vec![],
                    },
                ],
            }],
        };

        let evidence = roster_selection_evidence(&recommendation);

        assert_eq!(evidence.summary["core_selection_count"], 3);
        assert_eq!(evidence.summary["forced_core_count"], 1);
        assert_eq!(evidence.summary["positive_signal_core_count"], 1);
        assert_eq!(evidence.summary["direct_signal_core_count"], 1);
        assert_eq!(evidence.summary["cross_agent_signal_core_count"], 0);
        assert_eq!(evidence.summary["stable_fallback_core_count"], 1);
        assert_eq!(evidence.summary["fallback_dominated"], false);
        assert_eq!(evidence.summary["detail_level"], "summary");
        assert_eq!(
            evidence.summary["agents"][0]["core_preview"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(
            evidence.summary["agents"][0]
                .get("core_selections")
                .is_none()
        );
        assert_eq!(evidence.full["detail_level"], "full");
        assert_eq!(
            evidence.full["agents"][0]["core_selections"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(evidence.uncertainty.is_none());
    }

    #[test]
    fn roster_selection_uncertainty_types_a_cross_agent_majority() {
        let cross_selection =
            |skill_id: &str, source: &str| crate::roster_recommendation::CoreSelection {
                skill_id: skill_id.into(),
                name: skill_id.into(),
                reason: "cross_agent_observed_loaded",
                evidence_scope: "cross_agent",
                evidence_agents: vec![source.into()],
            };
        let recommendation = crate::roster_recommendation::RosterRecommendation {
            changes: vec![],
            agents: vec![crate::roster_recommendation::AgentRecommendation {
                agent: AgentKind::ClaudeCode,
                before_default_exposure: 51,
                unique_skill_count: 51,
                core_count: 3,
                on_demand_count: 48,
                positive_signal_count: 2,
                direct_signal_count: 0,
                cross_agent_signal_count: 2,
                fallback_core_count: 1,
                core_selections: vec![
                    cross_selection("skill_a", "codex"),
                    cross_selection("skill_b", "cursor"),
                    crate::roster_recommendation::CoreSelection {
                        skill_id: "skill_c".into(),
                        name: "skill_c".into(),
                        reason: "stable_fallback",
                        evidence_scope: "fallback",
                        evidence_agents: vec![],
                    },
                ],
            }],
        };

        let evidence = roster_selection_evidence(&recommendation);

        assert_eq!(evidence.summary["positive_signal_core_count"], 2);
        assert_eq!(evidence.summary["direct_signal_core_count"], 0);
        assert_eq!(evidence.summary["cross_agent_signal_core_count"], 2);
        assert_eq!(evidence.summary["cross_agent_dominated"], true);
        assert_eq!(
            evidence.uncertainty.as_ref().unwrap()["code"],
            "cross_agent_dominated_core_selection"
        );
        assert_eq!(
            evidence.full["agents"][0]["core_selections"][0]["evidence_scope"],
            "cross_agent"
        );
        assert_eq!(
            evidence.full["agents"][0]["core_selections"][0]["evidence_agents"],
            json!(["codex"])
        );
    }

    #[test]
    fn roster_selection_uncertainty_preserves_mixed_agent_dominance() {
        let selection = |scope: &'static str, reason: &'static str, id: &str| {
            crate::roster_recommendation::CoreSelection {
                skill_id: id.into(),
                name: id.into(),
                reason,
                evidence_scope: scope,
                evidence_agents: if scope == "cross_agent" {
                    vec!["cursor".into()]
                } else {
                    Vec::new()
                },
            }
        };
        let agent =
            |agent: AgentKind, selections: Vec<crate::roster_recommendation::CoreSelection>| {
                let cross_agent_signal_count = selections
                    .iter()
                    .filter(|selection| selection.evidence_scope == "cross_agent")
                    .count();
                let fallback_core_count = selections
                    .iter()
                    .filter(|selection| selection.evidence_scope == "fallback")
                    .count();
                crate::roster_recommendation::AgentRecommendation {
                    agent,
                    before_default_exposure: 51,
                    unique_skill_count: 51,
                    core_count: selections.len(),
                    on_demand_count: 51 - selections.len(),
                    positive_signal_count: cross_agent_signal_count,
                    direct_signal_count: 0,
                    cross_agent_signal_count,
                    fallback_core_count,
                    core_selections: selections,
                }
            };
        let recommendation = crate::roster_recommendation::RosterRecommendation {
            changes: vec![],
            agents: vec![
                agent(
                    AgentKind::Codex,
                    vec![
                        selection("fallback", "stable_fallback", "fallback_a"),
                        selection("fallback", "stable_fallback", "fallback_b"),
                        selection("target_agent", "observed_loaded", "direct"),
                    ],
                ),
                agent(
                    AgentKind::ClaudeCode,
                    vec![
                        selection("cross_agent", "cross_agent_observed_loaded", "cross_a"),
                        selection("cross_agent", "cross_agent_observed_loaded", "cross_b"),
                        selection("fallback", "stable_fallback", "fallback_c"),
                    ],
                ),
            ],
        };

        let uncertainty = roster_selection_evidence(&recommendation)
            .uncertainty
            .unwrap();

        assert_eq!(
            uncertainty["code"],
            "mixed_evidence_dominated_core_selection"
        );
        assert_eq!(
            uncertainty["dominance_codes"],
            json!([
                "fallback_dominated_core_selection",
                "cross_agent_dominated_core_selection"
            ])
        );
        assert_eq!(uncertainty["fallback_dominated_agent_count"], 1);
        assert_eq!(uncertainty["cross_agent_dominated_agent_count"], 1);
        assert_eq!(uncertainty["review_required"], true);
    }

    #[test]
    fn exact_duplicate_planning_never_governs_provider_managed_placements() {
        let placement = |id: &str, governable: bool| scan::SkillPlacement {
            id: id.into(),
            skill_id: "skill_shared".into(),
            agent: governable.then_some(AgentKind::Codex),
            root: PathBuf::from(if governable {
                "/home/test/.codex/skills"
            } else {
                "/home/test/.codex/plugins/cache/market/plugin/1/skills"
            }),
            directory: PathBuf::from(format!("/fixture/{id}")),
            entrypoint: PathBuf::from(format!("/fixture/{id}/SKILL.md")),
            physical_directory: None,
            content_digest: "digest_shared".into(),
            entrypoint_digest: None,
            fingerprint_completeness: scan::FingerprintCompleteness::Complete,
            fingerprint_detail: None,
            link_target: None,
            link_status: scan::LinkStatus::NotLink,
            default_exposed: governable,
            owned_by_agent: Some(governable),
            mutation_scope: Some(if governable {
                scan::MutationScope::Mutable
            } else {
                scan::MutationScope::ProviderReadOnly
            }),
            governable,
            provider: (!governable).then(|| "plugin@market".into()),
            executable_files: Vec::new(),
            declared_name_matches_directory: Some(true),
        };
        let scan = ScanResult {
            placements: vec![
                placement("placement_agent", true),
                placement("placement_plugin", false),
            ],
            ..ScanResult::default()
        };
        let finding = FindingRecord {
            id: FindingId::parse("finding_external-duplicate").unwrap(),
            report_id: ReportId::parse("report_external-duplicate").unwrap(),
            category: FindingCategory::Overlap,
            severity: Severity::Warning,
            title: "Exact duplicate".into(),
            summary: String::new(),
            details: json!({
                "affected_skill_ids": ["skill_shared"],
                "affected_placement_ids": ["placement_agent", "placement_plugin"]
            }),
            evidence_ids: vec![EvidenceId::parse("evidence_external-duplicate").unwrap()],
        };
        let scan_id = ScanId::parse("scan_external-duplicate").unwrap();

        let planning = finding_library_planning(&finding, &scan_id, &scan_id, &scan).unwrap();

        assert_eq!(planning["supported"], false);
        assert_eq!(planning["reason"], "provider_managed_read_only");
        assert_eq!(planning["mutation_scopes"], json!(["provider_read_only"]));
        assert_eq!(planning["protected_placement_count"], 1);

        let blocker = finding_roster_safety_blocker(&[
            crate::roster_plan::RosterChangeExclusion {
                agent: "claude-code".into(),
                skill_id: "skill_package_variants".into(),
                name: "package-variants".into(),
                reason: "multiple_package_fingerprints_require_explicit_preservation",
                observed_source_target: None,
                safety_blocker: None,
            },
            crate::roster_plan::RosterChangeExclusion {
                agent: "codex".into(),
                skill_id: "skill_shared".into(),
                name: "shared".into(),
                reason: "provider_managed_placement_is_read_only",
                observed_source_target: None,
                safety_blocker: Some(crate::roster_plan::RosterSafetyBlocker::ProviderManaged {
                    skill_id: "skill_shared".into(),
                    placement_ids: vec!["placement_plugin".into()],
                    paths: vec![PathBuf::from("/fixture/placement_plugin")],
                    providers: vec!["plugin@market".into()],
                }),
            },
        ])
        .unwrap();
        let blocked: Value = serde_json::from_str(&error_json("plan", &blocker)).unwrap();
        assert_eq!(
            blocked["error"]["code"],
            "roster_provider_managed_read_only"
        );
        assert_eq!(
            blocked["error"]["details"]["placement_ids"],
            json!(["placement_plugin"])
        );
        assert_eq!(
            blocked["error"]["details"]["paths"],
            json!(["/fixture/placement_plugin"])
        );
        assert_eq!(
            blocked["error"]["details"]["providers"],
            json!(["plugin@market"])
        );
    }

    #[test]
    fn raw_library_plan_rejects_incomplete_package_fingerprints() {
        let placement = |id: &str, completeness| scan::SkillPlacement {
            id: id.into(),
            skill_id: "skill_bounded".into(),
            agent: Some(AgentKind::Codex),
            root: PathBuf::from("/fixture/root"),
            directory: PathBuf::from(format!("/fixture/{id}")),
            entrypoint: PathBuf::from(format!("/fixture/{id}/SKILL.md")),
            physical_directory: None,
            content_digest: "same-fallback-digest".into(),
            entrypoint_digest: None,
            fingerprint_completeness: completeness,
            fingerprint_detail: Some("package exceeded fingerprint limit".into()),
            link_target: None,
            link_status: scan::LinkStatus::NotLink,
            default_exposed: true,
            owned_by_agent: Some(true),
            mutation_scope: Some(scan::MutationScope::Mutable),
            governable: true,
            provider: None,
            executable_files: Vec::new(),
            declared_name_matches_directory: Some(true),
        };
        let scan = ScanResult {
            skills: vec![scan::ScannedSkill {
                id: "skill_bounded".into(),
                name: "bounded".into(),
                metadata: scan::SkillMetadata::default(),
                content_digest: "same-fallback-digest".into(),
                content_identity_digest: None,
                digest_algorithm: "sha256-v1".into(),
                summary: String::new(),
                normalized_text: String::new(),
                modified_at_unix: None,
            }],
            placements: vec![
                placement("placement_a", scan::FingerprintCompleteness::Bounded),
                placement("placement_b", scan::FingerprintCompleteness::Complete),
            ],
            ..ScanResult::default()
        };
        assert_eq!(
            persistence_identity_key(&scan.skills[0], None),
            "incomplete-snapshot-skill:skill_bounded"
        );
        assert_ne!(
            persistence_identity_key(&scan.skills[0], None),
            "content:same-fallback-digest"
        );
        let request = LibraryChangeRequest {
            skill_id: "skill_bounded".into(),
            canonical_placement_id: "placement_b".into(),
            placement_ids: vec!["placement_a".into(), "placement_b".into()],
            requested_state: RequestedGovernanceState::Managed,
        };

        let error = match normalize_library_plan(
            json!({}),
            &scan,
            Path::new("/fixture/state"),
            vec![request],
        ) {
            Ok(_) => panic!("incomplete package fingerprints must block Library planning"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("resolve fingerprint incompleteness before governance")
        );
        let blocked: Value = serde_json::from_str(&error_json("plan", error.as_ref())).unwrap();
        assert_eq!(blocked["error"]["code"], "incomplete_package_fingerprint");
        assert_eq!(blocked["error"]["details"]["stage"], "plan");
        assert_eq!(
            blocked["error"]["details"]["next_action"],
            "resolve_fingerprint_incompleteness_then_scan"
        );
        assert_eq!(
            blocked["error"]["details"]["remediation"]["required_before_rescan"],
            true
        );
        assert_eq!(blocked["error"]["details"]["files_changed"], false);
    }

    #[test]
    fn apply_validation_rejects_legacy_unknown_governance_fingerprints() {
        let prepared = PreparedPlan {
            id: PlanId::new().to_string(),
            scan_id: ScanId::new().to_string(),
            evidence_ids: vec![],
            digest: "sha256:test".into(),
            operations: vec![Operation::CreateDirectory {
                target: PathBuf::from("/fixture/new"),
                expected_fingerprint: "missing".into(),
            }],
            roster_changes: vec![change::RosterChange {
                agent: "codex".into(),
                skill_id: "skill_legacy".into(),
                state: "on_demand".into(),
            }],
            source_updates: vec![],
            library_changes: vec![],
            approved_roots: vec![PathBuf::from("/fixture")],
            state_dir: PathBuf::from("/fixture/state"),
        };
        let scan = ScanResult {
            placements: vec![scan::SkillPlacement {
                id: "placement_legacy".into(),
                skill_id: "skill_legacy".into(),
                agent: Some(AgentKind::Codex),
                root: PathBuf::from("/fixture/root"),
                directory: PathBuf::from("/fixture/root/legacy"),
                entrypoint: PathBuf::from("/fixture/root/legacy/SKILL.md"),
                physical_directory: None,
                content_digest: "legacy-digest".into(),
                entrypoint_digest: None,
                fingerprint_completeness: scan::FingerprintCompleteness::Unknown,
                fingerprint_detail: None,
                link_target: None,
                link_status: scan::LinkStatus::NotLink,
                default_exposed: true,
                owned_by_agent: Some(true),
                mutation_scope: Some(scan::MutationScope::Mutable),
                governable: true,
                provider: None,
                executable_files: vec![],
                declared_name_matches_directory: Some(true),
            }],
            ..ScanResult::default()
        };

        let error = validate_governance_fingerprint_completeness(&prepared, &scan).unwrap_err();

        let blocked: Value = serde_json::from_str(&error_json("apply", error.as_ref())).unwrap();
        assert_eq!(
            blocked["error"]["details"]["remediation"]["options"],
            json!(["scan_with_current_skillroster"])
        );
        assert_eq!(
            blocked["error"]["details"]["remediation"]["required_before_rescan"],
            false
        );
    }

    #[test]
    fn apply_validation_rejects_legacy_roster_operations_on_unknown_scope() {
        let placement_path = PathBuf::from("/fixture/root/legacy");
        let prepared = PreparedPlan {
            id: PlanId::new().to_string(),
            scan_id: ScanId::new().to_string(),
            evidence_ids: vec![],
            digest: "sha256:test".into(),
            operations: vec![Operation::MoveRecoverable {
                source: placement_path.clone(),
                target: PathBuf::from("/fixture/state/backup/legacy"),
                expected_fingerprint: "sha256:legacy".into(),
            }],
            roster_changes: vec![change::RosterChange {
                agent: "codex".into(),
                skill_id: "skill_legacy".into(),
                state: "core".into(),
            }],
            source_updates: vec![],
            library_changes: vec![],
            approved_roots: vec![PathBuf::from("/fixture")],
            state_dir: PathBuf::from("/fixture/state"),
        };
        let scan = ScanResult {
            placements: vec![scan::SkillPlacement {
                id: "placement_legacy".into(),
                skill_id: "skill_legacy".into(),
                agent: Some(AgentKind::Codex),
                root: PathBuf::from("/fixture/root"),
                directory: placement_path,
                entrypoint: PathBuf::from("/fixture/root/legacy/SKILL.md"),
                physical_directory: None,
                content_digest: "legacy-digest".into(),
                entrypoint_digest: None,
                fingerprint_completeness: scan::FingerprintCompleteness::Complete,
                fingerprint_detail: None,
                link_target: None,
                link_status: scan::LinkStatus::NotLink,
                default_exposed: true,
                owned_by_agent: None,
                mutation_scope: None,
                governable: true,
                provider: None,
                executable_files: vec![],
                declared_name_matches_directory: Some(true),
            }],
            ..ScanResult::default()
        };

        let error = validate_governance_operation_mutation_scopes(&prepared, &scan).unwrap_err();

        let blocker = error
            .downcast_ref::<crate::roster_plan::RosterSafetyBlocker>()
            .unwrap();
        assert!(matches!(
            blocker,
            crate::roster_plan::RosterSafetyBlocker::MutationScopeReadOnly {
                mutation_scopes,
                ..
            } if mutation_scopes == &["unknown"]
        ));
    }

    #[test]
    fn apply_validation_rejects_legacy_library_operations_on_unknown_scope() {
        let placement_path = PathBuf::from("/fixture/root/legacy");
        let prepared = PreparedPlan {
            id: PlanId::new().to_string(),
            scan_id: ScanId::new().to_string(),
            evidence_ids: vec![],
            digest: "sha256:test".into(),
            operations: vec![Operation::MoveRecoverable {
                source: placement_path.clone(),
                target: PathBuf::from("/fixture/state/backup/legacy"),
                expected_fingerprint: "sha256:legacy".into(),
            }],
            roster_changes: vec![],
            source_updates: vec![],
            library_changes: vec![change::LibraryChangeAction {
                skill_id: "skill_legacy".into(),
                canonical_placement_id: "placement_legacy".into(),
                placement_ids: vec!["placement_legacy".into()],
                requested_state: "managed".into(),
                canonical_path: placement_path.clone(),
                library_path: None,
            }],
            approved_roots: vec![PathBuf::from("/fixture")],
            state_dir: PathBuf::from("/fixture/state"),
        };
        let scan = ScanResult {
            placements: vec![placement_with_scope(
                "placement_legacy",
                "skill_legacy",
                placement_path.to_str().unwrap(),
                None,
            )],
            ..ScanResult::default()
        };

        let error = validate_governance_operation_mutation_scopes(&prepared, &scan).unwrap_err();

        let blocker = error
            .downcast_ref::<crate::roster_plan::RosterSafetyBlocker>()
            .unwrap();
        assert!(matches!(
            blocker,
            crate::roster_plan::RosterSafetyBlocker::MutationScopeReadOnly {
                skill_id,
                placement_ids,
                mutation_scopes,
                ..
            } if skill_id == "skill_legacy"
                && placement_ids == &["placement_legacy"]
                && mutation_scopes == &["unknown"]
        ));
    }

    #[test]
    fn apply_validation_rejects_legacy_source_updates_on_unknown_scope() {
        let placement_path = PathBuf::from("/fixture/root/legacy");
        let entrypoint = placement_path.join("SKILL.md");
        let prepared = PreparedPlan {
            id: PlanId::new().to_string(),
            scan_id: ScanId::new().to_string(),
            evidence_ids: vec![],
            digest: "sha256:test".into(),
            operations: vec![Operation::ReplaceFile {
                target: entrypoint.clone(),
                content: "replacement".into(),
                expected_fingerprint: "sha256:legacy".into(),
            }],
            roster_changes: vec![],
            source_updates: vec![change::SourceUpdateAction {
                skill_id: "skill_legacy".into(),
                placement_id: "placement_legacy".into(),
                choice: "adopt".into(),
                source: "fixture".into(),
                from_revision: "old".into(),
                to_revision: "new".into(),
                current_digest: "legacy-digest".into(),
                expected_file_fingerprint: "sha256:legacy".into(),
                upstream_digest: "upstream-digest".into(),
                baseline_trusted: true,
                choice_reason: "fixture".into(),
                target: entrypoint,
            }],
            library_changes: vec![],
            approved_roots: vec![PathBuf::from("/fixture")],
            state_dir: PathBuf::from("/fixture/state"),
        };
        let scan = ScanResult {
            placements: vec![placement_with_scope(
                "placement_legacy",
                "skill_legacy",
                placement_path.to_str().unwrap(),
                None,
            )],
            ..ScanResult::default()
        };

        let error = validate_governance_operation_mutation_scopes(&prepared, &scan).unwrap_err();

        let blocker = error
            .downcast_ref::<crate::roster_plan::RosterSafetyBlocker>()
            .unwrap();
        assert!(matches!(
            blocker,
            crate::roster_plan::RosterSafetyBlocker::MutationScopeReadOnly {
                skill_id,
                placement_ids,
                mutation_scopes,
                ..
            } if skill_id == "skill_legacy"
                && placement_ids == &["placement_legacy"]
                && mutation_scopes == &["unknown"]
        ));
    }

    #[test]
    fn operation_scope_blocker_groups_nested_matches_by_skill() {
        let prepared = PreparedPlan {
            id: PlanId::new().to_string(),
            scan_id: ScanId::new().to_string(),
            evidence_ids: vec![],
            digest: "sha256:test".into(),
            operations: vec![Operation::MoveRecoverable {
                source: PathBuf::from("/fixture/root/shared"),
                target: PathBuf::from("/fixture/state/backup/shared"),
                expected_fingerprint: "sha256:legacy".into(),
            }],
            roster_changes: vec![],
            source_updates: vec![],
            library_changes: vec![change::LibraryChangeAction {
                skill_id: "skill_a".into(),
                canonical_placement_id: "placement_a2".into(),
                placement_ids: vec!["placement_a2".into()],
                requested_state: "managed".into(),
                canonical_path: PathBuf::from("/fixture/root/shared/a2"),
                library_path: None,
            }],
            approved_roots: vec![PathBuf::from("/fixture")],
            state_dir: PathBuf::from("/fixture/state"),
        };
        let scan = ScanResult {
            placements: vec![
                placement_with_scope(
                    "placement_z",
                    "skill_z",
                    "/fixture/root/shared/z",
                    Some(scan::MutationScope::DurableReadOnly),
                ),
                placement_with_scope(
                    "placement_a2",
                    "skill_a",
                    "/fixture/root/shared/a2",
                    Some(scan::MutationScope::DurableReadOnly),
                ),
                placement_with_scope(
                    "placement_a1",
                    "skill_a",
                    "/fixture/root/shared/a1",
                    Some(scan::MutationScope::DurableReadOnly),
                ),
            ],
            ..ScanResult::default()
        };

        let error = validate_governance_operation_mutation_scopes(&prepared, &scan).unwrap_err();

        let blocker = error
            .downcast_ref::<crate::roster_plan::RosterSafetyBlocker>()
            .unwrap();
        assert!(matches!(
            blocker,
            crate::roster_plan::RosterSafetyBlocker::MutationScopeReadOnly {
                skill_id,
                placement_ids,
                ..
            } if skill_id == "skill_a"
                && placement_ids == &["placement_a1", "placement_a2"]
        ));
    }

    #[test]
    fn bootstrap_digest_classification_distinguishes_release_content_from_local_edits() {
        let current = bootstrap_content_digest(include_bytes!("../skill/skillroster/SKILL.md"));
        assert_eq!(
            bootstrap_content_status(&current, &current),
            BootstrapContentStatus::Current
        );
        assert!(
            !LEGACY_SINGLE_FILE_BOOTSTRAPS
                .iter()
                .any(|(_, digest)| *digest == current)
        );
        for (version, digest) in LEGACY_SINGLE_FILE_BOOTSTRAPS {
            assert_eq!(
                bootstrap_content_status(digest, &current),
                BootstrapContentStatus::OfficialOutdated(version)
            );
        }
        assert_eq!(
            bootstrap_content_status("sha256:local-edit", &current),
            BootstrapContentStatus::Modified
        );
        let windows_legacy =
            include_str!("../tests/fixtures/bootstrap-v1.4.0.md").replace('\n', "\r\n");
        assert_eq!(
            bootstrap_content_status(
                &bootstrap_content_digest(windows_legacy.as_bytes()),
                &current
            ),
            BootstrapContentStatus::OfficialOutdated("1.4.0")
        );
    }

    #[test]
    fn exact_complete_bootstrap_manifest_upgrades_and_undoes_without_accepting_a_mix() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        let root = home.join(".codex/skills");
        let package = root.join("skillroster");
        std::fs::create_dir_all(package.join("references")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let current = current_bootstrap_package();
        let previous = current
            .iter()
            .map(|(_, _, content)| format!("{content}\n<!-- previous official package -->\n"))
            .collect::<Vec<_>>();
        let previous_digests = previous
            .iter()
            .map(|content| content_digest(content.as_bytes()))
            .collect::<Vec<_>>();
        let previous_files = current
            .iter()
            .zip(&previous_digests)
            .map(|((relative_path, _, _), digest)| (*relative_path, digest.as_str()))
            .collect::<Vec<_>>();
        let manifests = [BootstrapPackageManifest {
            version: "previous-complete-fixture",
            file_digests: &previous_files,
        }];
        for ((relative_path, _, _), content) in current.iter().zip(&previous) {
            std::fs::write(package.join(relative_path), content).unwrap();
        }
        let store = StateStore::open(state.join("skillroster.db")).unwrap();
        scan_command(&store, &home, &state, Vec::new(), Vec::new()).unwrap();

        std::fs::write(package.join(current[1].0), &current[1].2).unwrap();
        let mixed = setup_command_with_manifests(&store, &home, &state, None, &manifests).unwrap();
        assert_eq!(mixed["state"], "modified_choice_required");
        assert_eq!(mixed["outdated_count"], 0);
        std::fs::write(package.join(current[1].0), &previous[1]).unwrap();

        let incomplete_manifests = [BootstrapPackageManifest {
            version: "incomplete-fixture",
            file_digests: &previous_files[..3],
        }];
        let incomplete =
            setup_command_with_manifests(&store, &home, &state, None, &incomplete_manifests)
                .unwrap();
        assert_eq!(incomplete["state"], "modified_choice_required");

        let duplicate_path_files = [
            previous_files[0],
            previous_files[1],
            previous_files[2],
            previous_files[2],
        ];
        let duplicate_path_manifests = [BootstrapPackageManifest {
            version: "duplicate-path-fixture",
            file_digests: &duplicate_path_files,
        }];
        let duplicate_path =
            setup_command_with_manifests(&store, &home, &state, None, &duplicate_path_manifests)
                .unwrap();
        assert_eq!(duplicate_path["state"], "modified_choice_required");

        let upgrade =
            setup_command_with_manifests(&store, &home, &state, None, &manifests).unwrap();
        assert_eq!(upgrade["state"], "preview_ready");
        assert_eq!(upgrade["outdated_count"], 1);
        assert_eq!(upgrade["operation_count"], 4);
        assert_eq!(
            upgrade["targets"][0]["installed_version"],
            "previous-complete-fixture"
        );
        let applied = apply_command(&store, upgrade["plan_id"].as_str().unwrap()).unwrap();
        for (relative_path, _, content) in &current {
            assert_eq!(
                std::fs::read_to_string(package.join(relative_path)).unwrap(),
                *content
            );
        }
        let undone = undo_command(&store, applied["receipt_id"].as_str().unwrap()).unwrap();
        assert_eq!(undone["verification"], "passed");
        for ((relative_path, _, _), content) in current.iter().zip(&previous) {
            assert_eq!(
                std::fs::read_to_string(package.join(relative_path)).unwrap(),
                *content
            );
        }
    }

    #[test]
    fn scan_warnings_group_repeated_unsafe_links_for_agent_callers() {
        let warnings = vec![
            "did not read unsafe Skill link /one/SKILL.md".into(),
            "did not read unsafe Skill link /two/SKILL.md".into(),
            "did not read unsafe Skill link /three/SKILL.md".into(),
            "session evidence was truncated".into(),
        ];

        let compact = compact_scan_warnings(warnings);

        assert_eq!(compact.len(), 2);
        assert_eq!(
            compact[0],
            "3 unsafe Skill links were not read; inspect the layout Finding for paths and link targets"
        );
        assert_eq!(compact[1], "session evidence was truncated");
    }

    #[test]
    fn source_roots_must_be_absolute_and_are_lexically_deduplicated() {
        assert!(parse_source_roots(&[PathBuf::from("relative")]).is_err());
        let absolute = std::env::current_dir().unwrap().join("source");
        assert_eq!(
            parse_source_roots(&[absolute.clone(), absolute.clone()]).unwrap(),
            vec![absolute]
        );
    }

    #[test]
    fn readable_skill_path_ignores_stale_alternatives_when_a_valid_copy_exists() {
        let temp = TempDir::new().unwrap();
        let first_root = temp.path().join("first");
        let second_root = temp.path().join("second");
        for (root, description) in [
            (&first_root, "Current database migration helper"),
            (&second_root, "Older unrelated helper"),
        ] {
            let directory = root.join("example");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join("SKILL.md"),
                format!("---\nname: example\ndescription: {description}\n---\n"),
            )
            .unwrap();
        }
        let mut options = ScanOptions::for_home(temp.path().join("home"));
        options.explicit_skill_roots = vec![
            ExplicitSkillRoot {
                agent: AgentKind::Codex,
                path: first_root.clone(),
            },
            ExplicitSkillRoot {
                agent: AgentKind::ClaudeCode,
                path: second_root,
            },
        ];
        options.include_session_evidence = false;
        let scan = scan::scan(&options).unwrap();
        let skill_id = scan
            .placements
            .iter()
            .find(|placement| placement.root == first_root)
            .unwrap()
            .skill_id
            .clone();

        let current = current_readable_skill_paths(&scan, temp.path(), &skill_id).unwrap();

        assert!(!current.drifted);
        assert_eq!(current.paths.len(), 1);
    }

    #[test]
    fn orphan_applied_journal_is_listed_and_blocks_the_next_write() {
        let temp = TempDir::new().unwrap();
        let state_dir = temp.path().join("state");
        std::fs::create_dir_all(state_dir.join("receipts")).unwrap();
        let receipt = ChangeReceipt {
            id: ReceiptId::new().to_string(),
            plan_id: PlanId::new().to_string(),
            status: change::ReceiptStatus::Applied,
            changed_paths: vec![temp.path().join("agent-skill")],
            compensations: vec![],
            approved_roots: vec![temp.path().to_path_buf()],
            state_dir: state_dir.clone(),
            error: None,
            reverses_receipt_id: None,
            operation_results: vec![],
        };
        std::fs::write(
            state_dir
                .join("receipts")
                .join(format!("{}.json", receipt.id)),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        let store = StateStore::open_in_memory().unwrap();
        let issues = journal_issues(&store, &state_dir).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0]["tracked_in_sqlite"], false);
        assert!(require_clear_journals(&store, &state_dir).is_err());
        assert_eq!(recovery_text(&store, &state_dir).unwrap(), "required");
        let database_path = state_dir.join("skillroster.db");
        drop(StateStore::open(&database_path).unwrap());
        assert!(lifecycle_delete_command(&database_path, &state_dir).is_err());
        assert!(database_path.is_file());
        assert!(
            state_dir
                .join("receipts")
                .join(format!("{}.json", receipt.id))
                .is_file()
        );
    }

    #[test]
    fn lifecycle_recovery_imports_orphan_journal_as_recovery_required() {
        let temp = TempDir::new().unwrap();
        let state_dir = temp.path().join("state");
        std::fs::create_dir_all(state_dir.join("receipts")).unwrap();
        let store = StateStore::open_in_memory().unwrap();
        let scan = ScanRun {
            id: ScanId::new(),
            started_at: 1,
            completed_at: Some(2),
            status: ScanStatus::Completed,
            coverage_notes: vec![],
        };
        store.save_scan(&scan).unwrap();
        let prepared = PreparedPlan {
            id: PlanId::new().to_string(),
            scan_id: scan.id.to_string(),
            evidence_ids: vec![],
            digest: "sha256:test".to_owned(),
            operations: vec![],
            roster_changes: vec![],
            source_updates: vec![],
            library_changes: vec![],
            approved_roots: vec![temp.path().to_path_buf()],
            state_dir: state_dir.clone(),
        };
        let plan = PlanRecord {
            id: PlanId::parse(prepared.id.clone()).unwrap(),
            scan_id: scan.id,
            report_id: None,
            created_at: 3,
            status: PlanStatus::Ready,
            input: json!({
                "prepared": prepared,
                "roster_before": [],
                "library_before": [],
            }),
            fingerprint: "sha256:test".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        let receipt = ChangeReceipt {
            id: ReceiptId::new().to_string(),
            plan_id: plan.id.to_string(),
            status: change::ReceiptStatus::Applied,
            changed_paths: vec![temp.path().join("agent-skill")],
            compensations: vec![],
            approved_roots: vec![temp.path().to_path_buf()],
            state_dir: state_dir.clone(),
            error: None,
            reverses_receipt_id: None,
            operation_results: vec![],
        };
        std::fs::write(
            state_dir
                .join("receipts")
                .join(format!("{}.json", receipt.id)),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();

        let result = lifecycle_recovery_command(&store, &state_dir).unwrap();
        assert_eq!(result["imported_receipt_ids"][0], receipt.id);
        let imported = store
            .get_receipt(&ReceiptId::parse(receipt.id).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(imported.status, ReceiptStatus::RecoveryRequired);
        assert_eq!(
            store.get_plan(&plan.id).unwrap().unwrap().status,
            PlanStatus::RecoveryRequired
        );
    }

    #[test]
    fn plan_detail_keeps_legacy_selection_evidence_without_full_selections() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = ScanRun {
            id: ScanId::new(),
            started_at: 1,
            completed_at: Some(2),
            status: ScanStatus::Completed,
            coverage_notes: vec![],
        };
        store.save_scan(&scan).unwrap();
        let prepared = PreparedPlan {
            id: PlanId::new().to_string(),
            scan_id: scan.id.to_string(),
            evidence_ids: vec![],
            digest: "sha256:legacy".to_owned(),
            operations: vec![],
            roster_changes: vec![],
            source_updates: vec![],
            library_changes: vec![],
            approved_roots: vec![],
            state_dir: PathBuf::new(),
        };
        let legacy_selection = json!({
            "core_selection_count": 10,
            "stable_fallback_core_count": 9
        });
        let plan = PlanRecord {
            id: PlanId::parse(prepared.id.clone()).unwrap(),
            scan_id: scan.id,
            report_id: None,
            created_at: 3,
            status: PlanStatus::Ready,
            input: json!({
                "prepared": prepared,
                "roster_before": [],
                "library_before": [],
                "summary": {"selection_evidence": legacy_selection}
            }),
            fingerprint: "sha256:legacy".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();

        let detail = plan_detail_command(&store, plan.id.as_str()).unwrap();

        assert_eq!(detail["selection_evidence"], legacy_selection);
        assert_eq!(detail["detail_level"], "full");
    }

    #[test]
    fn combined_lifecycle_purge_checks_recovery_before_mutating_usage() {
        let temp = TempDir::new().unwrap();
        let state_dir = temp.path().join("state");
        std::fs::create_dir_all(state_dir.join("receipts")).unwrap();
        let store = StateStore::open_in_memory().unwrap();
        let scan = ScanRun {
            id: ScanId::new(),
            started_at: 1,
            completed_at: Some(2),
            status: ScanStatus::Completed,
            coverage_notes: vec![],
        };
        store.save_scan(&scan).unwrap();
        store
            .save_scan_payload(
                &scan.id,
                &json!({
                    "usage": [{
                        "agent": "codex",
                        "skill_id": "skill_fixture",
                        "stage": "loaded",
                        "quality": "observed",
                        "event_count": 1,
                        "first_seen_unix": 1,
                        "last_seen_unix": 1
                    }]
                }),
            )
            .unwrap();
        let orphan = ChangeReceipt {
            id: ReceiptId::new().to_string(),
            plan_id: PlanId::new().to_string(),
            status: change::ReceiptStatus::Applied,
            changed_paths: vec![temp.path().join("agent-skill")],
            compensations: vec![],
            approved_roots: vec![temp.path().to_path_buf()],
            state_dir: state_dir.clone(),
            error: None,
            reverses_receipt_id: None,
            operation_results: vec![],
        };
        std::fs::write(
            state_dir
                .join("receipts")
                .join(format!("{}.json", orphan.id)),
            serde_json::to_vec(&orphan).unwrap(),
        )
        .unwrap();

        assert!(lifecycle_purge_command(&store, &state_dir, Some(0), true, false).is_err());
        let (_, payload): (ScanId, Value) = store.latest_scan_payload().unwrap().unwrap();
        assert_eq!(payload["usage"].as_array().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn source_confirmation_purge_validates_paths_before_mutating_usage() {
        let temp = TempDir::new().unwrap();
        let state_dir = temp.path().join("state");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep.json"), b"{}").unwrap();
        std::os::unix::fs::symlink(&outside, state_dir.join("source-confirmation")).unwrap();
        let store = StateStore::open_in_memory().unwrap();
        let scan = ScanRun {
            id: ScanId::new(),
            started_at: 1,
            completed_at: Some(2),
            status: ScanStatus::Completed,
            coverage_notes: vec![],
        };
        store.save_scan(&scan).unwrap();
        store
            .save_scan_payload(
                &scan.id,
                &json!({
                    "usage": [{
                        "agent": "codex",
                        "skill_id": "skill_fixture",
                        "stage": "loaded",
                        "quality": "observed",
                        "event_count": 1,
                        "first_seen_unix": 1,
                        "last_seen_unix": 1
                    }]
                }),
            )
            .unwrap();

        assert!(lifecycle_purge_command(&store, &state_dir, Some(0), false, true).is_err());

        let (_, payload): (ScanId, Value) = store.latest_scan_payload().unwrap().unwrap();
        assert_eq!(payload["usage"].as_array().unwrap().len(), 1);
        assert!(outside.join("keep.json").is_file());
    }

    #[test]
    fn transition_counts_only_effective_state_changes() {
        let before = vec![
            json!({"state": "core"}),
            json!({"state": "on_demand"}),
            json!({"state": Value::Null}),
        ];
        let after = vec![
            json!({"state": "core"}),
            json!({"state": "core"}),
            json!({"state": "on_demand"}),
        ];

        assert_eq!(transition_change_count(&before, &after, "state"), 2);
    }

    #[test]
    fn bounded_plan_impact_is_stable_after_json_storage_round_trip() {
        let impact = bounded_plan_impact(json!({
            "before_default_exposure": 548,
            "after_default_exposure": 242,
            "exposure_reduction_percent": 55.839416058394164_f64
        }));
        let stored: Value = serde_json::from_str(&serde_json::to_string(&impact).unwrap()).unwrap();

        assert_eq!(impact, stored);
        assert_eq!(impact["exposure_reduction_percent"], 55.84);
    }

    #[test]
    fn bounded_library_impact_keeps_complete_totals_when_items_are_truncated() {
        let items = (0..12)
            .map(|index| {
                json!({
                    "skill_id": format!("skill_{index}"),
                    "before": {
                        "governance_state": "observed",
                        "physical_source_count": 3,
                        "placement_count": 6,
                        "default_exposed_placement_count": 5
                    },
                    "after": {
                        "governance_state": "managed",
                        "physical_source_count": 1,
                        "placement_count": 6,
                        "default_exposed_placement_count": 5,
                        "relinked_placement_count": 2
                    }
                })
            })
            .collect::<Vec<_>>();

        let impact = bounded_plan_impact(json!(items));

        assert_eq!(impact["item_count"], 12);
        assert_eq!(impact["items"].as_array().unwrap().len(), 10);
        assert_eq!(impact["items_truncated"], true);
        assert_eq!(impact["totals"]["before"]["physical_source_count"], 36);
        assert_eq!(impact["totals"]["after"]["physical_source_count"], 12);
        assert_eq!(impact["totals"]["delta"]["physical_source_count"], -24);
        assert_eq!(impact["totals"]["before"]["placement_count"], 72);
        assert_eq!(impact["totals"]["after"]["placement_count"], 72);
        assert_eq!(impact["totals"]["delta"]["placement_count"], 0);
        assert_eq!(
            impact["totals"]["delta"]["default_exposed_placement_count"],
            0
        );
        assert_eq!(impact["totals"]["relinked_placement_count"], 24);
    }

    #[test]
    fn source_block_json_keeps_complete_roots_in_the_detail_file() {
        let state = TempDir::new().unwrap();
        let action_argv_prefix = vec![
            "--state-dir".to_owned(),
            state.path().display().to_string(),
            "--home".to_owned(),
            crate::roster_plan::test_absolute_path("home")
                .display()
                .to_string(),
        ];
        let exclusions = (0..11)
            .map(|index| {
                let skill_id = format!("skill_{index:032}");
                let path =
                    crate::roster_plan::test_absolute_path(&format!("opt/root-{index:02}/pkg"));
                crate::roster_plan::RosterChangeExclusion {
                    agent: "codex".into(),
                    skill_id: skill_id.clone(),
                    name: format!("skill-{index:02}"),
                    reason: "untrusted_external_placement_blocks_mutation",
                    observed_source_target: Some(path.clone()),
                    safety_blocker: Some(
                        crate::roster_plan::RosterSafetyBlocker::MutationScopeReadOnly {
                            skill_id,
                            placement_ids: vec![format!("placement_{index:032}")],
                            paths: vec![path],
                            owned_by_agent: Some(true),
                            mutation_scopes: vec!["untrusted_external".into()],
                        },
                    ),
                }
            })
            .collect::<Vec<_>>();
        let blocked = crate::roster_plan::source_confirmation_block(
            "finding_fixture",
            10,
            &exclusions,
            state.path(),
            &action_argv_prefix,
        )
        .unwrap();
        let envelope: Value = serde_json::from_str(&error_json_with_context(
            "plan",
            &blocked,
            &ActionContext {
                argv: action_argv_prefix.clone(),
            },
        ))
        .unwrap();
        assert_eq!(
            envelope["error"]["details"]["blocked_changes"]
                .as_array()
                .unwrap()
                .len(),
            10
        );
        assert_eq!(
            envelope["error"]["details"]["source_roots"]
                .as_array()
                .unwrap()
                .len(),
            10
        );
        assert_eq!(
            envelope["error"]["details"]["blocked_changes_truncated"],
            true
        );
        assert_eq!(envelope["error"]["details"]["source_roots_truncated"], true);
        assert_eq!(envelope["error"]["paths"].as_array().unwrap().len(), 10);
        let argv = envelope["suggested_actions"][0]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(argv[1..=action_argv_prefix.len()], action_argv_prefix);
        let bounded_template = envelope["error"]["details"]["after_confirmation"]["argv_template"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            bounded_template[1..=action_argv_prefix.len()],
            action_argv_prefix
        );
        let expected_roots = (0..11)
            .map(|index| {
                crate::roster_plan::test_absolute_path(&format!("opt/root-{index:02}/pkg"))
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>();
        for root in expected_roots.iter().take(10) {
            assert!(
                argv.windows(2)
                    .any(|pair| pair[0] == "--source-root" && pair[1] == root),
                "missing bounded --source-root {root} in {argv:?}"
            );
        }
        assert!(!argv.iter().any(|value| value == &expected_roots[10]));
        let detail_path = envelope["error"]["details"]["detail"]["path"]
            .as_str()
            .unwrap();
        let complete: Value = serde_json::from_slice(&std::fs::read(detail_path).unwrap()).unwrap();
        assert_eq!(complete["schema_version"], 2);
        assert_eq!(complete["action_context_argv"], json!(action_argv_prefix));
        assert_eq!(complete["source_roots"], json!(expected_roots));
        let complete_argv = complete["after_confirmation"]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            complete_argv[1..=action_argv_prefix.len()],
            action_argv_prefix
        );
        for root in &expected_roots {
            assert!(
                complete_argv
                    .windows(2)
                    .any(|pair| pair[0] == "--source-root" && pair[1] == root),
                "missing complete --source-root {root} in {complete_argv:?}"
            );
        }
        validate_source_confirmation_detail(Path::new(detail_path)).unwrap();
        assert_eq!(
            source_confirmation_detail_summary(state.path()).unwrap()["count"],
            1
        );
        assert_eq!(
            read_source_confirmation_details(state.path())
                .unwrap()
                .len(),
            1
        );
        let mut unrecognized = complete.clone();
        unrecognized["action_context_argv"] = json!(["--yes", "true"]);
        assert!(!recognized_source_confirmation_detail(&unrecognized));
        assert_eq!(remove_source_confirmation_details(state.path()).unwrap(), 1);
        assert!(!Path::new(detail_path).exists());
    }

    #[test]
    fn roster_operation_conflict_exposes_operation_facts_as_json() {
        let error = crate::roster_plan::RosterOperationConflict {
            identity_role: "target",
            operation_kinds: vec!["move_recoverable".into(), "create_symlink".into()],
            path: PathBuf::from("/tmp/library/shared"),
        };

        let output: Value = serde_json::from_str(&error_json("plan", &error)).unwrap();

        assert_eq!(
            output["error"]["code"],
            "roster_operation_identity_conflict"
        );
        assert_eq!(
            output["error"]["details"]["reason"],
            "duplicate_physical_operation_identity"
        );
        assert_eq!(
            output["error"]["details"]["conflicting_operation_kinds"],
            json!(["move_recoverable", "create_symlink"])
        );
        assert_eq!(output["error"]["details"]["identity_role"], "target");
        assert_eq!(output["error"]["details"]["path"], "/tmp/library/shared");
        assert_eq!(output["error"]["details"]["files_changed"], false);
    }

    #[test]
    fn roster_package_variants_expose_a_typed_no_change_blocker() {
        let error = crate::roster_plan::RosterPackageFingerprintVariants {
            skill_id: "skill_fixture".into(),
            placement_ids: vec!["placement_a".into(), "placement_b".into()],
            fingerprint_count: 2,
        };

        let output: Value = serde_json::from_str(&error_json("plan", &error)).unwrap();

        assert_eq!(
            output["error"]["code"],
            "roster_package_fingerprint_variants"
        );
        assert_eq!(
            output["error"]["details"]["reason"],
            "multiple_package_fingerprints_require_explicit_preservation"
        );
        assert_eq!(output["error"]["details"]["fingerprint_count"], 2);
        assert_eq!(output["error"]["details"]["files_changed"], false);
        assert_eq!(
            output["error"]["details"]["next_action"],
            "review_each_package_before_roster_mutation"
        );
    }

    #[test]
    fn roster_safety_blockers_expose_stable_agent_actions() {
        let provider = crate::roster_plan::RosterSafetyBlocker::ProviderManaged {
            skill_id: "skill_fixture".into(),
            placement_ids: vec!["placement_provider".into()],
            paths: vec![PathBuf::from("/provider/skill")],
            providers: vec!["fixture-provider".into()],
        };
        let provider: Value = serde_json::from_str(&error_json("plan", &provider)).unwrap();
        assert_eq!(
            provider["error"]["code"],
            "roster_provider_managed_read_only"
        );
        assert_eq!(
            provider["error"]["details"]["reason"],
            "provider_managed_placement_is_read_only"
        );
        assert_eq!(
            provider["error"]["details"]["placement_ids"],
            json!(["placement_provider"])
        );
        assert_eq!(
            provider["error"]["details"]["paths"],
            json!(["/provider/skill"])
        );
        assert_eq!(
            provider["error"]["details"]["providers"],
            json!(["fixture-provider"])
        );
        assert_eq!(
            provider["error"]["details"]["next_action"],
            "exclude_provider_managed_placement"
        );

        let dependent = crate::roster_plan::RosterSafetyBlocker::DependentSource {
            skill_id: "skill_fixture".into(),
            placement_ids: vec!["placement_source".into()],
            paths: vec![PathBuf::from("/sources/skill")],
        };
        let dependent: Value = serde_json::from_str(&error_json("plan", &dependent)).unwrap();
        assert_eq!(
            dependent["error"]["code"],
            "roster_dependent_source_conflict"
        );
        assert_eq!(
            dependent["error"]["details"]["next_action"],
            "preserve_or_retarget_dependent_source"
        );
        assert_eq!(dependent["error"]["details"]["files_changed"], false);
    }

    #[test]
    fn physical_drift_exposes_expected_and_current_paths() {
        let drift = crate::scan::PhysicalDirectoryDrift {
            placement_id: "placement_fixture".into(),
            expected: Some(PathBuf::from("/tmp/shared-a/skill")),
            current: Some(PathBuf::from("/tmp/shared-b/skill")),
        };

        let output: Value = serde_json::from_str(&error_json("plan", &drift)).unwrap();

        assert_eq!(output["error"]["code"], "state_drift");
        assert_eq!(
            output["error"]["details"]["reason"],
            "physical_source_drift"
        );
        assert_eq!(
            output["error"]["details"]["placement_id"],
            "placement_fixture"
        );
        assert_eq!(
            output["error"]["details"]["expected_path"],
            "/tmp/shared-a/skill"
        );
        assert_eq!(
            output["error"]["details"]["current_path"],
            "/tmp/shared-b/skill"
        );
        assert_eq!(output["error"]["details"]["next_action"], "scan");
        assert_eq!(output["error"]["details"]["files_changed"], false);
    }
}

fn finding_category(value: crate::query::FindingCategory) -> FindingCategory {
    match value {
        crate::query::FindingCategory::Inventory => FindingCategory::Inventory,
        crate::query::FindingCategory::Layout => FindingCategory::Layout,
        crate::query::FindingCategory::Exposure => FindingCategory::Exposure,
        crate::query::FindingCategory::Usage => FindingCategory::Usage,
        crate::query::FindingCategory::Overlap => FindingCategory::Overlap,
        crate::query::FindingCategory::Routing => FindingCategory::Routing,
        crate::query::FindingCategory::Lifecycle => FindingCategory::Lifecycle,
    }
}

fn severity(value: crate::query::Severity) -> Severity {
    match value {
        crate::query::Severity::Info | crate::query::Severity::Low => Severity::Info,
        crate::query::Severity::Medium => Severity::Warning,
        crate::query::Severity::High => Severity::Critical,
    }
}
