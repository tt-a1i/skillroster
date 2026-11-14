use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Local Skill governance for AI Agents.
#[derive(Debug, Parser)]
#[command(
    name = "skillroster",
    version,
    about = "One library. The right roster for every agent."
)]
pub struct Cli {
    /// Emit one stable JSON document for Agent callers.
    #[arg(long, global = true)]
    pub json: bool,

    /// Store state in this directory instead of ~/.skillroster.
    #[arg(long, global = true, env = "SKILLROSTER_STATE_DIR")]
    pub state_dir: Option<PathBuf>,

    /// Resolve supported Agent roots relative to this home directory.
    #[arg(long, global = true, env = "SKILLROSTER_HOME")]
    pub home: Option<PathBuf>,

    /// Add an approved scan root as AGENT=PATH. Repeatable.
    #[arg(long = "root", global = true, value_name = "AGENT=PATH")]
    pub roots: Vec<String>,

    /// Add a non-exposed approved Skill source directory. Repeatable.
    #[arg(long = "source-root", global = true, value_name = "PATH")]
    pub source_roots: Vec<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Cli {
    pub fn command_name(&self) -> &'static str {
        match &self.command {
            Some(Command::Check) => "check",
            Some(Command::Init(_)) => "init",
            Some(Command::Mcp) => "mcp",
            Some(Command::Scan(_)) => "scan",
            Some(Command::Report(_)) => "report",
            Some(Command::Find(_)) => "find",
            Some(Command::Plan(_)) => "plan",
            Some(Command::Apply(_)) => "apply",
            Some(Command::Undo(_)) => "undo",
            Some(Command::Status) => "status",
            Some(Command::Lifecycle(_)) => "lifecycle",
            Some(Command::Setup(_)) => "setup",
            Some(Command::SourceRoot(_)) => "source-root",
            None => "home",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect Skills in one read-only call, scanning when current facts require it.
    Check,
    /// Install or upgrade Bootstrap with one interactive, reviewable confirmation.
    Init(InitArgs),
    /// Serve local read-only Skill search and diagnostics over stdio MCP.
    Mcp,
    /// Discover Skills and persist an immutable Snapshot.
    Scan(ScanArgs),
    /// Analyze the latest Snapshot or drill into one Finding. Defaults to the bounded Summary view.
    Report(ReportArgs),
    /// Rank locally known Skills for a task without activating them.
    Find(FindArgs),
    /// Preview a Finding or shared adoption, or validate a JSON Plan from stdin.
    Plan(PlanArgs),
    /// Apply one previously validated Plan.
    Apply(IdArgs),
    /// Undo only the changes recorded by one Receipt.
    Undo(IdArgs),
    /// Inspect the local state store and recovery boundary.
    Status,
    /// Inspect, export, or prune retained local lifecycle data.
    Lifecycle(LifecycleArgs),
    /// Preview bootstrap Skill installation or upgrade for detected Agents.
    Setup(SetupArgs),
    /// Confirm, inspect, or revoke exact local source-root read permissions.
    SourceRoot(SourceRootArgs),
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Return the bounded Agent continuation view while persisting the complete Snapshot.
    #[arg(long)]
    pub summary: bool,
}

#[derive(Debug, Args)]
pub struct SourceRootArgs {
    #[command(subcommand)]
    pub command: SourceRootCommand,
}

#[derive(Debug, Subcommand)]
pub enum SourceRootCommand {
    /// Permit factual reads of one exact target observed by an escaping-link Finding.
    Confirm(SourceRootConfirmArgs),
    /// Inspect active, revoked, and drifted local read permissions.
    Inspect(SourceRootInspectArgs),
    /// Revoke one exact local read permission by opaque ID.
    Revoke(IdArgs),
}

#[derive(Debug, Args)]
pub struct SourceRootInspectArgs {
    /// Maximum permission records returned in this page.
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: u16,

    /// Zero-based permission-record offset.
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
}

#[derive(Debug, Args)]
pub struct SourceRootConfirmArgs {
    /// Current escaping-link Finding that observed the exact target.
    #[arg(long)]
    pub finding: String,

    /// Exact absolute observed canonical directory.
    #[arg(long)]
    pub path: PathBuf,
}

#[derive(Debug, Args)]
pub struct ReportArgs {
    #[arg(long, conflicts_with_all = ["findings", "summary"])]
    pub finding: Option<String>,

    /// List compact Finding summaries with pagination and optional filters.
    #[arg(long, conflicts_with_all = ["finding", "summary", "full"])]
    pub findings: bool,

    /// Return the exhaustive report, or complete records for one Finding.
    #[arg(long, conflicts_with_all = ["findings", "summary"])]
    pub full: bool,

    /// Return core metrics and the three highest-priority Findings.
    #[arg(long, conflicts_with_all = ["finding", "findings", "full"])]
    pub summary: bool,

    /// Filter a paged Finding list to one category.
    #[arg(long, value_enum, requires = "findings")]
    pub category: Option<ReportCategory>,

    /// Filter a paged Finding list to one severity.
    #[arg(long, value_enum, requires = "findings")]
    pub severity: Option<ReportSeverity>,

    /// Maximum rows returned. Defaults to 5 for compact detail and 20 otherwise.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: Option<u16>,

    /// Zero-based offset for Finding list or detail pagination.
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ReportCategory {
    Inventory,
    Layout,
    Exposure,
    Usage,
    Overlap,
    Routing,
    Lifecycle,
}

impl ReportCategory {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Inventory => "inventory",
            Self::Layout => "layout",
            Self::Exposure => "exposure",
            Self::Usage => "usage",
            Self::Overlap => "overlap",
            Self::Routing => "routing",
            Self::Lifecycle => "lifecycle",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ReportSeverity {
    Info,
    Low,
    Medium,
    High,
}

impl ReportSeverity {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Args)]
pub struct FindArgs {
    #[arg(allow_hyphen_values = true, value_parser = parse_non_empty_task)]
    pub task: String,

    /// Add an Agent-authored lexical retrieval hint while preserving TASK. Repeatable.
    #[arg(long = "hint", value_name = "TEXT")]
    pub hints: Vec<String>,

    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u16).range(1..=i64::from(MAX_FIND_RESULTS)))]
    pub limit: u16,

