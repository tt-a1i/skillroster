use std::collections::BTreeSet;
use std::io::Read;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::change::{
    self, ChangeReceipt, Operation, OperationPolicy, PrepareContext, PreparedPlan,
};
use crate::cli::{
    Cli, Command, LifecycleCommand, ModifiedBootstrapChoice, ReportCategory, ReportSeverity,
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

pub struct Output {
    pub json: String,
    pub human: String,
}

pub fn run(cli: Cli) -> Result<Output> {
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
                    "The SQLite database and Receipt journals will be deleted. Agent and Library files are preserved. A new Scan rebuilds inventory state.",
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

    let (command, result, warnings, actions) = match cli.command {
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
            } else {
                vec![action(
                    "scan",
                    &["scan", "--json"],
                    false,
                    false,
                    "refresh_facts",
                )]
            };
            ("status", result, vec![], actions)
        }
        Some(Command::Scan) => {
            let (result, warnings) = scan_command(
                &store,
                &home,
                parse_explicit_roots(&cli.roots)?,
                parse_source_roots(&cli.source_roots)?,
            )?;
            (
                "scan",
                result,
                warnings,
                vec![action(
                    "report",
                    &["report", "--summary", "--json"],
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
                ReportRequest::Exhaustive
            };
            let result = report_command(&store, request)?;
            let actions = report_actions(&result, request);
            ("report", result, vec![], actions)
        }
        Some(Command::Find(args)) => (
            "find",
            find_command(
                &store,
                &state_dir,
                &args.task,
                &args.hints,
                usize::from(args.limit),
            )?,
            vec![],
            vec![],
        ),
        Some(Command::Plan(args)) => {
            let showing_detail = args.show.is_some();
            let result = match args.show.as_deref() {
                Some(id) => plan_detail_command(&store, id)?,
                None => plan_command(&store, &state_dir)?,
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
                lifecycle_export_command(&store, &args.output)?,
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
                if args.raw_days.is_none() && !args.plans_receipts {
                    bail!("purge requires --raw-days DAYS and/or --plans-receipts");
                }
                if args.plans_receipts && args.confirm.as_deref() != Some("PURGE-PLANS-RECEIPTS") {
                    bail!("Plans and Receipts purge requires --confirm PURGE-PLANS-RECEIPTS");
                }
                if !cli.json
                    && !require_human_confirmation(
                        "Purge the explicitly selected local lifecycle state?",
                        if args.plans_receipts {
                            "Selected Plans, Receipts, and their Undo history will be deleted. Agent and Library files are preserved."
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

    let mut envelope = JsonEnvelope::success(command, result.clone());
    envelope.warnings = warnings;
    envelope.suggested_actions = actions;
    Ok(Output {
        json: serde_json::to_string(&envelope)?,
        human: crate::present::human(command, &result),
    })
}

fn command_requires_exclusive_state_lock(command: Option<&Command>) -> bool {
    matches!(
        command,
        Some(Command::Apply(_) | Command::Undo(_))
            | Some(Command::Lifecycle(crate::cli::LifecycleArgs {
                command: LifecycleCommand::Purge(_),
            }))
    )
}

pub fn error_json(command: &str, error: &(dyn std::error::Error + 'static)) -> String {
    let classified = classify_error(error);
    serde_json::to_string(&JsonEnvelope::<Value>::failure(
        command,
        ApiError {
            code: classified.0.into(),
            message: error.to_string(),
            retryable: classified.1,
            relevant_ids: extract_relevant_ids(&error.to_string()),
            paths: extract_paths(&error.to_string()),
        },
    ))
    .unwrap_or_else(|_| r#"{"schema_version":1,"ok":false}"#.into())
}

fn classify_error(error: &(dyn std::error::Error + 'static)) -> (&'static str, bool) {
    let message = error.to_string().to_lowercase();
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
    } else if message.contains("must be absolute") || message.contains("unsupported agent") {
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
    let pending_plans = store
        .pending_plans()?
        .into_iter()
        .map(|plan| {
            json!({
                "plan_id": plan.id,
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
        "pending_plan_count": pending_plans.len(),
        "pending_plans": pending_plans,
        "last_receipt": last_receipt,
        "recovery_state": recovery_text(store, state_dir)?,
        "journal_issues": journal_issues(store, state_dir)?,
        "retention": {
            "raw_usage_days": 180,
            "older_usage": "monthly_aggregates_retained",
            "automatic_purge": false,
            "current": lifecycle,
        },
        "files_changed": false
    }))
}

fn lifecycle_export_command(store: &StateStore, output: &Path) -> Result<Value> {
    let export = json!({
        "schema_version": 1,
        "generated_at": Utc::now().timestamp(),
        "retention": {
            "raw_usage_days": 180,
            "older_usage": "monthly_aggregates_retained",
        },
        "data": store.export_lifecycle()?,
        "evidence_exclusions": store.evidence_exclusions()?,
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
        "files_changed": true,
    }))
}

fn lifecycle_inspect_command(
    store: &StateStore,
    database_path: &Path,
    state_dir: &Path,
) -> Result<Value> {
    Ok(json!({
        "operation": "inspect",
        "database_path": database_path,
        "counts": store.lifecycle_counts()?,
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
) -> Result<Value> {
    if plans_receipts && recovery_text(store, state_dir)? == "required" {
        bail!("recovery is required before Plans and Receipts can be purged");
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
    Ok(json!({
        "operation": "purge",
        "raw_usage_days": raw_days,
        "cutoff": cutoff,
        "usage_result": usage_result,
        "plan_receipt_result": plan_receipt_result,
        "monthly_aggregates_retained": raw_days.is_some(),
        "plans_or_receipts_changed": plans_or_receipts_changed,
        "agent_files_changed": false,
        "library_files_changed": false,
        "files_changed": usage_changed || plans_or_receipts_changed,
    }))
}

fn lifecycle_delete_command(database_path: &Path, state_dir: &Path) -> Result<Value> {
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("cannot create {}", state_dir.display()))?;
    let _write_lock = change::WriteLock::acquire(state_dir)?;
    let existed = database_path.exists();
    let journals = change::journals(state_dir)?;
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
    let files_changed = !removed_database_files.is_empty()
        || removed_journals > 0
        || removed_recovery_directories > 0;
    Ok(json!({
        "operation": "delete_local_state",
        "database_path": database_path,
        "database_existed": existed,
        "removed_database_files": removed_database_files,
        "removed_receipt_journals": removed_journals,
        "removed_recovery_directories": removed_recovery_directories,
        "rebuild_command": "skillroster scan --json",
        "agent_files_changed": false,
        "library_files_changed": false,
        "files_changed": files_changed,
    }))
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
        if store.receipt_exists(&receipt_id)? {
            continue;
        }
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
        match store.save_receipt(&imported_receipt) {
            Ok(()) => {
                store.mark_plan_recovery_if_applying(&plan_id)?;
                imported.push(receipt_id);
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
    explicit: Vec<ExplicitSkillRoot>,
    source_roots: Vec<PathBuf>,
) -> Result<(Value, Vec<String>)> {
    let started = Utc::now().timestamp();
    let id = ScanId::new();
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
    options.excluded_session_agents = store
        .evidence_exclusions()?
        .iter()
        .map(|agent| parse_agent_kind(agent))
        .collect::<Result<_>>()?;
    let result = match scan::scan(&options) {
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
    let warnings = compact_scan_warnings(result.warnings);
    Ok((
        json!({
            "snapshot_id": id,
            "agents_checked": agents_checked,
            "skill_count": result.skills.len(),
            "placement_count": result.placements.len(),
            "roots": result.roots,
            "coverage": result.coverage,
            "files_changed": false
        }),
        warnings,
    ))
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

fn report_command(store: &StateStore, request: ReportRequest<'_>) -> Result<Value> {
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
            let latest_scan_id = store
                .latest_completed_scan()?
                .ok_or_else(|| anyhow!("no completed Snapshot exists"))?
                .id;
            if let Some(planning) =
                finding_library_planning(&stored, &report.scan_id, &latest_scan_id, &scan)
            {
                object.insert("planning".into(), planning);
            } else if let Some(planning) =
                finding_roster_planning(store, &stored, &report.scan_id, &latest_scan_id, &scan)?
            {
                object.insert("planning".into(), planning);
            }
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
            let paged_evidence_ids = stored
                .evidence_ids
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
                        "governable": placement.governable,
                        "provider": placement.provider,
                        "content_digest": placement.content_digest
                    })
                })
                .collect::<Vec<_>>();
            placements.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
            let evidence = paged_evidence_ids
                .iter()
                .map(|id| store.get_evidence(id))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let total = affected_skill_ids
                .len()
                .max(affected_placement_ids.len())
                .max(stored.evidence_ids.len());
            let next_offset = (end < total).then_some(end);
            object.insert("affected_skill_ids".into(), json!(paged_skill_ids));
            object.insert("affected_placement_ids".into(), json!(paged_placement_ids));
            object.insert("evidence_ids".into(), json!(paged_evidence_ids));
            object.insert(
                "primary_evidence_id".into(),
                json!(stored.evidence_ids.first()),
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
                        "evidence": stored.evidence_ids.len()
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
    if object.get("title").and_then(Value::as_str)
        != Some("Same-name Skills have different content")
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
            let governable = placements.iter().any(|placement| placement.governable);
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
                "governable": governable
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
            let returned = result["findings"]
                .as_array()
                .map_or(0, |findings| findings.len() as u64);
            if total <= returned {
                return Vec::new();
            }
            vec![action(
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
            )]
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
            let planning_blocked = result["planning"]["supported"].as_bool() == Some(false);
            let mut actions = Vec::new();
            if !requires_trust_decision && !requires_variant_decision && !planning_blocked {
                actions.push(action(
                    "plan",
                    &["plan", "--stdin", "--json"],
                    false,
                    false,
                    "finding_action_available",
                ));
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
    let protected_count = scan
        .placements
        .iter()
        .filter(|placement| affected.contains(placement.id.as_str()) && !placement.governable)
        .count();
    if protected_count > 0 {
        return Some(json!({
            "supported": false,
            "reason": "external_observed_placements",
            "snapshot_id": scan_id,
            "protected_placement_count": protected_count,
            "next_step": "Keep provider-managed plugin Skills read-only; govern only Agent-owned placements."
        }));
    }
    let mut physical_groups =
        std::collections::BTreeMap::<PathBuf, Vec<&scan::SkillPlacement>>::new();
    for placement in scan
        .placements
        .iter()
        .filter(|placement| affected.contains(placement.id.as_str()) && placement.governable)
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
                    "governable": placement.governable,
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

fn finding_roster_planning(
    store: &StateStore,
    finding: &FindingRecord,
    scan_id: &ScanId,
    latest_scan_id: &ScanId,
    scan: &ScanResult,
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
    let recommendation = match crate::roster_recommendation::recommend(
        finding,
        scan,
        &declared_core_pairs(store, scan)?,
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
    let (selection_evidence, uncertainty) = roster_selection_evidence(&recommendation);
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
                        "reason": selection.reason
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
        .map(|exclusion| {
            json!({
                "agent": exclusion.agent,
                "skill_id": exclusion.skill_id,
                "reason": exclusion.reason,
                "state": "unchanged"
            })
        })
        .collect::<Vec<_>>();
    if blocked_change_count > 0 {
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
                "dependent_link_targets": observed_link_targets,
                "after_resolution": {
                    "next": "move or retarget the dependent source link, then rescan and reopen the new large-Roster Finding"
                }
            })));
        }
        return Ok(Some(json!({
            "supported": false,
            "reason": "trusted_canonical_sources_required",
            "decision": "confirm_trusted_source_roots",
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
            "positive_usage_evidence",
            "stable_fallback"
        ],
        "absence_of_usage_evidence": "not_negative_evidence",
        "automatic_target_states": ["core", "on_demand"],
        "explicit_only_or_archive_decision_implied": false,
        "selection_evidence": selection_evidence,
        "uncertainty": uncertainty,
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
    let mut value = json!({
        "id": id,
        "kind": crate::roster_recommendation::finding_kind(
            &finding_category(finding.category),
            &finding.title
        ),
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
    let reliable_agents = scan
        .coverage
        .iter()
        .filter(|coverage| coverage.denominator_reliable)
        .map(|coverage| coverage.agent.id())
        .collect::<Vec<_>>();
    let limited_agents = scan
        .coverage
        .iter()
        .filter(|coverage| !coverage.denominator_reliable)
        .map(|coverage| coverage.agent.id())
        .collect::<Vec<_>>();
    json!({
        "evidence_quality": finding.evidence_quality,
        "reliable_agents": reliable_agents,
        "limited_agents": limited_agents,
        "supported_agent_count": AgentKind::ALL.len(),
        "denominator_reliable": limited_agents.is_empty()
    })
}

fn find_command(
    store: &StateStore,
    state_dir: &Path,
    task: &str,
    hints: &[String],
    limit: usize,
) -> Result<Value> {
    let (scan_id, scan) = latest_scan(store)?;
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
    for skill_id in routable_ids.clone() {
        let id = SkillId::parse(skill_id.clone())?;
        let states = store.roster_states_for_skill(&id)?;
        if !states.is_empty() && states.iter().all(|state| *state == RosterState::Archived) {
            routable_ids.remove(&skill_id);
        }
    }
    if crate::query::contains_cjk(retrieval_query.text()) {
        candidate_ids.extend(routable_ids.iter().cloned());
    }
    candidate_ids.retain(|skill_id| routable_ids.contains(skill_id));
    let pool_limit = if retrieval_hints.is_empty() {
        limit
    } else {
        limit.saturating_mul(4).clamp(20, 100)
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
    for (index, found) in matches.iter_mut().enumerate() {
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
        if index < 3 && found.variant_count > 1 {
            warnings.push(format!(
                "{} represents {} same-name Skill variants; inspect the layout Finding before choosing content",
                found.name, found.variant_count
            ));
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
    Ok(json!({
        "snapshot_id": scan_id,
        "task": task,
        "retrieval_hints": retrieval_hints,
        "ranking_strategy": ranking_strategy,
        "matches": matches,
        "rescan_required": rescan_required,
        "warnings": warnings,
        "files_changed": false
    }))
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
    let directory_name = safe_skill_directory_name(&skill.name)?;
    let mut candidates = vec![
        state_dir
            .join("library")
            .join(&directory_name)
            .join("SKILL.md"),
    ];
    candidates.extend(
        scan.placements
            .iter()
            .filter(|placement| placement.skill_id == skill_id)
            .map(|placement| placement.entrypoint.clone()),
    );
    candidates.extend(
        scan.roots
            .iter()
            .filter(|root| {
                root.kind == RootKind::Skills && root.status == scan::RootStatus::Included
            })
            .map(|root| root.path.join(&directory_name).join("SKILL.md")),
    );
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
        if actual_id != skill.id || actual_fingerprint != skill.content_digest {
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

fn plan_command(store: &StateStore, state_dir: &Path) -> Result<Value> {
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

fn bounded_plan_impact(mut impact: Value) -> Value {
    let Some(object) = impact.as_object_mut() else {
        let items = impact.as_array().cloned().unwrap_or_default();
        return json!({
            "item_count": items.len(),
            "items": items.iter().take(10).collect::<Vec<_>>(),
            "items_truncated": items.len() > 10
        });
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
    selection_evidence: Option<Value>,
    uncertainty: Option<Value>,
}

fn roster_selection_evidence(
    recommendation: &crate::roster_recommendation::RosterRecommendation,
) -> (Value, Option<Value>) {
    let mut core_selection_count = 0_usize;
    let mut forced_core_count = 0_usize;
    let mut positive_signal_core_count = 0_usize;
    let mut stable_fallback_core_count = 0_usize;
    let mut fallback_dominated_agent_count = 0_usize;
    let mut reason_counts = std::collections::BTreeMap::<&str, usize>::new();
    let agents = recommendation
        .agents
        .iter()
        .map(|agent| {
            let mut forced = 0_usize;
            let mut positive = 0_usize;
            let mut fallback = 0_usize;
            for selection in &agent.core_selections {
                *reason_counts.entry(selection.reason).or_default() += 1;
                match selection.reason {
                    "protected_by_request" | "declared_core" | "skillroster_bootstrap" => {
                        forced += 1;
                    }
                    "stable_fallback" => fallback += 1,
                    _ => positive += 1,
                }
            }
            let core_count = agent.core_selections.len();
            let fallback_dominated = fallback > core_count / 2;
            core_selection_count += core_count;
            forced_core_count += forced;
            positive_signal_core_count += positive;
            stable_fallback_core_count += fallback;
            fallback_dominated_agent_count += usize::from(fallback_dominated);
            json!({
                "agent": agent.agent.id(),
                "core_selection_count": core_count,
                "forced_core_count": forced,
                "positive_signal_core_count": positive,
                "stable_fallback_core_count": fallback,
                "fallback_dominated": fallback_dominated
            })
        })
        .collect::<Vec<_>>();
    let fallback_dominated = fallback_dominated_agent_count > 0;
    let evidence = json!({
        "selection_policy": [
            "protected_by_request",
            "declared_core",
            "skillroster_bootstrap",
            "positive_usage_evidence",
            "stable_fallback"
        ],
        "core_selection_count": core_selection_count,
        "forced_core_count": forced_core_count,
        "positive_signal_core_count": positive_signal_core_count,
        "stable_fallback_core_count": stable_fallback_core_count,
        "fallback_dominated": fallback_dominated,
        "fallback_dominated_agent_count": fallback_dominated_agent_count,
        "reason_counts": reason_counts,
        "agents": agents,
        "absence_of_usage_evidence": "not_negative_evidence"
    });
    let uncertainty = fallback_dominated.then(|| {
        json!({
            "code": "fallback_dominated_core_selection",
            "review_required": true,
            "core_selection_count": core_selection_count,
            "stable_fallback_core_count": stable_fallback_core_count,
            "fallback_dominated_agent_count": fallback_dominated_agent_count,
            "absence_of_usage_evidence": "not_negative_evidence"
        })
    });
    (evidence, uncertainty)
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
    let (selection_evidence, uncertainty) = roster_selection_evidence(&recommendation);
    let supported =
        crate::roster_plan::exclude_unpreservable_demotions(scan, recommendation.changes.clone())?;
    if !supported.exclusions.is_empty() {
        if supported
            .exclusions
            .iter()
            .any(|exclusion| exclusion.reason == "non_agent_source_link_depends_on_removal")
        {
            bail!(
                "Finding {finding_id} is blocked because a non-Agent source link depends on a placement scheduled for removal; resolve the reported source dependency, rescan, and use the new Finding"
            );
        }
        bail!(
            "Finding {finding_id} is blocked by {} Roster changes without owned exact content; confirm the reported source roots, rescan, and use the new Finding",
            supported.exclusions.len()
        );
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
            selection_evidence: Some(selection_evidence),
            uncertainty,
        }),
    ))
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
            selection_evidence: None,
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
        if !placement.governable {
            bail!(
                "Placement {} is provider-managed and read-only; source updates are not allowed",
                request.placement_id
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

fn validated_physical_directory(placement: &scan::SkillPlacement) -> Result<PathBuf> {
    let expected = placement.physical_directory.as_ref().ok_or_else(|| {
        anyhow!(
            "Placement {} has no captured physical source; state may have drifted, run skillroster scan",
            placement.id
        )
    })?;
    let current_entrypoint = std::fs::canonicalize(&placement.entrypoint).map_err(|error| {
        anyhow!(
            "Placement {} physical source drifted; run skillroster scan: {error}",
            placement.id
        )
    })?;
    let current = current_entrypoint.parent().ok_or_else(|| {
        anyhow!(
            "Placement {} physical source drifted; run skillroster scan",
            placement.id
        )
    })?;
    if current != expected {
        bail!(
            "Placement {} physical source drifted from {} to {}; run skillroster scan",
            placement.id,
            expected.display(),
            current.display()
        );
    }
    Ok(expected.clone())
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
        if all_placements.iter().any(|placement| !placement.governable) {
            bail!(
                "Library change for {} includes provider-managed read-only placements",
                request.skill_id
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
                .entry(validated_physical_directory(placement)?)
                .or_default()
                .push(*placement);
        }
        let canonical_physical = validated_physical_directory(canonical)?;
        if !placement_owns_physical_source(canonical) {
            bail!("canonical placement must be the owned physical source directory");
        }
        let state_name = match request.requested_state {
            RequestedGovernanceState::Managed => "managed",
            RequestedGovernanceState::Hosted => "hosted",
        };
        let safe_name = safe_skill_directory_name(&skill.name)?;
        let library_path = library_root.join(safe_name);
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

        let mut relinked = 0_usize;
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
                "placement_count": all_placements.len(),
                "canonical_path": canonical.directory,
                "governance_state": "observed"
            },
            "after": {
                "governance_state": state_name,
                "canonical_path": link_source,
                "relinked_placement_count": relinked
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
) -> Result<Value> {
    if store.recovery_required()? {
        bail!("recovery is required before another Plan can be prepared");
    }
    let (scan_id, scan) = latest_scan(store)?;
    let (input, finding_provenance) = if matches!(origin, PlanOrigin::Agent) {
        let (input, roster_provenance) =
            expand_finding_roster_changes(store, input, &scan_id, &scan)?;
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
    let selection_evidence = finding_provenance
        .as_ref()
        .and_then(|provenance| provenance.selection_evidence.clone());
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
            "contains": [
                "operations",
                "roster_changes",
                "source_updates",
                "library_changes",
                "before_state"
            ]
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
    if let Some(selection_evidence) = selection_evidence {
        summary_object.insert("selection_evidence".into(), selection_evidence);
    }
    if let Some(uncertainty) = uncertainty {
        summary_object.insert("uncertainty".into(), uncertainty);
    }
    store.save_plan(&plan_record(
        &prepared,
        input,
        roster_before.clone(),
        library_before.clone(),
        report_id,
        finding_ids.clone(),
        summary.clone(),
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
        bail!("Plan {id} is stale; a newer Snapshot exists");
    }
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

const LEGACY_BOOTSTRAPS: &[(&str, &str)] = &[
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
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BootstrapContentStatus {
    Current,
    OfficialOutdated(&'static str),
    Modified,
}

impl BootstrapContentStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::OfficialOutdated(_) => "official_outdated",
            Self::Modified => "modified",
        }
    }

    fn installed_version(self) -> Option<&'static str> {
        match self {
            Self::Current => Some(env!("CARGO_PKG_VERSION")),
            Self::OfficialOutdated(version) => Some(version),
            Self::Modified => None,
        }
    }
}

fn bootstrap_content_status(digest: &str, current_digest: &str) -> BootstrapContentStatus {
    if digest == current_digest {
        BootstrapContentStatus::Current
    } else if let Some((version, _)) = LEGACY_BOOTSTRAPS
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
    let Some(snapshot) = store.latest_completed_scan()? else {
        return Ok(json!({
            "detected_agents": [],
            "targets": [],
            "plan_id": Value::Null,
            "state": "scan_required",
            "bootstrap_skill": "skillroster",
            "bootstrap_version": env!("CARGO_PKG_VERSION"),
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
    let current_content =
        normalized_bootstrap_content(include_str!("../skill/skillroster/SKILL.md"));
    let current_digest = content_digest(current_content.as_bytes());
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
    let mut planned_entrypoints = BTreeSet::new();
    for roots in harness::known_agent_roots(home) {
        for root in roots.skill_roots.into_iter().filter(|path| path.is_dir()) {
            let directory = root.join("skillroster");
            let entrypoint = directory.join("SKILL.md");
            let physical_root = std::fs::canonicalize(&root).with_context(|| {
                format!("failed to resolve Agent Skill root {}", root.display())
            })?;
            let physical_directory = physical_root.join("skillroster");
            let physical_entrypoint = physical_directory.join("SKILL.md");
            physical_targets.insert(physical_entrypoint.clone());
            detected.push(json!({"agent": roots.agent.id(), "target": entrypoint}));
            let directory_is_unsupported = match std::fs::symlink_metadata(&directory) {
                Ok(metadata) => metadata.file_type().is_symlink() || !metadata.file_type().is_dir(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => true,
            };
            let (status, installed_version) = match std::fs::symlink_metadata(&entrypoint) {
                Ok(metadata)
                    if directory_is_unsupported
                        || metadata.file_type().is_symlink()
                        || !metadata.file_type().is_file() =>
                {
                    unsupported_count += 1;
                    ("unsupported", None)
                }
                Ok(_) => match std::fs::read(&entrypoint) {
                    Ok(content) => {
                        let digest = bootstrap_content_digest(&content);
                        let content_status = bootstrap_content_status(&digest, &current_digest);
                        match content_status {
                            BootstrapContentStatus::Current => current_count += 1,
                            BootstrapContentStatus::OfficialOutdated(_) => {
                                outdated_count += 1;
                                replace_count += 1;
                                if planned_entrypoints.insert(physical_entrypoint.clone()) {
                                    operations.push(json!({
                                        "kind": "replace_file",
                                        "target": entrypoint,
                                        "content": &current_content,
                                        "expected_fingerprint": change::fingerprint(&entrypoint)?
                                    }));
                                }
                            }
                            BootstrapContentStatus::Modified => {
                                modified_count += 1;
                                if matches!(
                                    modified_choice,
                                    Some(ModifiedBootstrapChoice::AdoptCurrent)
                                ) {
                                    replace_count += 1;
                                    if planned_entrypoints.insert(physical_entrypoint.clone()) {
                                        operations.push(json!({
                                            "kind": "replace_file",
                                            "target": entrypoint,
                                            "content": &current_content,
                                            "expected_fingerprint": change::fingerprint(&entrypoint)?
                                        }));
                                    }
                                }
                            }
                        }
                        (content_status.as_str(), content_status.installed_version())
                    }
                    Err(_) => {
                        unsupported_count += 1;
                        ("unsupported", None)
                    }
                },
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && !directory_is_unsupported =>
                {
                    missing_count += 1;
                    if !directory.exists() && planned_directories.insert(physical_directory.clone())
                    {
                        operations.push(json!({
                            "kind": "create_directory",
                            "target": directory,
                            "expected_fingerprint": "missing"
                        }));
                    }
                    if planned_entrypoints.insert(physical_entrypoint.clone()) {
                        operations.push(json!({
                            "kind": "write_file",
                            "target": entrypoint,
                            "content": &current_content,
                            "expected_fingerprint": "missing"
                        }));
                    }
                    ("missing", None)
                }
                Err(_) => {
                    unsupported_count += 1;
                    ("unsupported", None)
                }
            };
            targets.push(json!({
                "agent": roots.agent.id(),
                "target": entrypoint,
                "physical_target": physical_entrypoint,
                "status": status,
                "installed_version": installed_version
            }));
        }
    }
    let base = json!({
        "detected_agents": detected,
        "targets": targets,
        "bootstrap_skill": "skillroster",
        "bootstrap_version": env!("CARGO_PKG_VERSION"),
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
        let identity_key = match (&skill.metadata.source, &declared_revision) {
            (Some(source), Some(revision)) => format!("source:{source}@{revision}"),
            _ => format!("content:{}", skill.content_digest),
        };
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
                "governable": placement.governable,
                "provider": placement.provider,
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
        let reference = format!(
            "usage:{}:{}:{:?}:{}",
            usage.agent.id(),
            usage.skill_id,
            usage.stage,
            usage.source_path_digest
        );
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
            occurred_at: usage.last_seen_unix.unwrap_or_default() as i64,
            outcome: None,
        })?;
    }
    Ok(())
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
        if !scan
            .placements
            .iter()
            .any(|placement| placement.skill_id == change.skill_id && placement.governable)
        {
            bail!("Skill {skill_id} is provider-managed and read-only");
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

fn plan_record(
    prepared: &PreparedPlan,
    raw: Value,
    roster_before: Vec<Value>,
    library_before: Vec<Value>,
    report_id: Option<ReportId>,
    finding_ids: Vec<FindingId>,
    summary: Value,
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
            "summary": summary
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
    use tempfile::TempDir;

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
                fallback_core_count: 1,
                core_selections: vec![
                    crate::roster_recommendation::CoreSelection {
                        skill_id: "skill_forced".into(),
                        name: "forced".into(),
                        reason: "protected_by_request",
                    },
                    crate::roster_recommendation::CoreSelection {
                        skill_id: "skill_observed".into(),
                        name: "observed".into(),
                        reason: "observed_loaded",
                    },
                    crate::roster_recommendation::CoreSelection {
                        skill_id: "skill_fallback".into(),
                        name: "fallback".into(),
                        reason: "stable_fallback",
                    },
                ],
            }],
        };

        let (evidence, uncertainty) = roster_selection_evidence(&recommendation);

        assert_eq!(evidence["core_selection_count"], 3);
        assert_eq!(evidence["forced_core_count"], 1);
        assert_eq!(evidence["positive_signal_core_count"], 1);
        assert_eq!(evidence["stable_fallback_core_count"], 1);
        assert_eq!(evidence["fallback_dominated"], false);
        assert!(uncertainty.is_none());
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
            link_target: None,
            link_status: scan::LinkStatus::NotLink,
            default_exposed: governable,
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
        assert_eq!(planning["reason"], "external_observed_placements");
        assert_eq!(planning["protected_placement_count"], 1);
    }

    #[test]
    fn bootstrap_digest_classification_distinguishes_release_content_from_local_edits() {
        let current = bootstrap_content_digest(include_bytes!("../skill/skillroster/SKILL.md"));
        assert_eq!(
            bootstrap_content_status(&current, &current),
            BootstrapContentStatus::Current
        );
        assert!(
            !LEGACY_BOOTSTRAPS
                .iter()
                .any(|(_, digest)| *digest == current)
        );
        for (version, digest) in LEGACY_BOOTSTRAPS {
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

        assert!(lifecycle_purge_command(&store, &state_dir, Some(0), true).is_err());
        let (_, payload): (ScanId, Value) = store.latest_scan_payload().unwrap().unwrap();
        assert_eq!(payload["usage"].as_array().unwrap().len(), 1);
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
