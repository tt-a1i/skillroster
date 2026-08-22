use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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
            Some(Command::Setup) => "setup",
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
    /// Preview bootstrap Skill installation for detected Agents.
    Setup,
}

#[derive(Debug, Args)]
pub struct ReportArgs {
    #[arg(long)]
    pub finding: Option<String>,
}

#[derive(Debug, Args)]
pub struct FindArgs {
    pub task: String,

    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: u16,
}

#[derive(Debug, Args)]
pub struct PlanArgs {
    #[arg(long, default_value_t = true)]
    pub stdin: bool,
}

#[derive(Debug, Args)]
pub struct IdArgs {
    pub id: String,
}

#[derive(Debug, Args)]
pub struct LifecycleArgs {
    #[command(subcommand)]
    pub command: LifecycleCommand,
}

#[derive(Debug, Subcommand)]
pub enum LifecycleCommand {
    /// Export retained usage and evidence summaries as local JSON.
    Export(LifecycleExportArgs),
    /// Aggregate and remove raw usage/evidence older than the retention window.
    Purge(LifecyclePurgeArgs),
    /// Inspect Receipts that require manual recovery.
    Recovery,
}

#[derive(Debug, Args)]
pub struct LifecycleExportArgs {
    /// New JSON file to create. Existing files are never overwritten.
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Debug, Args)]
pub struct LifecyclePurgeArgs {
    /// Raw usage/evidence retention window in days.
    #[arg(long, default_value_t = 180, value_parser = clap::value_parser!(u16).range(1..=3650))]
    pub raw_days: u16,
}