    /// Return the complete, verified SKILL.md for the unambiguous Top-1 match.
    #[arg(long)]
    pub load: bool,

    /// Load one exact identity from an ambiguous Top-1 same-name group.
    #[arg(long, value_name = "SKILL_ID")]
    pub variant_skill_id: Option<String>,

    /// Load one exact observed placement within the current Top-1 capability group.
    #[arg(long, value_name = "PLACEMENT_ID", requires = "load")]
    pub variant_placement_id: Option<String>,

    /// Fail closed unless the latest Snapshot still matches this opaque ID.
    #[arg(long, value_name = "SCAN_ID")]
    pub require_snapshot: Option<String>,
}

pub const MAX_FIND_RESULTS: u16 = 100;

fn parse_non_empty_task(value: &str) -> Result<String, String> {
    if value.is_empty() {
        Err("TASK must not be empty".into())
    } else {
        Ok(value.to_owned())
    }
}

#[derive(Debug, Args)]
#[command(
    after_long_help = "JSON requests accept finding_roster_changes, finding_library_changes, or adopt_shared_skills, as well as raw governance fields. Use a request_template from check or report."
)]
pub struct PlanArgs {
    /// Read a schema_version 1 JSON request from stdin (the default).
    #[arg(long, conflicts_with_all = ["show", "finding", "adopt_shared", "core_limit"])]
    pub stdin: bool,

    /// Preview the complete Roster change for this current Finding.
    #[arg(long, value_name = "FINDING_ID", conflicts_with_all = ["show", "adopt_shared"])]
    pub finding: Option<String>,

    /// Preview explicit adoption of the observed shared Skills.
    #[arg(long, conflicts_with_all = ["show", "finding"])]
    pub adopt_shared: bool,

    /// Maximum Core Skills per affected Agent (default: 50 for a Finding, 15 for shared adoption).
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=50))]
    pub core_limit: Option<u16>,

    /// Show the complete stored representation of an immutable Plan.
    #[arg(long, value_name = "PLAN_ID", conflicts_with = "core_limit")]
    pub show: Option<String>,
}

