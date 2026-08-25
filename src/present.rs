use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::io::{IsTerminal, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const PROGRESS_BEGIN: &str = "\r\x1b[2K\x1b[?25l";
const PROGRESS_END: &str = "\r\x1b[2K\x1b[?25h";
static PROGRESS_ACTIVE: AtomicBool = AtomicBool::new(false);
static INTERRUPT_HANDLER: OnceLock<bool> = OnceLock::new();

/// Owns the one-line TTY progress state and restores the terminal on every exit path.
pub struct ProgressGuard {
    active: bool,
    stop: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl ProgressGuard {
    pub fn start(command: &str, json: bool) -> Self {
        let progress = progress_copy(command);
        let tty = std::io::stderr().is_terminal();
        let styled = std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").map_or(true, |term| term != "dumb");
        let interactive = !json && tty && progress.is_some();
        if !interactive {
            return Self {
                active: false,
                stop: None,
                worker: None,
            };
        }
        let (stage, interrupted) = progress.expect("progress checked above");
        let handler_ready = *INTERRUPT_HANDLER.get_or_init(|| {
            ctrlc::set_handler(move || {
                if PROGRESS_ACTIVE.swap(false, Ordering::SeqCst) {
                    restore_terminal();
                }
                eprintln!("{interrupted}");
                std::process::exit(130);
            })
            .is_ok()
        });
        let active = styled && handler_ready;
        let (stop, worker) = if active {
            PROGRESS_ACTIVE.store(true, Ordering::SeqCst);
            let (stop, stopped) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                let started = Instant::now();
                let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                for tick in 0_usize.. {
                    if !PROGRESS_ACTIVE.load(Ordering::SeqCst) {
                        break;
                    }
                    let elapsed = started.elapsed().as_secs_f32();
                    let _ = write!(
                        std::io::stderr(),
                        "{PROGRESS_BEGIN}{} {stage} · {elapsed:.1}s",
                        frames[tick % frames.len()]
                    );
                    let _ = std::io::stderr().flush();
                    match stopped.recv_timeout(Duration::from_millis(80)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            });
            (Some(stop), Some(worker))
        } else {
            (None, None)
        };
        Self {
            active,
            stop,
            worker,
        }
    }

    pub fn finish(mut self) {
        self.restore();
    }

    fn restore(&mut self) {
        if self.active {
            self.active = false;
            PROGRESS_ACTIVE.store(false, Ordering::SeqCst);
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            restore_terminal();
        }
    }
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn progress_copy(command: &str) -> Option<(&'static str, &'static str)> {
    match command {
        "scan" => Some((
            "Scanning configured Agent roots",
            "Interrupted · scan incomplete · no Agent files changed.",
        )),
        "apply" => Some((
            "Checking Plan and filesystem preconditions",
            "Interrupted · Apply completion unknown · files may have changed under a durable journal; run skillroster status before retrying.",
        )),
        "undo" => Some((
            "Checking Receipt and filesystem preconditions",
            "Interrupted · Undo completion unknown · files may have changed under a durable journal; run skillroster status before retrying.",
        )),
        "setup" => Some((
            "Inspecting bootstrap Skill targets",
            "Interrupted · setup incomplete · no Agent files changed.",
        )),
        _ => None,
    }
}

fn restore_terminal() {
    let _ = std::io::stderr().write_all(PROGRESS_END.as_bytes());
    let _ = std::io::stderr().flush();
}

pub fn human(command: &str, result: &Value) -> String {
    let options = RenderOptions {
        width: terminal_width(),
        styled: std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").map_or(true, |term| term != "dumb"),
    };
    render(command, result, options)
}

#[derive(Clone, Copy)]
struct RenderOptions {
    width: usize,
    styled: bool,
}

fn render(command: &str, result: &Value, options: RenderOptions) -> String {
    let styled = options.styled;
    let header = format!("SkillRoster · {}", title(command));
    let mut lines = vec![
        if styled {
            format!("\u{1b}[1;36m{header}\u{1b}[0m")
        } else {
            header
        },
        String::new(),
    ];
    match command {
        "status" => status(result, &mut lines, options.width),
        "scan" => scan(result, &mut lines),
        "report" => report(result, &mut lines, options.width),
        "find" => find(result, &mut lines, options.width),
        "plan" => plan(result, &mut lines, options.width),
        "apply" | "undo" => mutation(result, &mut lines),
        "lifecycle" => lifecycle(result, &mut lines),
        "source-root" => source_root(result, &mut lines),
        "setup" => setup(result, &mut lines, options.width),
        _ => home(result, &mut lines),
    }
    if styled {
        for line in &mut lines {
            if line.starts_with("Read-only") || line.starts_with("Preview only") {
                *line = format!("\u{1b}[33m{line}\u{1b}[0m");
            } else if line.starts_with("Changed") {
                *line = format!("\u{1b}[32m{line}\u{1b}[0m");
            }
        }
    }
    lines.join("\n")
}

pub fn error_human(error: &dyn Display) -> String {
    format!("SkillRoster · Error\n\nERR  {error}\n\nNo files changed.")
}

pub fn blocked_roster_plan(details: &Value) -> String {
    let mut lines = vec!["SkillRoster · Plan".into(), String::new()];
    blocked_roster_plan_lines(details, &mut lines, terminal_width());
    lines.join("\n")
}

fn blocked_roster_plan_lines(details: &Value, lines: &mut Vec<String>, width: usize) {
    fact(lines, "Status", "blocked");
    let decision = match text(details, "decision").as_str() {
        "confirm_trusted_source_roots" => "confirm exact local reads".to_owned(),
        other => other.replace('_', " "),
    };
    fact(lines, "Decision", decision);
    fact(lines, "Core budget", text(details, "requested_core_budget"));
    fact(
        lines,
        "Blocked changes",
        details
            .get("blocked_change_count")
            .map(Value::to_string)
            .unwrap_or_else(|| "none".into()),
    );
    lines.push(String::new());
    lines.push("  Skills needing a source decision".into());
    let blockers = details
        .get("blocked_changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(5);
    for blocker in blockers {
        let agent = blocker
            .get("agent")
            .and_then(Value::as_str)
            .unwrap_or("agent");
        let name = blocker
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let label = format!("  {agent:<12} {name}");
        lines.push(middle_truncate(&label, width));
        if let Some(target) = blocker
            .get("observed_source_target")
            .and_then(Value::as_str)
        {
            lines.push(format!(
                "    {}",
                middle_truncate(target, width.saturating_sub(4))
            ));
        }
    }
    let more_blockers = details["blocked_changes_truncated"].as_bool() == Some(true)
        || details
            .get("blocked_changes")
            .and_then(Value::as_array)
            .is_some_and(|items| items.len() > 5);
    let detail_path = details.pointer("/detail/path").and_then(Value::as_str);
    if more_blockers && detail_path.is_none() {
        lines.push("  Inspect JSON error.details for remaining blockers".into());
    }
    lines.push(String::new());
    lines.push("  Reviewed source directories".into());
    let roots = details
        .get("source_roots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(5);
    let mut shown = 0;
    for root in roots {
        shown += 1;
        lines.push(format!(
            "  --source-root {}",
            middle_truncate(root, width.saturating_sub(18))
        ));
    }
    if shown == 0 {
        lines.push("  none disclosed".into());
    }
    let more_roots = details["source_roots_truncated"].as_bool() == Some(true)
        || details
            .get("source_roots")
            .and_then(Value::as_array)
            .is_some_and(|items| items.len() > 5);
    if let Some(path) = detail_path.filter(|_| more_blockers || more_roots) {
        lines.push(format!(
            "  Read {}",
            middle_truncate(path, width.saturating_sub(8))
        ));
    } else if more_roots {
        lines.push("  Inspect JSON error.details for remaining source roots".into());
    }
    lines.extend(summary(
        "Blocked · no automatic change is supported",
        "Permit exact local reads, then rescan",
    ));
}

fn title(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn fact(lines: &mut Vec<String>, label: &str, value: impl std::fmt::Display) {
    lines.push(format!("  {:<22} {value}", label));
}

fn fact_items(lines: &mut Vec<String>, label: &str, items: Vec<String>, width: usize) {
    if items.is_empty() {
        fact(lines, label, "none");
        return;
    }
    let value_budget = width.saturating_sub(25).max(1);
    let mut current = String::new();
    let mut first = true;
    for item in items {
        let candidate = if current.is_empty() {
            item.clone()
        } else {
            format!("{current} · {item}")
        };
        if !current.is_empty() && display_width(&candidate) > value_budget {
            fact(lines, if first { label } else { "" }, current);
            first = false;
            current = item;
        } else {
            current = candidate;
        }
    }
    if display_width(&current) <= value_budget {
        fact(lines, if first { label } else { "" }, current);
    } else {
        fact(lines, if first { label } else { "" }, "");
        lines.push(format!("    {current}"));
    }
}

fn status(value: &Value, lines: &mut Vec<String>, width: usize) {
    fact(lines, "Database", text(value, "database_path"));
    fact(lines, "Schema", text(value, "schema_version"));
    fact(lines, "Latest Snapshot", text(value, "latest_snapshot_id"));
    fact(
        lines,
        "Snapshot age",
        age(value.get("latest_snapshot_at").and_then(Value::as_i64)),
    );
    fact(lines, "Pending Plans", text(value, "pending_plan_count"));
    let pending_plan = value
        .get("pending_plans")
        .and_then(Value::as_array)
        .and_then(|plans| plans.first());
    if let Some(plan) = pending_plan {
        fact_items(lines, "Review Plan", vec![text(plan, "plan_id")], width);
        fact_items(
            lines,
            "Plan state",
            vec![
                text(plan, "status"),
                age(plan.get("created_at").and_then(Value::as_i64)),
            ],
            width,
        );
    }
    if let Some(receipt) = value.get("last_receipt").filter(|item| !item.is_null()) {
        fact(
            lines,
            "Last Receipt",
            format!(
                "{} · {} · {}",
                text(receipt, "receipt_id"),
                text(receipt, "status"),
                age(receipt.get("completed_at").and_then(Value::as_i64))
            ),
        );
    } else {
        fact(lines, "Last Receipt", "none");
    }
    fact(lines, "Recovery", text(value, "recovery_state"));
    let next = if value["recovery_state"] == "required" {
        "Next: skillroster lifecycle recovery".to_owned()
    } else if value.get("latest_snapshot_id").is_none_or(Value::is_null) {
        "Next: skillroster scan".to_owned()
    } else if pending_plan.is_some() {
        "Next: inspect the Review Plan above".to_owned()
    } else {
        "Next: scan only when fresher inventory is needed".to_owned()
    };
    lines.push(String::new());
    lines.push("Read-only · no Agent files changed".into());
    if display_width(&next) > width {
        lines.push(middle_truncate(&next, width));
    } else {
        lines.push(next);
    }
}

fn scan(value: &Value, lines: &mut Vec<String>) {
    fact(lines, "Agents checked", text(value, "agents_checked"));
    fact(lines, "Independent Skills", text(value, "skill_count"));
    fact(lines, "Placements", text(value, "placement_count"));
    fact(lines, "Snapshot", text(value, "snapshot_id"));
    lines.extend(summary(
        "Read-only · no Agent files changed",
        "Next: skillroster report",
    ));
}

fn report(value: &Value, lines: &mut Vec<String>, width: usize) {
    if value.get("id").and_then(Value::as_str).is_some()
        && value.get("title").and_then(Value::as_str).is_some()
    {
        finding_report(value, lines, width);
        return;
    }
    if value.get("view").and_then(Value::as_str) == Some("findings") {
        finding_list(value, lines, width);
        return;
    }
    fact(lines, "Independent Skills", text(value, "skill_count"));
    fact(lines, "Placements", text(value, "placement_count"));
    fact(lines, "Default exposure", text(value, "default_exposure"));
    fact(
        lines,
        "Observed-use Agents",
        text(value, "observed_use_agent_count"),
    );
    if let Some(coverage) = value.get("session_coverage") {
        fact(
            lines,
            "Session sample",
            format!(
                "sampled {}/{} · complete {}/{}",
                text(coverage, "sampled_agents"),
                text(coverage, "supported_agents"),
                text(coverage, "complete_agents"),
                text(coverage, "supported_agents")
            ),
        );
        fact(
            lines,
            "Coverage limits",
            format!(
                "limited {}/{} · missing {}/{}",
                text(coverage, "limited_agents"),
                text(coverage, "supported_agents"),
                text(coverage, "missing_root_agents"),
                text(coverage, "supported_agents")
            ),
        );
        fact(
            lines,
            "Inaccessible",
            format!(
                "{}/{}",
                text(coverage, "inaccessible_agents"),
                text(coverage, "supported_agents")
            ),
        );
    } else {
        fact(
            lines,
            "Reliable coverage",
            format!("{}/8", text(value, "coverage_reliable_agent_count")),
        );
    }
    lines.push(String::new());
    lines.push("  Top Findings".into());
    if let Some(findings) = value.get("findings").and_then(Value::as_array) {
        for finding in findings.iter().take(3) {
            lines.push(format!(
                "  {:<7} {:<10} {}",
                text(finding, "severity"),
                text(finding, "category"),
                text(finding, "title")
            ));
        }
        if findings.is_empty() {
            lines.push("  none".into());
        }
    }
    if let Some(rollups) = value.get("finding_rollups").and_then(Value::as_array) {
        if !rollups.is_empty() {
            const VISIBLE_ROLLUPS: usize = 5;
            lines.push(String::new());
            lines.push("  Finding groups".into());
            for rollup in rollups.iter().take(VISIBLE_ROLLUPS) {
                let prefix = format!(
                    "  {} × {} Skills · {} placements · ",
                    text(rollup, "finding_count"),
                    text(rollup, "affected_skill_count"),
                    text(rollup, "affected_placement_count")
                );
                lines.push(format!(
                    "{prefix}{}",
                    middle_truncate(
                        &text(rollup, "title"),
                        width.saturating_sub(display_width(&prefix))
                    )
                ));
            }
            if rollups.len() > VISIBLE_ROLLUPS {
                lines.push(format!(
                    "  … {} more groups in JSON",
                    rollups.len() - VISIBLE_ROLLUPS
                ));
            }
        }
    }
    if let Some(counts) = value.get("category_counts").and_then(Value::as_object) {
        lines.push(String::new());
        lines.push("  Category totals".into());
        for (category, count) in counts {
            lines.push(format!("  {category:<12} {count}"));
        }
    }
    lines.extend(summary(
        "Read-only · no Agent files changed",
        "Review evidence before planning changes",
    ));
}

fn finding_list(value: &Value, lines: &mut Vec<String>, width: usize) {
    let offset = value["page"]["offset"].as_u64().unwrap_or_default();
    let returned = value["page"]["returned"].as_u64().unwrap_or_default();
    let total = value["page"]["total"].as_u64().unwrap_or_default();
    let range = if returned == 0 {
        format!("0 of {total} matching")
    } else {
        format!("{}–{} of {total} matching", offset + 1, offset + returned)
    };
    fact(lines, "Finding page", range);
    fact(lines, "All Findings", text(value, "finding_count"));
    let category = value["filters"]["category"].as_str();
    let severity = value["filters"]["severity"].as_str();
    if category.is_some() || severity.is_some() {
        fact(
            lines,
            "Filters",
            format!(
                "category {} · severity {}",
                category.unwrap_or("any"),
                severity.unwrap_or("any")
            ),
        );
    }
    lines.push(String::new());
    if let Some(items) = value.get("items").and_then(Value::as_array) {
        for (index, finding) in items.iter().enumerate() {
            let prefix = format!(
                "  {:>3}. {:<6} {:<10} ",
                offset as usize + index + 1,
                text(finding, "severity"),
                text(finding, "category")
            );
            lines.push(format!(
                "{prefix}{}",
                middle_truncate(
                    &text(finding, "title"),
                    width.saturating_sub(display_width(&prefix))
                )
            ));
            let detail_prefix = "       ";
            let detail = format!(
                "{} · {} Skills · {} placements",
                text(finding, "id"),
                text(finding, "affected_skill_count"),
                text(finding, "affected_placement_count")
            );
            lines.push(format!(
                "{detail_prefix}{}",
                middle_truncate(&detail, width.saturating_sub(display_width(detail_prefix)))
            ));
        }
        if items.is_empty() {
            lines.push("  none".into());
        }
    }
    let next = value["page"]["next_offset"].as_u64().map_or_else(
        || "End of matching Findings".into(),
        |next| format!("Continue with --offset {next}"),
    );
    lines.extend(summary("Read-only · no Agent files changed", &next));
}

fn finding_report(value: &Value, lines: &mut Vec<String>, width: usize) {
    fact(
        lines,
        "Finding",
        middle_truncate(&text(value, "id"), width.saturating_sub(25)),
    );
    fact(
        lines,
        "Issue",
        middle_truncate(&text(value, "title"), width.saturating_sub(25)),
    );
    fact(
        lines,
        "Severity",
        format!("{} · {}", text(value, "severity"), text(value, "category")),
    );
    fact(lines, "Evidence quality", text(value, "evidence_quality"));
    fact(
        lines,
        "Affected",
        format!(
            "{} Skills · {} placements",
            value
                .pointer("/impact/affected_skill_count")
                .map_or_else(|| "none".into(), Value::to_string),
            value
                .pointer("/impact/affected_placement_count")
                .map_or_else(|| "none".into(), Value::to_string)
        ),
    );
    fact(
        lines,
        "Detail",
        value
            .pointer("/detail/mode")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
    );
    if let Some(overview) = value.get("usage_overview") {
        usage_finding_overview(overview, lines, width);
    } else {
        lines.push(String::new());
        lines.push(format!(
            "  {}",
            middle_truncate(&text(value, "summary"), width.saturating_sub(2))
        ));
        finding_evidence_paths(value, lines, width);
    }

    let trust_resolution =
        if value["resolution"]["decision"].as_str() == Some("confirm_trusted_source_roots") {
            &value["resolution"]
        } else {
            &value["planning"]
        };
    if trust_resolution["decision"].as_str() == Some("confirm_trusted_source_roots") {
        lines.push(String::new());
        lines.push("  Observed link targets".into());
        if let Some(targets) = trust_resolution["observed_link_targets"].as_array() {
            for target in targets.iter().filter_map(Value::as_str).take(5) {
                lines.push(format!(
                    "  {}",
                    middle_truncate(target, width.saturating_sub(2))
                ));
            }
        }
        lines.extend(summary(
            "Blocked · no automatic change is supported",
            "Permit exact local reads before rescanning",
        ));
    } else if value["resolution"]["decision"].as_str() == Some("choose_same_name_variant") {
        lines.push(String::new());
        lines.push("  Variants requiring a choice".into());
        if let Some(variants) = value["resolution"]["variants"].as_array() {
            for variant in variants.iter().take(5) {
                let digest = variant["content_digests"]
                    .as_array()
                    .and_then(|digests| digests.first())
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let agents = variant["agents"]
                    .as_array()
                    .map(|agents| {
                        agents
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|agents| !agents.is_empty())
                    .unwrap_or_else(|| "source-only".into());
                let label = format!("  {} · {agents}", take_width(digest, 12, false));
                lines.push(middle_truncate(&label, width));
                if let Some(path) = variant["paths"]
                    .as_array()
                    .and_then(|paths| paths.first())
                    .and_then(Value::as_str)
                {
                    lines.push(format!(
                        "    {}",
                        middle_truncate(path, width.saturating_sub(4))
                    ));
                }
                if let Some(additional) = variant["paths"]
                    .as_array()
                    .map(|paths| paths.len().saturating_sub(1))
                    .filter(|count| *count > 0)
                {
                    lines.push(format!("    + {additional} additional placements"));
                }
            }
        }
        if value["resolution"]["variants_truncated"].as_bool() == Some(true) {
            lines.push("  Additional variants require --full pagination".into());
        }
        lines.extend(summary(
            "Blocked · choose one canonical variant first",
            "Compare the reported paths before authoring a Plan",
        ));
    } else if value["planning"]["decision"].as_str() == Some("resolve_source_dependency") {
        lines.push(String::new());
        lines.push("  Dependent source link targets".into());
        if let Some(targets) = value["planning"]["dependent_link_targets"].as_array() {
            for target in targets.iter().filter_map(Value::as_str).take(5) {
                lines.push(format!(
                    "  {}",
                    middle_truncate(target, width.saturating_sub(2))
                ));
            }
        }
        lines.extend(summary(
            "Blocked · no automatic change is supported",
            "Move or retarget the dependent source link before rescanning",
        ));
    } else {
        lines.extend(summary(
            "Read-only · no Agent files changed",
            "Use --full only when exact complete records are needed",
        ));
    }
}

fn finding_evidence_paths(value: &Value, lines: &mut Vec<String>, width: usize) {
    let evidence_rows = value
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| value.get("placements").and_then(Value::as_array));
    let Some(rows) = evidence_rows.filter(|rows| !rows.is_empty()) else {
        return;
    };
    let path_count = rows
        .iter()
        .filter(|row| row.get("path").and_then(Value::as_str).is_some())
        .count();
    let rows_with_paths = rows
        .iter()
        .filter(|row| row.get("path").and_then(Value::as_str).is_some())
        .take(10)
        .collect::<Vec<_>>();
    if !rows_with_paths.is_empty() {
        lines.push(String::new());
        lines.push("  Evidence paths".into());
    }
    for row in rows_with_paths {
        let agent = row
            .get("facts")
            .and_then(|facts| facts.get("agent"))
            .and_then(Value::as_str)
            .or_else(|| row.get("agent").and_then(Value::as_str))
            .unwrap_or("source");
        let prefix = format!("  {agent:<12} ");
        let path = middle_truncate(
            &text(row, "path"),
            width.saturating_sub(display_width(&prefix)),
        );
        lines.push(format!("{prefix}{path}"));
    }
    if path_count > 10 {
        lines.push(format!("  + {} more on this page", path_count - 10));
    }
}

fn usage_finding_overview(overview: &Value, lines: &mut Vec<String>, width: usize) {
    lines.push(String::new());
    lines.push("  Stage evidence".into());
    if let Some(stages) = overview.get("stages").and_then(Value::as_array) {
        for stage in stages {
            let label = title(&text(stage, "stage"));
            let last_seen = stage.get("last_seen_unix").and_then(Value::as_i64);
            let mut facts = vec![
                format!("{} {}", text(stage, "count"), text(stage, "unit")),
                text(stage, "quality"),
            ];
            if last_seen.is_some() {
                facts.push(age(last_seen));
            }
            fact_items(lines, &label, facts, width);
        }
    }

    if let Some(coverage) = overview.get("coverage") {
        let supported = coverage["supported_agent_count"].as_u64().unwrap_or(0);
        lines.push(String::new());
        lines.push("  Session coverage".into());
        fact_items(
            lines,
            "Roots / sampled",
            vec![
                ratio(coverage, "roots_present_agent_count", supported),
                ratio(coverage, "sampled_agent_count", supported),
            ],
            width,
        );
        fact_items(
            lines,
            "Complete / limited",
            vec![
                ratio(coverage, "complete_agent_count", supported),
                ratio(coverage, "limited_agent_count", supported),
            ],
            width,
        );
        fact_items(
            lines,
            "Missing / inaccessible",
            vec![
                ratio(coverage, "missing_agent_count", supported),
                ratio(coverage, "inaccessible_agent_count", supported),
            ],
            width,
        );
        fact_items(
            lines,
            "File discovery",
            vec![
                format!("{} discovered", text(coverage, "files_discovered")),
                format!("{} observed", text(coverage, "files_observed")),
            ],
            width,
        );
        fact_items(
            lines,
            "File handling",
            vec![
                format!("{} partial", text(coverage, "files_partially_observed")),
                format!("{} skipped", text(coverage, "files_skipped")),
            ],
            width,
        );
        fact_items(
            lines,
            "Observed volume",
            vec![
                readable_bytes(coverage["bytes_observed"].as_u64().unwrap_or(0)),
                format!("{} lines", text(coverage, "lines_observed")),
            ],
            width,
        );
        fact_items(
            lines,
            "Sample boundary",
            vec![
                if coverage["truncated"].as_bool() == Some(true) {
                    "content bounded".into()
                } else {
                    "content complete".into()
                },
                if coverage["discovery_truncated"].as_bool() == Some(true) {
                    "discovery bounded".into()
                } else {
                    "discovery complete".into()
                },
            ],
            width,
        );
    }

    lines.push(String::new());
    lines.push("  Observed Skills".into());
    let observed = overview
        .get("observed_skills")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if observed.is_empty() {
        lines.push("  none beyond default exposure".into());
    } else {
        for signal in observed {
            let ambiguous_name = observed
                .iter()
                .filter(|candidate| {
                    candidate["agent"] == signal["agent"]
                        && candidate["skill_name"] == signal["skill_name"]
                        && candidate["stage"] == signal["stage"]
                })
                .count()
                > 1;
            let mut suffix = format!(
                " · {} ×{}",
                text(signal, "stage"),
                text(signal, "event_count")
            );
            if ambiguous_name {
                suffix.push_str(&format!(
                    " · {}",
                    take_width(&text(signal, "skill_id"), 12, true)
                ));
            }
            let detail_budget = width.saturating_sub(25);
            let skill_name = middle_truncate(
                &text(signal, "skill_name"),
                detail_budget.saturating_sub(display_width(&suffix)),
            );
            let detail = format!("{skill_name}{suffix}");
            fact(lines, &text(signal, "agent"), detail);
        }
        let total = overview["observed_signal_count"].as_u64().unwrap_or(0) as usize;
        if total > observed.len() {
            lines.push(format!(
                "  + {} more observed Skill signals",
                total - observed.len()
            ));
        }
    }
}

fn ratio(value: &Value, key: &str, denominator: u64) -> String {
    format!("{}/{}", value[key].as_u64().unwrap_or(0), denominator)
}

fn readable_bytes(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1} MB", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1} KB", value as f64 / 1_000.0)
    } else {
        format!("{value} bytes")
    }
}

fn find(value: &Value, lines: &mut Vec<String>, width: usize) {
    if let Some(items) = value.get("matches").and_then(Value::as_array) {
        for (index, item) in items.iter().enumerate() {
            let path = item
                .get("paths")
                .and_then(Value::as_array)
                .and_then(|paths| paths.first())
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let source = text(item, "source");
            let roster = text(item, "roster_state");
            let providers = item
                .get("providers")
                .and_then(Value::as_array)
                .map(|providers| {
                    providers
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|providers| !providers.is_empty());
            let reasons = item
                .get("match_reasons")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .take(3)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "not provided".into());
            lines.push(format!("  {}. {}", index + 1, text(item, "name")));
            if let Some(providers) = providers {
                let management = if item
                    .get("governable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    "mixed ownership"
                } else {
                    "provider-managed · read-only"
                };
                lines.push(format!("     Codex plugin {providers} · {management}"));
            } else {
                lines.push(format!("     roster {roster} · source {source}"));
            }
            let has_variants = item
                .get("variant_count")
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 1);
            let variant_details = item
                .get("variants")
                .and_then(Value::as_array)
                .filter(|variants| !variants.is_empty());
            if has_variants {
                let next = item.get("variant_finding");
                let resolution = next
                    .and_then(|value| value.get("finding_id"))
                    .and_then(Value::as_str)
                    .map(|finding_id| format!("Finding {finding_id}"))
                    .or_else(|| {
                        match next
                            .and_then(|value| value.get("state"))
                            .and_then(Value::as_str)
                        {
                            Some("rescan_required") => Some("refresh Snapshot first".into()),
                            Some("source_confirmation_required") => {
                                Some("confirm exact source root first".into())
                            }
                            Some("report_required") => Some("current Report required".into()),
                            _ => None,
                        }
                    })
                    .unwrap_or_else(|| "inspect layout Finding".into());
                lines.push(format!(
                    "     variants {} · {resolution}",
                    text(item, "variant_count"),
                ));
            }
            lines.push(format!("     reasons {reasons}"));
            if let Some(variants) = variant_details {
                for variant in variants.iter().take(3) {
                    let providers = variant
                        .get("providers")
                        .and_then(Value::as_array)
                        .map(|providers| {
                            providers
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .filter(|providers| !providers.is_empty());
                    let label = if let Some(providers) = providers {
                        let management = if variant
                            .get("governable")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        {
                            "mixed"
                        } else {
                            "read-only"
                        };
                        format!("plugin {providers} · {management}")
                    } else {
                        let agents = variant
                            .get("agents")
                            .and_then(Value::as_array)
                            .map(|agents| {
                                agents
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .filter(|agents| !agents.is_empty())
                            .unwrap_or_else(|| "source".into());
                        format!("{agents} · roster {}", text(variant, "roster_state"))
                    };
                    let variant_path = variant
                        .get("paths")
                        .and_then(Value::as_array)
                        .and_then(|paths| paths.first())
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    lines.push(format!(
                        "       {}",
                        middle_truncate(&label, width.saturating_sub(7))
                    ));
                    lines.push(format!(
                        "       {}",
                        middle_truncate(variant_path, width.saturating_sub(7))
                    ));
                }
            } else {
                lines.push(format!(
                    "     {}",
                    middle_truncate(path, width.saturating_sub(5))
                ));
            }
        }
        if items.is_empty() {
            lines.push("  No matching Skills found.".into());
        }
    }
    if let Some(warnings) = value.get("warnings").and_then(Value::as_array) {
        if !warnings.is_empty() {
            lines.push(String::new());
            lines.push("  Retrieval notes".into());
            for warning in warnings.iter().filter_map(Value::as_str).take(3) {
                lines.push(format!("  - {warning}"));
            }
        }
    }
    let loaded = value.get("loaded_skill").is_some_and(Value::is_object);
    let variant_state = value
        .pointer("/matches/0/variant_finding/state")
        .and_then(Value::as_str);
    let next = if loaded {
        "Top match verified and fully loaded in Agent JSON"
    } else {
        match variant_state {
            Some("source_confirmation_required") => {
                "Inspect the linked Finding before confirming exact source roots"
            }
            Some("rescan_required") => "Refresh the Snapshot before loading a variant",
            Some("report_required") => "Materialize the current Report before choosing a variant",
            Some("available") => "Inspect the linked Finding before loading an exact variant",
            Some("finding_unavailable") => {
                "Inspect current layout Findings before choosing a variant"
            }
            _ if value
                .get("matches")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty) =>
            {
                "Refine the task or add one concrete capability hint"
            }
            _ => "Use --load for one-call verified instructions",
        }
    };
    lines.extend(summary("Read-only · no Skill was activated", next));
}

fn plan(value: &Value, lines: &mut Vec<String>, width: usize) {
    fact(lines, "Plan", text(value, "plan_id"));
    let operation_count = value
        .pointer("/change_summary/operation_count")
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or_else(|| array_len(value, "operations"));
    fact(lines, "Operations", operation_count);
    let operation_categories = value
        .get("operation_groups")
        .and_then(Value::as_object)
        .map(|groups| {
            groups
                .iter()
                .map(|(kind, count)| format!("{kind} {}", count.as_u64().unwrap_or_default()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            let counts = value
                .get("operations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("kind").and_then(Value::as_str))
                .fold(
                    std::collections::BTreeMap::<&str, usize>::new(),
                    |mut counts, kind| {
                        *counts.entry(kind).or_default() += 1;
                        counts
                    },
                );
            counts
                .iter()
                .map(|(kind, count)| format!("{kind} {count}"))
                .collect::<Vec<_>>()
        });
    fact_items(lines, "Operation categories", operation_categories, width);
    let roster_changes = value
        .get("roster_changes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let agents = roster_changes
        .iter()
        .filter_map(|item| item.get("agent").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let skills = roster_changes
        .iter()
        .filter_map(|item| item.get("skill_id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let after = roster_changes
        .iter()
        .filter_map(|item| item.get("state").and_then(Value::as_str))
        .fold(
            std::collections::BTreeMap::<&str, usize>::new(),
            |mut counts, state| {
                *counts.entry(state).or_default() += 1;
                counts
            },
        );
    let risk = text(value, "risk");
    let (transition_label, transition) = if risk == "library_governance" {
        let items = value
            .pointer("/impact/items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let item_total = |pointer: &str| {
            items
                .iter()
                .map(|item| item.pointer(pointer).and_then(Value::as_u64))
                .collect::<Option<Vec<_>>>()
                .map(|values| values.into_iter().sum::<u64>())
        };
        let total = |totals_pointer: &str, item_pointer: &str| {
            value
                .pointer(totals_pointer)
                .and_then(Value::as_u64)
                .or_else(|| item_total(item_pointer))
        };
        let relinked = total(
            "/impact/totals/relinked_placement_count",
            "/after/relinked_placement_count",
        )
        .unwrap_or_default();
        let aggregate_state = |totals_pointer: &str, item_pointer: &str, fallback: &str| {
            if let Some(counts) = value.pointer(totals_pointer).and_then(Value::as_object) {
                return if counts.len() == 1 {
                    counts
                        .keys()
                        .next()
                        .map_or_else(|| fallback.to_owned(), std::string::ToString::to_string)
                } else {
                    "mixed".to_owned()
                };
            }
            let states = items
                .iter()
                .filter_map(|item| item.pointer(item_pointer).and_then(Value::as_str))
                .collect::<BTreeSet<_>>();
            match states.len() {
                0 => fallback.to_owned(),
                1 => states
                    .iter()
                    .next()
                    .map_or_else(|| fallback.to_owned(), |state| (*state).to_owned()),
                _ => "mixed".to_owned(),
            }
        };
        let before = aggregate_state(
            "/impact/totals/before/governance_state_counts",
            "/before/governance_state",
            "current",
        );
        let after = aggregate_state(
            "/impact/totals/after/governance_state_counts",
            "/after/governance_state",
            "planned",
        );
        let comparable = match (
            total(
                "/impact/totals/before/physical_source_count",
                "/before/physical_source_count",
            ),
            total(
                "/impact/totals/after/physical_source_count",
                "/after/physical_source_count",
            ),
            total(
                "/impact/totals/before/placement_count",
                "/before/placement_count",
            ),
            total(
                "/impact/totals/after/placement_count",
                "/after/placement_count",
            ),
            total(
                "/impact/totals/before/default_exposed_placement_count",
                "/before/default_exposed_placement_count",
            ),
            total(
                "/impact/totals/after/default_exposed_placement_count",
                "/after/default_exposed_placement_count",
            ),
        ) {
            (
                Some(before_sources),
                Some(after_sources),
                Some(before_placements),
                Some(after_placements),
                Some(before_exposed),
                Some(after_exposed),
            ) => format!(
                "{before} → {after} · sources {before_sources}→{after_sources} · placements {before_placements}→{after_placements} · default-exposed {before_exposed}→{after_exposed} · relinked {relinked}"
            ),
            _ => format!("{before} → {after} · {relinked} placements relinked"),
        };
        ("Library before → after", comparable)
    } else if risk == "source_update" {
        (
            "Source updates",
            format!(
                "{} reviewed changes",
                value
                    .pointer("/diff_summary/item_count")
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
            ),
        )
    } else if risk == "filesystem_change" {
        ("Filesystem change", format!("{operation_count} operations"))
    } else {
        let transition = match (
            value
                .pointer("/impact/before_default_exposure")
                .and_then(Value::as_u64),
            value
                .pointer("/impact/after_default_exposure")
                .and_then(Value::as_u64),
        ) {
            (Some(before), Some(after)) => format!("default exposure {before} → {after}"),
            _ => value
                .pointer("/impact/roster/after_state_counts")
                .and_then(Value::as_object)
                .map(|counts| {
                    counts
                        .iter()
                        .map(|(state, count)| {
                            format!("{state} {}", count.as_u64().unwrap_or_default())
                        })
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .map(|after| format!("current stored states → {after}"))
                .unwrap_or_else(|| format!("current stored states → {}", map_counts(&after))),
        };
        ("Roster before → after", transition)
    };
    fact_items(
        lines,
        transition_label,
        transition.split(" · ").map(str::to_owned).collect(),
        width,
    );
    fact(
        lines,
        "Affected",
        format!(
            "{} Agents · {} Skills",
            value
                .pointer("/affected/agent_count")
                .and_then(Value::as_u64)
                .unwrap_or(agents.len() as u64),
            value
                .pointer("/affected/skill_count")
                .and_then(Value::as_u64)
                .unwrap_or(skills.len() as u64)
        ),
    );
    if let Some(selection) = value.get("selection_evidence") {
        let positive = selection
            .get("positive_signal_core_count")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        fact(
            lines,
            "Core selection",
            format!(
                "{} {} · {} forced · {} fallback",
                positive,
                if positive == 1 { "signal" } else { "signals" },
                selection
                    .get("forced_core_count")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                selection
                    .get("stable_fallback_core_count")
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
            ),
        );
        let direct = selection
            .get("direct_signal_core_count")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let cross_agent = selection
            .get("cross_agent_signal_core_count")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if cross_agent > 0 {
            fact(
                lines,
                "Signal scope",
                format!("{direct} target Agent · {cross_agent} cross-Agent"),
            );
        }
        let preview_width = width.saturating_sub(25).max(1);
        let previews = selection
            .get("agents")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|agent| {
                let agent_id = agent.get("agent").and_then(Value::as_str)?;
                let selected = agent
                    .get("core_preview")
                    .and_then(Value::as_array)?
                    .first()?;
                let name = selected.get("name").and_then(Value::as_str)?;
                let reason = selected
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(core_reason_label)
                    .unwrap_or("selected");
                let remaining = agent
                    .get("core_selection_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .saturating_sub(1);
                let more = if remaining > 0 {
                    format!(" +{remaining}")
                } else {
                    String::new()
                };
                let prefix = format!("{}: ", core_agent_label(agent_id));
                let suffix = format!(" [{reason}]{more}");
                let name_width = preview_width
                    .saturating_sub(display_width(&prefix) + display_width(&suffix))
                    .max(1);
                Some(middle_truncate(
                    &format!("{prefix}{}{suffix}", middle_truncate(name, name_width)),
                    preview_width,
                ))
            })
            .collect::<Vec<_>>();
        if !previews.is_empty() {
            fact_items(lines, "Core preview", previews, width);
        }
    }
    if value
        .pointer("/uncertainty/review_required")
        .and_then(Value::as_bool)
        == Some(true)
    {
        let review = match value.pointer("/uncertainty/code").and_then(Value::as_str) {
            Some("cross_agent_dominated_core_selection") => "cross-Agent-dominated Core selection",
            Some("mixed_evidence_dominated_core_selection") => "fallback + cross-Agent dominance",
            _ => "fallback-dominated Core selection",
        };
        fact(lines, "Review required", review);
    }
    fact(lines, "Risk", risk);
    fact(lines, "Reversible", text(value, "reversible"));
    fact(
        lines,
        "Canonical deletions",
        text(value, "canonical_deletion_count"),
    );
    fact(
        lines,
        "Blocked preconditions",
        value
            .pointer("/impact/blocked_precondition_count")
            .and_then(Value::as_u64)
            .map(|count| count.to_string())
            .or_else(|| {
                value
                    .get("blocked_preconditions")
                    .and_then(Value::as_array)
                    .map(|items| items.len().to_string())
            })
            .unwrap_or_else(|| "none".into()),
    );
    fact(lines, "State", text(value, "state"));
    lines.extend(summary(
        "Preview only · no files changed",
        "Review the Plan before Apply",
    ));
}

fn core_reason_label(reason: &str) -> &str {
    match reason {
        "protected_by_request" => "protected",
        "declared_core" => "declared",
        "skillroster_bootstrap" => "bootstrap",
        "observed_outcome" => "outcome",
        "observed_applied" => "applied",
        "observed_loaded" => "loaded",
        "observed_matched" => "matched",
        "inferred_outcome" => "inferred outcome",
        "inferred_applied" => "inferred applied",
        "inferred_loaded" => "inferred loaded",
        "inferred_matched" => "inferred matched",
        "unknown_quality_outcome" => "outcome?",
        "unknown_quality_applied" => "applied?",
        "unknown_quality_loaded" => "loaded?",
        "unknown_quality_matched" => "matched?",
        "cross_agent_observed_outcome" => "elsewhere outcome",
        "cross_agent_observed_applied" => "elsewhere applied",
        "cross_agent_observed_loaded" => "elsewhere loaded",
        "cross_agent_observed_matched" => "elsewhere matched",
        "cross_agent_inferred_outcome" => "elsewhere inferred outcome",
        "cross_agent_inferred_applied" => "elsewhere inferred applied",
        "cross_agent_inferred_loaded" => "elsewhere inferred loaded",
        "cross_agent_inferred_matched" => "elsewhere inferred matched",
        "cross_agent_unknown_quality_outcome" => "elsewhere outcome?",
        "cross_agent_unknown_quality_applied" => "elsewhere applied?",
        "cross_agent_unknown_quality_loaded" => "elsewhere loaded?",
        "cross_agent_unknown_quality_matched" => "elsewhere matched?",
        "stable_fallback" => "fallback",
        _ => reason,
    }
}

fn core_agent_label(agent: &str) -> &str {
    match agent {
        "codex" => "Codex",
        "claude-code" => "Claude",
        "pi" => "Pi",
        "opencode" => "OpenCode",
        "hermes" => "Hermes",
        "cursor" => "Cursor",
        "gemini-cli" => "Gemini",
        "github-copilot" => "Copilot",
        _ => agent,
    }
}

fn mutation(value: &Value, lines: &mut Vec<String>) {
    if value.get("status").and_then(Value::as_str) == Some("cancelled") {
        fact(lines, "Status", "cancelled");
        fact(lines, "Changed paths", 0);
        fact(lines, "Verification", "not run");
        lines.extend(summary(
            "Cancelled · no files changed",
            "Review the immutable Plan before trying again",
        ));
        return;
    }
    fact(lines, "Plan", text(value, "plan_id"));
    fact(lines, "Receipt", text(value, "receipt_id"));
    fact(lines, "Changed paths", text(value, "changed_path_count"));
    fact(lines, "Verification", text(value, "verification"));
    fact(lines, "Undo available", text(value, "undo_available"));
    if value
        .get("files_changed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        lines.extend(summary(
            "Changed · bounded by the Receipt",
            "Use the Receipt ID to inspect or undo",
        ));
    } else {
        lines.extend(summary(
            "Verified · no files changed",
            "The Receipt records this no-change decision",
        ));
    }
}

fn setup(value: &Value, lines: &mut Vec<String>, width: usize) {
    fact(lines, "State", text(value, "state"));
    fact(lines, "CLI version", text(value, "cli_version"));
    fact(
        lines,
        "Bootstrap content",
        text(value, "bootstrap_content_version"),
    );
    fact(
        lines,
        "Detected Agents",
        array_len(value, "detected_agents"),
    );
    fact(
        lines,
        "Physical targets",
        text(value, "physical_target_count"),
    );
    fact(lines, "Current", text(value, "current_count"));
    fact(lines, "Missing", text(value, "missing_count"));
    fact(lines, "Official outdated", text(value, "outdated_count"));
    fact(lines, "Locally modified", text(value, "modified_count"));
    fact(lines, "Unsupported", text(value, "unsupported_count"));
    fact(lines, "Plan", text(value, "plan_id"));
    let attention = value
        .get("targets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|target| target.get("status").and_then(Value::as_str) != Some("current"))
        .collect::<Vec<_>>();
    if !attention.is_empty() {
        lines.push(String::new());
        lines.push("Targets needing attention".into());
        for target in attention {
            let status = text(target, "status");
            let agent = text(target, "agent");
            let prefix = format!("  {status:<18} {agent:<10} ");
            let path = middle_truncate(
                &text(target, "target"),
                width.saturating_sub(display_width(&prefix)),
            );
            lines.push(format!("{prefix}{path}"));
        }
    }
    match value
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "preview_ready" => lines.extend(summary(
            "Preview only · no files changed",
            "Review the recoverable Plan before Apply",
        )),
        "modified_choice_required" => lines.extend(summary(
            "Blocked · local bootstrap changes need a decision",
            "Choose retain-local or adopt-current; setup will not decide",
        )),
        "unsupported_targets" => lines.extend(summary(
            "Blocked · unsupported bootstrap targets were preserved",
            "Inspect links, non-files, or unreadable targets before retrying",
        )),
        "local_modifications_retained" => lines.extend(summary(
            "No change · local bootstrap modifications retained",
            "Use adopt-current only after the user approves replacement",
        )),
        "up_to_date" => lines.extend(summary(
            "Healthy · bootstrap Skill is current",
            "No setup action is needed",
        )),
        "no_supported_agent" => lines.extend(summary(
            "No change · no supported Agent target found",
            "Run Scan after configuring a supported Agent Skill root",
        )),
        "scan_required" => lines.extend(summary(
            "Blocked · no completed Scan is available",
            "Run skillroster scan before setup",
        )),
        _ => lines.extend(summary(
            "Preview only · no files changed",
            "Follow the reported setup state",
        )),
    }
}

fn lifecycle(value: &Value, lines: &mut Vec<String>) {
    let operation = text(value, "operation");
    fact(lines, "Operation", &operation);
    match operation.as_str() {
        "export" => {
            fact(lines, "Output", text(value, "output_path"));
            lines.extend(summary(
                "Changed · one new export file created",
                "Agent files and configuration were not changed",
            ));
        }
        "purge" => {
            if value.get("raw_usage_days").is_some_and(Value::is_number) {
                fact(
                    lines,
                    "Raw retention",
                    format!("{} days", text(value, "raw_usage_days")),
                );
                fact(
                    lines,
                    "Monthly aggregates",
                    text(value, "monthly_aggregates_retained"),
                );
            }
            let removed_source_details = value
                .get("removed_source_confirmation_details")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if removed_source_details > 0 {
                fact(lines, "Source details removed", removed_source_details);
            }
            let files_changed = value
                .get("files_changed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let plan_history_changed = value
                .get("plans_or_receipts_changed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if plan_history_changed {
                lines.extend(summary(
                    "Changed · Plans and Receipts removed from local state",
                    "Agent and Library files were preserved",
                ));
            } else if removed_source_details > 0
                && value.get("raw_usage_days").is_some_and(Value::is_number)
            {
                lines.extend(summary(
                    "Changed · selected local lifecycle state removed",
                    "Plans, Receipts, Agent files, and Library files were preserved",
                ));
            } else if removed_source_details > 0 {
                lines.extend(summary(
                    "Changed · source-confirmation details removed",
                    "Plans, Receipts, Agent files, and Library files were preserved",
                ));
            } else if files_changed {
                lines.extend(summary(
                    "Changed · raw usage condensed in local state",
                    "Plans, Receipts, Agent files, and Library files were preserved",
                ));
            } else {
                lines.extend(summary(
                    "No stored history matched · no files changed",
                    "Plans, Receipts, Agent files, and Library files were preserved",
                ));
            }
        }
        "recovery_inspect" => {
            fact(lines, "Recovery", text(value, "recovery_state"));
            fact(lines, "Receipts", array_len(value, "receipts"));
            if let Some(receipt) = value
                .get("receipts")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
            {
                fact(
                    lines,
                    "Latest recovery",
                    format!(
                        "{} · {} changed paths · {}",
                        text(receipt, "receipt_id"),
                        array_len(receipt, "changed_paths"),
                        age(receipt.get("completed_at").and_then(Value::as_i64))
                    ),
                );
            }
            lines.extend(summary(
                "Read-only · no files changed",
                "Inspect exact Receipt paths before manual repair",
            ));
        }
        _ => lines.extend(summary(
            "No Agent files changed",
            "Inspect JSON for details",
        )),
    }
}

fn source_root(value: &Value, lines: &mut Vec<String>) {
    let operation = text(value, "operation");
    fact(lines, "Operation", &operation);
    fact(lines, "Scope", "exact local reads only");
    match operation.as_str() {
        "confirm" | "revoke" => {
            fact(
                lines,
                "Permission",
                text(&value["permission"], "permission_id"),
            );
            fact(lines, "Path", text(&value["permission"], "path"));
            fact(lines, "State", text(&value["permission"], "state"));
        }
        "inspect" => {
            fact(lines, "Active", text(value, "active_count"));
            fact(lines, "Revoked", text(value, "revoked_count"));
            for permission in value["permissions"]
                .as_array()
                .into_iter()
                .flatten()
                .take(5)
            {
                fact(
                    lines,
                    &text(permission, "state"),
                    format!(
                        "{} · {}",
                        text(permission, "permission_id"),
                        text(permission, "path")
                    ),
                );
            }
        }
        _ => {}
    }
    lines.extend(summary(
        "Local policy only · no Agent or Skill files changed",
        "Read permission does not endorse content, raise Evidence quality, or authorize governance",
    ));
}

fn home(value: &Value, lines: &mut Vec<String>) {
    fact(lines, "State", text(value, "state"));
    fact(lines, "Recovery", text(value, "recovery_state"));
    lines.push(String::new());
    lines.push("Next: skillroster scan | report | status".into());
}

fn summary(safety: &str, next: &str) -> Vec<String> {
    vec![String::new(), safety.into(), next.into()]
}

fn text(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(v)) => v.clone(),
        Some(Value::Number(v)) => v.to_string(),
        Some(Value::Bool(v)) => v.to_string(),
        Some(Value::Null) | None => "none".into(),
        Some(other) => other.to_string(),
    }
}

fn array_len(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

fn age(timestamp: Option<i64>) -> String {
    let Some(timestamp) = timestamp else {
        return "unknown".into();
    };
    let elapsed = chrono::Utc::now().timestamp().saturating_sub(timestamp);
    if elapsed < 60 {
        format!("{elapsed}s ago")
    } else if elapsed < 3_600 {
        format!("{}m ago", elapsed / 60)
    } else if elapsed < 86_400 {
        format!("{}h ago", elapsed / 3_600)
    } else {
        format!("{}d ago", elapsed / 86_400)
    }
}

fn map_counts(counts: &std::collections::BTreeMap<&str, usize>) -> String {
    if counts.is_empty() {
        return "no Roster changes".into();
    }
    counts
        .iter()
        .map(|(state, count)| format!("{state} {count}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(80)
        .max(40)
}

fn middle_truncate(value: &str, maximum: usize) -> String {
    if display_width(value) <= maximum {
        return value.into();
    }
    if maximum < 5 {
        return take_width(value, maximum, false);
    }
    let left_budget = (maximum - 1) / 2;
    let right_budget = maximum - left_budget - 1;
    format!(
        "{}…{}",
        take_width(value, left_budget, false),
        take_width(value, right_budget, true)
    )
}

fn display_width(value: &str) -> usize {
    value.chars().map(char_width).sum()
}

fn char_width(character: char) -> usize {
    if character == '\0' || character.is_control() {
        0
    } else if matches!(
        character as u32,
        0x1100..=0x115f
            | 0x2329..=0x232a
            | 0x2e80..=0xa4cf
            | 0xac00..=0xd7a3
            | 0xf900..=0xfaff
            | 0xfe10..=0xfe19
            | 0xfe30..=0xfe6f
            | 0xff00..=0xff60
            | 0xffe0..=0xffe6
            | 0x1f300..=0x1faff
            | 0x20000..=0x3fffd
    ) {
        2
    } else {
        1
    }
}

fn take_width(value: &str, budget: usize, reverse: bool) -> String {
    let mut width = 0;
    let source = if reverse {
        value.chars().rev().collect::<Vec<_>>()
    } else {
        value.chars().collect::<Vec<_>>()
    };
    let mut chars = Vec::new();
    for character in source {
        let next = char_width(character);
        if width + next > budget {
            break;
        }
        width += next;
        chars.push(character);
    }
    if reverse {
        chars.reverse();
    }
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn progress_is_limited_to_real_long_running_stages() {
        assert_eq!(
            progress_copy("scan"),
            Some((
                "Scanning configured Agent roots",
                "Interrupted · scan incomplete · no Agent files changed."
            ))
        );
        assert!(progress_copy("report").is_none());
        assert!(progress_copy("find").is_none());
        let interrupted = progress_copy("apply").unwrap().1;
        assert!(interrupted.contains("completion unknown"));
        assert!(interrupted.contains("files may have changed"));
        assert!(interrupted.contains("skillroster status"));
    }

    #[test]
    fn progress_sequences_hide_then_restore_the_cursor() {
        assert!(PROGRESS_BEGIN.contains("?25l"));
        assert!(PROGRESS_END.contains("?25h"));
        assert!(PROGRESS_BEGIN.starts_with('\r'));
        assert!(PROGRESS_END.starts_with('\r'));
    }

    #[test]
    fn status_routes_to_the_highest_priority_read_only_decision() {
        let base = json!({
            "database_path": "/db",
            "schema_version": 9,
            "latest_snapshot_id": "scan_current",
            "latest_snapshot_at": 1,
            "pending_plan_count": 1,
            "pending_plans": [{
                "plan_id": "plan_01M0QDAC102GEXKCMHVP97GF2V",
                "snapshot_id": "scan_current",
                "status": "ready",
                "created_at": 1
            }],
            "last_receipt": null,
            "recovery_state": "clear"
        });
        let options = RenderOptions {
            width: 80,
            styled: false,
        };

        let pending = render("status", &base, options);
        assert!(pending.contains("Review Plan            plan_01M0QDAC102GEXKCMHVP97GF2V"));
        assert!(pending.contains("Next: inspect the Review Plan above"));
        assert!(!pending.contains("Next: skillroster scan"));

        let narrow = render(
            "status",
            &base,
            RenderOptions {
                width: 40,
                styled: false,
            },
        );
        assert!(narrow.contains("Review Plan"));
        assert!(narrow.contains("\n    plan_01M0QDAC102GEXKCMHVP97GF2V"));
        assert!(!narrow.contains("Review Plan            plan_"));
        assert!(narrow.contains("Next: inspect the Review Plan above"));
        assert!(narrow.lines().all(|line| display_width(line) <= 40));

        let mut recovery = base.clone();
        recovery["recovery_state"] = json!("required");
        let recovery = render("status", &recovery, options);
        assert!(recovery.contains("Next: skillroster lifecycle recovery"));

        let mut missing_snapshot = base.clone();
        missing_snapshot["latest_snapshot_id"] = Value::Null;
        let missing_snapshot = render("status", &missing_snapshot, options);
        assert!(missing_snapshot.contains("Next: skillroster scan"));

        let mut healthy = base;
        healthy["pending_plan_count"] = json!(0);
        healthy["pending_plans"] = json!([]);
        let healthy = render("status", &healthy, options);
        assert!(healthy.contains("Next: scan only when fresher inventory is needed"));
    }

    #[test]
    fn cancelled_mutation_is_a_successful_explicit_no_change_result() {
        let output = render(
            "apply",
            &json!({"status": "cancelled", "files_changed": false}),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(output.contains("Cancelled · no files changed"));
        assert!(output.contains("Verification           not run"));
        assert!(!output.contains("bounded by the Receipt"));
    }

    #[test]
    fn verified_no_change_mutation_does_not_claim_a_file_change() {
        let output = render(
            "apply",
            &json!({
                "plan_id": "plan_1",
                "receipt_id": "receipt_1",
                "changed_path_count": 0,
                "verification": "passed",
                "undo_available": false,
                "files_changed": false
            }),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(output.contains("Verified · no files changed"));
        assert!(!output.contains("Changed · bounded"));
    }

    #[test]
    fn setup_output_makes_modified_and_unsupported_targets_actionable() {
        let modified = render(
            "setup",
            &json!({
                "state": "modified_choice_required",
                "cli_version": "1.8.28",
                "bootstrap_content_version": "1.8.28",
                "bootstrap_version": "1.8.28",
                "detected_agents": [{"agent": "codex"}],
                "current_count": 0,
                "missing_count": 0,
                "outdated_count": 0,
                "modified_count": 1,
                "unsupported_count": 0,
                "plan_id": null
            }),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(modified.contains("local bootstrap changes need a decision"));
        assert!(modified.contains("retain-local or adopt-current"));
        assert!(modified.contains("Locally modified       1"));

        let unsupported = render(
            "setup",
            &json!({
                "state": "unsupported_targets",
                "cli_version": "1.8.28",
                "bootstrap_content_version": "1.8.28",
                "bootstrap_version": "1.8.28",
                "detected_agents": [{"agent": "codex"}],
                "current_count": 0,
                "missing_count": 0,
                "outdated_count": 0,
                "modified_count": 0,
                "unsupported_count": 1,
                "plan_id": null
            }),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(unsupported.contains("unsupported bootstrap targets were preserved"));
        assert!(unsupported.contains("Unsupported            1"));
    }

    #[test]
    fn destructive_history_purge_names_what_was_removed() {
        let output = render(
            "lifecycle",
            &json!({
                "operation": "purge",
                "raw_usage_days": null,
                "monthly_aggregates_retained": false,
                "plans_or_receipts_changed": true,
                "files_changed": true
            }),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(output.contains("Plans and Receipts removed"));
        assert!(output.contains("Agent and Library files were preserved"));
        assert!(!output.contains("Plans, Receipts, Agent files"));
    }

    #[test]
    fn find_output_stays_plain_and_bounded_at_reference_widths() {
        let value = json!({
            "matches": [{
                "name": "research",
                "paths": ["/a/very/long/path/that/contains/a/deeply/nested/skill/SKILL.md"]
            }]
        });
        for width in [60_usize, 80, 120] {
            let rendered_path = middle_truncate(
                value["matches"][0]["paths"][0].as_str().unwrap(),
                width.saturating_sub(16),
            );
            assert!(rendered_path.chars().count() <= width.saturating_sub(16));
            assert!(!rendered_path.contains('\u{1b}'));
        }
    }

    #[test]
    fn errors_have_an_explicit_no_change_summary() {
        let rendered = error_human(&"drifted plan");
        assert!(rendered.contains("drifted plan"));
        assert!(rendered.ends_with("No files changed."));
    }

    #[test]
    fn custom_budget_source_blockers_stay_bounded_at_reference_widths() {
        let details = json!({
            "decision": "confirm_trusted_source_roots",
            "requested_core_budget": 10,
            "blocked_change_count": 2,
            "blocked_changes": [
                {
                    "agent": "codex",
                    "name": "alpha",
                    "observed_source_target": "/Users/example/reviewed/alpha"
                },
                {
                    "agent": "codex",
                    "name": "beta",
                    "observed_source_target": "/Users/example/reviewed/beta"
                }
            ],
            "blocked_changes_truncated": false,
            "source_roots": ["/Users/example/reviewed"],
            "source_roots_truncated": false
        });
        for width in [60, 80, 120] {
            let mut lines = vec!["SkillRoster · Plan".into(), String::new()];
            blocked_roster_plan_lines(&details, &mut lines, width);
            let output = lines.join("\n");
            assert!(output.contains("blocked"));
            assert!(output.contains("alpha"));
            assert!(output.contains("--source-root"));
            assert!(output.contains("no automatic change is supported"));
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "line exceeded {width} columns:\n{output}"
            );
        }
    }

    #[test]
    fn custom_budget_overflow_points_at_the_detail_file() {
        let blocked_changes = (0..10)
            .map(|index| {
                json!({
                    "agent": "codex",
                    "name": format!("skill-{index:02}"),
                    "observed_source_target": format!("/opt/root-{index:02}/pkg")
                })
            })
            .collect::<Vec<_>>();
        let source_roots = (0..10)
            .map(|index| json!(format!("/opt/root-{index:02}/pkg")))
            .collect::<Vec<_>>();
        let details = json!({
            "decision": "confirm_trusted_source_roots",
            "requested_core_budget": 10,
            "blocked_change_count": 11,
            "blocked_changes": blocked_changes,
            "blocked_changes_truncated": true,
            "source_roots": source_roots,
            "source_roots_truncated": true,
            "detail": {
                "path": "/tmp/skillroster/source-confirmation/overflow.json"
            }
        });
        let mut lines = vec!["SkillRoster · Plan".into(), String::new()];
        blocked_roster_plan_lines(&details, &mut lines, 80);
        let output = lines.join("\n");
        assert!(output.contains("skill-00"), "{output}");
        assert!(output.contains("skill-04"), "{output}");
        assert!(!output.contains("skill-05"), "{output}");
        assert!(output.contains("/opt/root-00/pkg"), "{output}");
        assert!(!output.contains("/opt/root-05/pkg"), "{output}");
        assert!(
            output.contains("Read /tmp/skillroster/source-confirmation/overflow.json"),
            "{output}"
        );
        assert!(!output.contains("Inspect JSON error.details"), "{output}");
        assert!(!output.to_lowercase().contains("truncated"), "{output}");
        assert!(
            output.lines().all(|line| display_width(line) <= 80),
            "line exceeded 80 columns:\n{output}"
        );
    }

    #[test]
    fn report_preserves_metrics_top_three_rollups_and_totals_at_reference_widths() {
        let value = json!({
            "skill_count": 137,
            "placement_count": 212,
            "default_exposure": 184,
            "observed_use_agent_count": 4,
            "coverage_reliable_agent_count": 3,
            "session_coverage": {
                "supported_agents": 8,
                "roots_present_agents": 6,
                "sampled_agents": 5,
                "complete_agents": 3,
                "limited_agents": 3,
                "missing_root_agents": 2,
                "inaccessible_agents": 0
            },
            "findings": [
                {"severity": "high", "category": "layout", "title": "Broken links"},
                {"severity": "medium", "category": "overlap", "title": "Exact duplicates"},
                {"severity": "low", "category": "lifecycle", "title": "Unknown source"},
                {"severity": "info", "category": "usage", "title": "Coverage"}
            ],
            "finding_rollups": [
                {
                    "title": "Exact duplicate Skill placements",
                    "finding_count": 110,
                    "affected_skill_count": 110,
                    "affected_placement_count": 561
                },
                {
                    "title": "Semantic overlap candidate",
                    "finding_count": 25,
                    "affected_skill_count": 50,
                    "affected_placement_count": 0
                }
            ],
            "category_counts": {"layout": 1, "overlap": 1, "lifecycle": 1, "usage": 1}
        });
        for width in [60, 80, 120] {
            let output = render(
                "report",
                &value,
                RenderOptions {
                    width,
                    styled: false,
                },
            );
            for expected in [
                "Independent Skills",
                "Placements",
                "Default exposure",
                "Observed-use Agents",
                "Broken links",
                "Exact duplicates",
                "Unknown source",
                "Finding groups",
                "110 ×",
                "110 Skills",
                "561 placements",
                "Category totals",
            ] {
                assert!(output.contains(expected), "{expected} missing at {width}");
            }
            assert!(!output.contains("Coverage\n"));
            assert!(output.contains("sampled 5/8 · complete 3/8"));
            assert!(output.contains("limited 3/8 · missing 2/8"));
            assert!(output.contains("Inaccessible"));
            assert!(output.contains("0/8"));
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "line exceeded {width} columns:\n{output}"
            );
        }
    }

    #[test]
    fn finding_report_shows_evidence_and_trust_decision_instead_of_empty_metrics() {
        let value = json!({
            "id": "finding_00000000000000000000000000000000",
            "title": "Skill links escape an approved root",
            "summary": "One placement points outside its approved Agent root.",
            "severity": "high",
            "category": "layout",
            "evidence_quality": "observed",
            "impact": {"affected_skill_count": 1, "affected_placement_count": 1},
            "detail": {"mode": "compact"},
            "items": [{
                "path": "/Users/example/.codex/skills/example/SKILL.md",
                "facts": {"agent": "codex"}
            }],
            "resolution": {
                "decision": "confirm_trusted_source_roots",
                "observed_link_targets": ["/Users/example/source/example"]
            }
        });
        for width in [60, 80, 120] {
            let output = render(
                "report",
                &value,
                RenderOptions {
                    width,
                    styled: false,
                },
            );
            assert!(output.contains("Skill links escape an approved root"));
            assert!(output.contains("Evidence paths"));
            assert!(output.contains("Observed link targets"));
            assert!(output.contains("no automatic change is supported"));
            assert!(!output.contains("Independent Skills     none"));
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "line exceeded {width} columns:\n{output}"
            );
        }
    }

    #[test]
    fn usage_finding_is_readable_and_path_free_at_reference_widths() {
        let first_skill_id = crate::model::SkillId::new().to_string();
        let second_skill_id = crate::model::SkillId::new().to_string();
        let first_skill_suffix = take_width(&first_skill_id, 12, true);
        let second_skill_suffix = take_width(&second_skill_id, 12, true);
        assert_ne!(first_skill_suffix, second_skill_suffix);
        let value = json!({
            "id": "finding_00000000000000000000000000000000",
            "title": "Five-stage usage evidence",
            "summary": "Exposed=548 [1..9; observed]; Matched=3 [2..9; observed]; Loaded=51 [3..9; observed]. Coverage: files discovered=3831, observed=302, partial=28, skipped=3529; truncated=true.",
            "severity": "info",
            "category": "usage",
            "evidence_quality": "unknown",
            "impact": {"affected_skill_count": 117, "affected_placement_count": 0},
            "detail": {"mode": "compact"},
            "usage_overview": {
                "stages": [
                    {"stage": "exposed", "count": 548, "unit": "placements", "quality": "observed", "last_seen_unix": null},
                    {"stage": "matched", "count": 3, "unit": "events", "quality": "observed", "last_seen_unix": 9},
                    {"stage": "loaded", "count": 51, "unit": "events", "quality": "observed", "last_seen_unix": 9},
                    {"stage": "applied", "count": 0, "unit": "events", "quality": "unknown", "last_seen_unix": null},
                    {"stage": "outcome", "count": 0, "unit": "events", "quality": "unknown", "last_seen_unix": null}
                ],
                "coverage": {
                    "supported_agent_count": 8,
                    "roots_present_agent_count": 5,
                    "sampled_agent_count": 5,
                    "complete_agent_count": 0,
                    "limited_agent_count": 5,
                    "missing_agent_count": 3,
                    "inaccessible_agent_count": 0,
                    "files_observed": 302,
                    "files_discovered": 3831,
                    "files_partially_observed": 28,
                    "files_skipped": 3529,
                    "bytes_observed": 19805822,
                    "lines_observed": 30057,
                    "truncated": true,
                    "discovery_truncated": false
                },
                "observed_skills": [
                    {"agent": "cursor", "skill_id": first_skill_id, "skill_name": "code-review", "stage": "loaded", "event_count": 1},
                    {"agent": "claude-code", "skill_id": "skill_history11111", "skill_name": "computer-history", "stage": "matched", "event_count": 2},
                    {"agent": "cursor", "skill_id": second_skill_id, "skill_name": "code-review", "stage": "loaded", "event_count": 3}
                ],
                "observed_signal_count": 8,
                "observed_skills_truncated": true
            },
            "items": [{
                "kind": "usage",
                "path": "/Users/private/session.jsonl",
                "facts": {"agent": "cursor", "skill_name": "code-review", "stage": "loaded"}
            }]
        });
        for width in [60, 80, 120] {
            let output = render(
                "report",
                &value,
                RenderOptions {
                    width,
                    styled: false,
                },
            );
            for expected in [
                "Stage evidence",
                "Exposed",
                "548 placements",
                "Loaded",
                "51 events",
                "Session coverage",
                "Roots / sampled",
                "5/8 · 5/8",
                "3831 discovered",
                "19.8 MB",
                "30057 lines",
                "Observed Skills",
                "cursor",
                "loaded ×1",
                "+ 5 more observed Skill signals",
                "Use --full only when exact complete records are needed",
            ] {
                assert!(
                    output.contains(expected),
                    "{expected} missing at {width}:\n{output}"
                );
            }
            assert!(
                output.contains(&first_skill_suffix),
                "first stable Skill ID suffix missing at {width}:\n{output}"
            );
            assert!(
                output.contains(&second_skill_suffix),
                "second stable Skill ID suffix missing at {width}:\n{output}"
            );
            if width >= 80 {
                assert!(
                    output.contains("code-review"),
                    "Skill name missing at {width}:\n{output}"
                );
            }
            assert!(!output.contains("o…ncated=true"));
            let exposed = output
                .lines()
                .find(|line| line.trim_start().starts_with("Exposed"))
                .expect("Exposed row");
            assert!(!exposed.contains("ago"));
            assert!(!output.contains("unknown · unknown"));
            assert!(!output.contains("/Users/private"));
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "line exceeded {width} columns:\n{output}"
            );
        }
    }

    #[test]
    fn finding_report_shows_same_name_variant_choice_without_empty_paths() {
        let value = json!({
            "id": "finding_00000000000000000000000000000000",
            "title": "Same-name Skills have different content",
            "summary": "humanizer-zh resolves to 2 distinct content digests.",
            "severity": "medium",
            "category": "layout",
            "evidence_quality": "observed",
            "impact": {"affected_skill_count": 2, "affected_placement_count": 5},
            "detail": {"mode": "compact"},
            "items": [{"path": null}, {"path": null}],
            "resolution": {
                "decision": "choose_same_name_variant",
                "variants_truncated": false,
                "variants": [
                    {
                        "content_digests": ["13ccd64485a12613e9e10c96b5289290"],
                        "agents": ["claude-code", "codex", "pi"],
                        "paths": ["/Users/example/.agents_skills/humanizer-zh/SKILL.md"]
                    },
                    {
                        "content_digests": ["b3685e6c0ee6f527e9462db3d36b2a1b"],
                        "agents": ["hermes"],
                        "paths": ["/Users/example/.hermes/skills/humanizer-zh/SKILL.md"]
                    }
                ]
            }
        });
        for width in [60, 80, 120] {
            let output = render(
                "report",
                &value,
                RenderOptions {
                    width,
                    styled: false,
                },
            );
            assert!(output.contains("Variants requiring a choice"));
            assert!(output.contains("13ccd64485a1"));
            assert!(output.contains("hermes"));
            assert!(output.contains("choose one canonical variant first"));
            assert!(!output.contains("source       none"));
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "line exceeded {width} columns:\n{output}"
            );
        }
    }

    #[test]
    fn finding_list_keeps_page_filters_ids_and_width_visible() {
        let value = json!({
            "view": "findings",
            "finding_count": 189,
            "filters": {"category": "overlap", "severity": "medium"},
            "page": {"offset": 20, "returned": 2, "total": 150, "next_offset": 22},
            "items": [
                {
                    "id": "finding_00000000000000000000000000000001",
                    "title": "Exact duplicate Skill placements with an intentionally long title",
                    "severity": "medium",
                    "category": "overlap",
                    "affected_skill_count": 1,
                    "affected_placement_count": 6
                },
                {
                    "id": "finding_00000000000000000000000000000002",
                    "title": "Semantic overlap candidate",
                    "severity": "medium",
                    "category": "overlap",
                    "affected_skill_count": 2,
                    "affected_placement_count": 8
                }
            ]
        });
        for width in [60, 80, 120] {
            let output = render(
                "report",
                &value,
                RenderOptions {
                    width,
                    styled: false,
                },
            );
            assert!(output.contains("21–22 of 150 matching"));
            assert!(output.contains("category overlap · severity medium"));
            assert!(output.contains("Exact dupl"));
            assert!(output.contains("Continue with --offset 22"));
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "line exceeded {width} columns:\n{output}"
            );
        }
    }

    #[test]
    fn plan_wraps_operation_groups_and_keeps_selection_uncertainty_visible() {
        let value = json!({
            "plan_id": "plan_01M0NKH3KS880PP0P4QQDS7W93",
            "change_summary": {"operation_count": 310},
            "operation_groups": {
                "create_directory": 2,
                "create_symlink": 1,
                "move_recoverable": 307
            },
            "affected": {"agent_count": 5, "skill_count": 137},
            "impact": {
                "before_default_exposure": 548,
                "after_default_exposure": 242,
                "blocked_precondition_count": 0
            },
            "selection_evidence": {
                "positive_signal_core_count": 2,
                "direct_signal_core_count": 1,
                "cross_agent_signal_core_count": 1,
                "forced_core_count": 0,
                "stable_fallback_core_count": 198,
                "agents": [
                    {
                        "agent": "codex",
                        "core_preview": [
                            {"name": "code-review", "reason": "observed_loaded"},
                            {"name": "agent-reach", "reason": "stable_fallback"}
                        ],
                        "core_selection_count": 10,
                        "core_preview_truncated": true
                    },
                    {
                        "agent": "claude-code",
                        "core_preview": [
                            {"name": "code-review", "reason": "cross_agent_unknown_quality_loaded"}
                        ],
                        "core_selection_count": 10,
                        "core_preview_truncated": true
                    }
                ]
            },
            "uncertainty": {
                "code": "fallback_dominated_core_selection",
                "review_required": true
            },
            "risk": "roster_change",
            "reversible": true,
            "canonical_deletion_count": 0,
            "state": "ready"
        });
        for width in [60, 80, 120] {
            let output = render(
                "plan",
                &value,
                RenderOptions {
                    width,
                    styled: false,
                },
            );
            for expected in [
                "create_directory 2",
                "create_symlink 1",
                "move_recoverable 307",
                "2 signals · 0 forced · 198 fallback",
                "1 target Agent · 1 cross-Agent",
                "code-review",
                "elsewhere loaded?",
                "fallback-dominated Core selection",
            ] {
                assert!(output.contains(expected), "{expected} missing at {width}");
            }
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "line exceeded {width} columns:\n{output}"
            );
        }
        let mut mixed = value;
        mixed["uncertainty"]["code"] = json!("mixed_evidence_dominated_core_selection");
        let output = render(
            "plan",
            &mixed,
            RenderOptions {
                width: 60,
                styled: false,
            },
        );
        assert!(output.contains("fallback + cross-Agent dominance"));
        assert!(output.lines().all(|line| display_width(line) <= 60));
    }

    #[test]
    fn styled_and_plain_output_preserve_the_same_semantic_facts() {
        let value = json!({
            "skill_count": 3,
            "placement_count": 5,
            "default_exposure": 4,
            "observed_use_agent_count": 1,
            "coverage_reliable_agent_count": 0,
            "findings": [],
            "category_counts": {}
        });
        let plain = render(
            "report",
            &value,
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        let styled = render(
            "report",
            &value,
            RenderOptions {
                width: 80,
                styled: true,
            },
        );
        assert!(!plain.contains('\u{1b}'));
        assert!(styled.contains('\u{1b}'));
        assert_eq!(plain, strip_ansi(&styled));
    }

    #[test]
    fn cjk_paths_are_truncated_by_terminal_columns() {
        let path = "/用户/技能/这是一个很长的中文目录/另一个目录/SKILL.md";
        for width in [20, 40, 60] {
            let output = middle_truncate(path, width);
            assert!(display_width(&output) <= width);
            assert!(output.contains('…') || display_width(path) <= width);
        }
    }

    #[test]
    fn find_and_plan_show_agent_decision_facts() {
        let found = render(
            "find",
            &json!({"matches": [{
                "name": "research",
                "roster_state": "core",
                "source": "github:owner/repo",
                "variant_count": 2,
                "variant_finding": {
                    "state": "available",
                    "finding_id": "finding_variants"
                },
                "match_reasons": ["declared_trigger", "token_overlap:2"],
                "paths": ["/skills/research/SKILL.md"]
            }], "warnings": ["research has two content variants"]}),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(found.contains("roster core · source github:owner/repo"));
        assert!(found.contains("variants 2 · Finding finding_variants"));
        assert!(found.contains("declared_trigger"));
        assert!(found.contains("Retrieval notes"));
        assert!(found.contains("Inspect the linked Finding before loading an exact variant"));

        let source_confirmation = render(
            "find",
            &json!({"matches": [{
                "name": "shared-external",
                "roster_state": "unassigned",
                "source": null,
                "variant_count": 2,
                "variant_finding": {
                    "state": "source_confirmation_required",
                    "finding_id": "finding_source"
                },
                "match_reasons": ["exact_name"],
                "paths": []
            }]}),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(source_confirmation.contains("variants 2 · Finding finding_source"));
        assert!(
            source_confirmation
                .contains("Inspect the linked Finding before confirming exact source roots")
        );
        assert!(!source_confirmation.contains("Use --load"));

        let provider = render(
            "find",
            &json!({"matches": [{
                "name": "control-chrome",
                "providers": ["browser@openai-bundled"],
                "governable": false,
                "match_reasons": ["description_tokens:3"],
                "paths": ["/plugins/browser/skills/control-chrome/SKILL.md"]
            }]}),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(
            provider.contains("Codex plugin browser@openai-bundled · provider-managed · read-only")
        );

        let variants = render(
            "find",
            &json!({"matches": [{
                "name": "shared-browser",
                "roster_state": "unassigned",
                "source": null,
                "variant_count": 2,
                "match_reasons": ["name_tokens:1"],
                "paths": ["/local/shared-browser/SKILL.md"],
                "variants": [
                    {
                        "skill_id": "skill_local",
                        "paths": ["/local/shared-browser/SKILL.md"],
                        "agents": ["codex"],
                        "roster_state": "core",
                        "providers": [],
                        "governable": true
                    },
                    {
                        "skill_id": "skill_plugin",
                        "paths": ["/plugins/browser/shared-browser/SKILL.md"],
                        "agents": [],
                        "roster_state": "unassigned",
                        "providers": ["browser@openai-bundled"],
                        "governable": false
                    }
                ]
            }]}),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(variants.contains("codex · roster core"));
        assert!(variants.contains("/local/shared-browser/SKILL.md"));
        assert!(variants.contains("plugin browser@openai-bundled · read-only"));
        assert!(variants.contains("/plugins/browser/shared-browser/SKILL.md"));

        let planned = render(
            "plan",
            &json!({
                "plan_id": "plan_1",
                "operations": [],
                "roster_changes": [
                    {"agent": "codex", "skill_id": "skill_1", "state": "core"},
                    {"agent": "claude_code", "skill_id": "skill_1", "state": "on_demand"}
                ],
                "risk": "roster_change",
                "reversible": true,
                "canonical_deletion_count": 0,
                "state": "ready"
            }),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(planned.contains("current stored states → core 1 · on_demand 1"));
        assert!(planned.contains("2 Agents · 1 Skills"));
        assert!(planned.contains("Reversible"));

        let library = render(
            "plan",
            &json!({
                "plan_id": "plan_2",
                "change_summary": {"operation_count": 101},
                "operation_groups": {
                    "create_directory": 1,
                    "create_symlink": 50,
                    "move_recoverable": 50
                },
                "affected": {"agent_count": 1, "skill_count": 1},
                "impact": {"items": [{
                    "before": {
                        "governance_state": "observed",
                        "physical_source_count": 3,
                        "placement_count": 51,
                        "default_exposed_placement_count": 50
                    },
                    "after": {
                        "governance_state": "managed",
                        "physical_source_count": 1,
                        "placement_count": 51,
                        "default_exposed_placement_count": 50,
                        "relinked_placement_count": 50
                    }
                }]},
                "risk": "library_governance",
                "reversible": true,
                "canonical_deletion_count": 0,
                "state": "ready"
            }),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(library.contains("Library before → after"));
        assert!(library.contains("observed → managed"));
        assert!(library.contains("sources 3→1"));
        assert!(library.contains("placements 51→51"));
        assert!(library.contains("default-exposed 50→50"));
        assert!(library.contains("relinked 50"));
        assert!(library.lines().all(|line| display_width(line) <= 80));
        assert!(!library.contains("no Roster changes"));

        let mixed_library = render(
            "plan",
            &json!({
                "plan_id": "plan_mixed",
                "change_summary": {"operation_count": 5},
                "affected": {"agent_count": 2, "skill_count": 2},
                "impact": {
                    "item_count": 12,
                    "items_truncated": true,
                    "items": [{
                        "before": {"governance_state": "observed"},
                        "after": {"governance_state": "managed"}
                    }],
                    "totals": {
                        "before": {
                            "governance_state_counts": {"observed": 12},
                            "physical_source_count": 24,
                            "placement_count": 24,
                            "default_exposed_placement_count": 24
                        },
                        "after": {
                            "governance_state_counts": {"managed": 6, "hosted": 6},
                            "physical_source_count": 12,
                            "placement_count": 30,
                            "default_exposed_placement_count": 24
                        },
                        "relinked_placement_count": 18
                    }
                },
                "risk": "library_governance",
                "reversible": true,
                "state": "ready"
            }),
            RenderOptions {
                width: 60,
                styled: false,
            },
        );
        assert!(mixed_library.contains("observed → mixed"));
        assert!(mixed_library.contains("sources 24→12"));
        assert!(mixed_library.contains("placements 24→30"));
        assert!(mixed_library.contains("default-exposed 24→24"));
        assert!(mixed_library.contains("relinked 18"));
        assert!(mixed_library.lines().all(|line| display_width(line) <= 60));
    }

    fn strip_ansi(value: &str) -> String {
        let mut output = String::new();
        let mut chars = value.chars().peekable();
        while let Some(character) = chars.next() {
            if character == '\u{1b}' && chars.peek() == Some(&'[') {
                chars.next();
                for item in chars.by_ref() {
                    if item.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                output.push(character);
            }
        }
        output
    }
}
