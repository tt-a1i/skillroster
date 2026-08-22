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
        "status" => status(result, &mut lines),
        "scan" => scan(result, &mut lines),
        "report" => report(result, &mut lines, options.width),
        "find" => find(result, &mut lines, options.width),
        "plan" => plan(result, &mut lines),
        "apply" | "undo" => mutation(result, &mut lines),
        "lifecycle" => lifecycle(result, &mut lines),
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

fn status(value: &Value, lines: &mut Vec<String>) {
    fact(lines, "Database", text(value, "database_path"));
    fact(lines, "Schema", text(value, "schema_version"));
    fact(lines, "Latest Snapshot", text(value, "latest_snapshot_id"));
    fact(
        lines,
        "Snapshot age",
        age(value.get("latest_snapshot_at").and_then(Value::as_i64)),
    );
    fact(lines, "Pending Plans", text(value, "pending_plan_count"));
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
    lines.extend(summary(
        "Read-only · no Agent files changed",
        "Next: skillroster scan",
    ));
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
        format!(
            "{} · reliable {}/8",
            text(value, "observed_use_agent_count"),
            text(value, "coverage_reliable_agent_count")
        ),
    );
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
    lines.push(String::new());
    lines.push(format!(
        "  {}",
        middle_truncate(&text(value, "summary"), width.saturating_sub(2))
    ));

    let evidence_rows = value
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| value.get("placements").and_then(Value::as_array));
    if let Some(rows) = evidence_rows.filter(|rows| !rows.is_empty()) {
        lines.push(String::new());
        lines.push("  Evidence paths".into());
        for row in rows.iter().take(10) {
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
        if rows.len() > 10 {
            lines.push(format!("  + {} more on this page", rows.len() - 10));
        }
    }

    if value["resolution"]["decision"].as_str() == Some("confirm_trusted_source_roots") {
        lines.push(String::new());
        lines.push("  Observed link targets".into());
        if let Some(targets) = value["resolution"]["observed_link_targets"].as_array() {
            for target in targets.iter().filter_map(Value::as_str).take(5) {
                lines.push(format!(
                    "  {}",
                    middle_truncate(target, width.saturating_sub(2))
                ));
            }
        }
        lines.extend(summary(
            "Blocked · no automatic change is supported",
            "Confirm trusted source directories before rescanning",
        ));
    } else {
        lines.extend(summary(
            "Read-only · no Agent files changed",
            "Use --full only when exact complete records are needed",
        ));
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
            lines.push(format!("     roster {roster} · source {source}"));
            if item
                .get("variant_count")
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 1)
            {
                lines.push(format!(
                    "     variants {} · inspect layout Finding",
                    text(item, "variant_count")
                ));
            }
            lines.push(format!("     reasons {reasons}"));
            lines.push(format!(
                "     {}",
                middle_truncate(path, width.saturating_sub(5))
            ));
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
    lines.extend(summary(
        "Read-only · no Skill was activated",
        "Read the selected SKILL.md directly",
    ));
}

fn plan(value: &Value, lines: &mut Vec<String>) {
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
                .join(" · ")
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
            map_counts(&counts)
        });
    fact(lines, "Operation categories", operation_categories);
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
        let relinked = items
            .iter()
            .filter_map(|item| item.pointer("/after/relinked_placement_count"))
            .filter_map(Value::as_u64)
            .sum::<u64>();
        let before = items
            .first()
            .and_then(|item| item.pointer("/before/governance_state"))
            .and_then(Value::as_str)
            .unwrap_or("current");
        let after = items
            .first()
            .and_then(|item| item.pointer("/after/governance_state"))
            .and_then(Value::as_str)
            .unwrap_or("planned");
        (
            "Library before → after",
            format!("{before} → {after} · {relinked} placements relinked"),
        )
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
    fact(lines, transition_label, transition);
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
            .get("blocked_preconditions")
            .and_then(Value::as_array)
            .map_or_else(|| "none".into(), |items| items.len().to_string()),
    );
    fact(lines, "State", text(value, "state"));
    lines.extend(summary(
        "Preview only · no files changed",
        "Review the Plan before Apply",
    ));
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
    fact(lines, "Bootstrap version", text(value, "bootstrap_version"));
    fact(
        lines,
        "Detected Agents",
        array_len(value, "detected_agents"),
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
    if display_width(value) <= maximum || maximum < 5 {
        return value.into();
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
                "bootstrap_version": "1.5.0",
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
                "bootstrap_version": "1.5.0",
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
    fn report_preserves_four_metrics_top_three_and_category_totals_at_reference_widths() {
        let value = json!({
            "skill_count": 137,
            "placement_count": 212,
            "default_exposure": 184,
            "observed_use_agent_count": 4,
            "coverage_reliable_agent_count": 3,
            "findings": [
                {"severity": "high", "category": "layout", "title": "Broken links"},
                {"severity": "medium", "category": "overlap", "title": "Exact duplicates"},
                {"severity": "low", "category": "lifecycle", "title": "Unknown source"},
                {"severity": "info", "category": "usage", "title": "Coverage"}
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
                "Category totals",
            ] {
                assert!(output.contains(expected), "{expected} missing at {width}");
            }
            assert!(!output.contains("Coverage\n"));
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
                "match_reasons": ["declared_trigger", "token_overlap:2"],
                "paths": ["/skills/research/SKILL.md"]
            }], "warnings": ["research has two content variants"]}),
            RenderOptions {
                width: 80,
                styled: false,
            },
        );
        assert!(found.contains("roster core · source github:owner/repo"));
        assert!(found.contains("variants 2 · inspect layout Finding"));
        assert!(found.contains("declared_trigger"));
        assert!(found.contains("Retrieval notes"));

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
                    "before": {"governance_state": "observed"},
                    "after": {
                        "governance_state": "managed",
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
        assert!(library.contains("observed → managed · 50 placements relinked"));
        assert!(!library.contains("no Roster changes"));
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
