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
            Some(Command::Scan) => "scan",
            Some(Command::Report(_)) => "report",
            Some(Command::Find(_)) => "find",
            Some(Command::Plan(_)) => "plan",
            Some(Command::Apply(_)) => "apply",
            Some(Command::Undo(_)) => "undo",
            Some(Command::Status) => "status",
            Some(Command::Lifecycle(_)) => "lifecycle",
            Some(Command::Setup(_)) => "setup",
            None => "home",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Discover Skills and persist an immutable Snapshot.
    Scan,
    /// Analyze the latest Snapshot or drill into one Finding.
    Report(ReportArgs),
    /// Rank locally known Skills for a task without activating them.
    Find(FindArgs),
    /// Validate an Agent-authored immutable Plan from stdin.
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
}

#[derive(Debug, Args)]
pub struct ReportArgs {
    #[arg(long, conflicts_with_all = ["findings", "summary"])]
    pub finding: Option<String>,

    /// List compact Finding summaries with pagination and optional filters.
    #[arg(long, conflicts_with_all = ["finding", "summary", "full"])]
    pub findings: bool,

    /// Show complete IDs, placement records, and Evidence records for one Finding.
    #[arg(long, requires = "finding")]
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

    /// Maximum Finding summaries or detail rows returned.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: u16,

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
    pub task: String,

    /// Add an Agent-authored lexical retrieval hint while preserving TASK. Repeatable.
    #[arg(long = "hint", value_name = "TEXT")]
    pub hints: Vec<String>,

    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: u16,
}

#[derive(Debug, Args)]
pub struct PlanArgs {
    #[arg(long, default_value_t = true)]
    pub stdin: bool,

    /// Show the complete stored representation of an immutable Plan.
    #[arg(long, value_name = "PLAN_ID")]
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
    /// Decide how setup handles locally modified bootstrap Skill files.
    #[arg(long, value_enum)]
    pub modified_choice: Option<ModifiedBootstrapChoice>,
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