#[derive(Debug, Args)]
pub struct IdArgs {
    pub id: String,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ModifiedBootstrapChoice {
    RetainLocal,
    AdoptCurrent,
}

#[derive(Debug, Args)]
pub struct SetupArgs {
    /// Restrict setup to one detected Agent ID; omission includes every eligible Agent.
    #[arg(long, value_name = "AGENT_ID", value_parser = parse_setup_agent)]
    pub agent: Option<crate::harness::AgentKind>,
    /// Inspect detected Bootstrap targets without creating a Plan.
    #[arg(long, conflicts_with = "modified_choice")]
    pub check: bool,
    /// Decide how setup handles locally modified bootstrap Skill files.
    #[arg(long, value_enum)]
    pub modified_choice: Option<ModifiedBootstrapChoice>,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Restrict installation to one detected Agent ID.
    #[arg(long, value_name = "AGENT_ID", value_parser = parse_setup_agent)]
    pub agent: Option<crate::harness::AgentKind>,
    /// Explicitly retain or adopt locally modified Bootstrap files.
    #[arg(long, value_enum)]
    pub modified_choice: Option<ModifiedBootstrapChoice>,
}

fn parse_setup_agent(value: &str) -> Result<crate::harness::AgentKind, String> {
    crate::harness::AgentKind::ALL
        .into_iter()
        .find(|agent| agent.id() == value)
        .ok_or_else(|| format!("unsupported Agent: {value}"))
}

#[derive(Debug, Args)]
pub struct LifecycleArgs {
    #[command(subcommand)]
    pub command: LifecycleCommand,
}

#[derive(Debug, Subcommand)]
pub enum LifecycleCommand {
    /// Inspect retained local state and evidence-source exclusions.
    Inspect,
    /// Export retained usage and evidence summaries as local JSON.
    Export(LifecycleExportArgs),
    /// Exclude one Agent's session evidence from future Scans.
    Exclude(LifecycleExcludeArgs),
    /// Purge explicitly selected retained local state.
    Purge(LifecyclePurgeArgs),
    /// Inspect Receipts that require manual recovery.
    Recovery,
    /// Delete SkillRoster's rebuildable local state; Agent and Library files remain.
    Delete(LifecycleDeleteArgs),
}

#[derive(Debug, Args)]
pub struct LifecycleExportArgs {
    /// New JSON file to create. Existing files are never overwritten.
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Debug, Args)]
pub struct LifecycleExcludeArgs {
    /// One of the eight supported Agent IDs.
    pub agent: String,

    /// Remove this exclusion and allow future local session-evidence scans.
    #[arg(long)]
    pub remove: bool,
}

#[derive(Debug, Args)]
pub struct LifecyclePurgeArgs {
    /// Raw usage/evidence retention window in days.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=3650))]
    pub raw_days: Option<u16>,

    /// Explicitly purge all Plans and Receipts, including their Undo history.
    #[arg(long)]
    pub plans_receipts: bool,

    /// Purge retained source-confirmation detail artifacts.
    #[arg(long)]
    pub source_confirmation: bool,

    /// Required exact token when --plans-receipts is selected.
    #[arg(long)]
    pub confirm: Option<String>,
}

#[derive(Debug, Args)]
pub struct LifecycleDeleteArgs {
    /// Required exact token: DELETE-LOCAL-STATE.
    #[arg(long)]
    pub confirm: String,
}

#[cfg(test)]
mod setup_agent_tests {
    use super::*;

    #[test]
    fn setup_agent_accepts_one_supported_id_and_preserves_choice_conflict() {
        for agent in crate::harness::AgentKind::ALL {
            let cli =
                Cli::try_parse_from(["skillroster", "setup", "--agent", agent.id(), "--check"])
                    .unwrap();
            let Some(Command::Setup(args)) = cli.command else {
                panic!("expected Setup");
            };
            assert_eq!(args.agent, Some(agent));
            assert!(args.check);
        }
        for args in [
            vec!["--agent", "unknown"],
            vec!["--agent", "codex", "--agent", "codex"],
            vec!["--agent", "codex", "claude-code"],
            vec!["--agent", "codex,claude-code"],
            vec![
                "--agent",
                "codex",
                "--check",
                "--modified-choice",
                "retain-local",
            ],
        ] {
            assert!(Cli::try_parse_from([vec!["skillroster", "setup"], args].concat()).is_err());
        }
    }
}
