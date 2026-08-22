use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn opaque_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed) as u128;
    let process = std::process::id() as u128;
    format!("{prefix}_{:032x}", nanos ^ (process << 64) ^ sequence)
}

macro_rules! opaque_id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(opaque_id($prefix))
            }

            pub fn parse(value: impl Into<String>) -> Result<Self, InvalidId> {
                let value = value.into();
                let expected = concat!($prefix, "_");
                if value.starts_with(expected) && value.len() > expected.len() {
                    Ok(Self(value))
                } else {
                    Err(InvalidId {
                        expected_prefix: $prefix,
                        value,
                    })
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidId {
    expected_prefix: &'static str,
    value: String,
}

impl fmt::Display for InvalidId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "expected an opaque {} ID, got {:?}",
            self.expected_prefix, self.value
        )
    }
}

impl std::error::Error for InvalidId {}

opaque_id_type!(RunId, "run");
opaque_id_type!(AgentId, "agent");
opaque_id_type!(ScanId, "scan");
opaque_id_type!(RootId, "root");
opaque_id_type!(SkillId, "skill");
opaque_id_type!(PlacementId, "placement");
opaque_id_type!(EvidenceId, "evidence");
opaque_id_type!(ReportId, "report");
opaque_id_type!(FindingId, "finding");
opaque_id_type!(PlanId, "plan");
opaque_id_type!(OperationId, "operation");
opaque_id_type!(ReceiptId, "receipt");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Codex,
    ClaudeCode,
    Pi,
    OpenCode,
    Hermes,
    Cursor,
    GeminiCli,
    GithubCopilot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceState {
    Observed,
    Managed,
    Hosted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosterState {
    Core,
    OnDemand,
    ExplicitOnly,
    Archived,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootStatus {
    Included,
    Excluded,
    Missing,
    Inaccessible,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementKind {
    Directory,
    Symlink,
    Configuration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceQuality {
    Observed,
    Inferred,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Path,
    Digest,
    Source,
    Exposure,
    Usage,
    Coverage,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageStage {
    Exposed,
    Matched,
    Loaded,
    Applied,
    Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    Inventory,
    Layout,
    Exposure,
    Usage,
    Overlap,
    Routing,
    Lifecycle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Ready,
    Applying,
    Applied,
    FailedRolledBack,
    RecoveryRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Prepared,
    Applying,
    Applied,
    FailedRolledBack,
    RecoveryRequired,
    Undone,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonEnvelope<T> {
    pub schema_version: u32,
    pub ok: bool,
    pub command: String,
    pub run_id: RunId,
    pub result: Option<T>,
    pub warnings: Vec<String>,
    pub error: Option<ApiError>,
    pub suggested_actions: Vec<SuggestedAction>,
}

impl<T> JsonEnvelope<T> {
    pub fn success(command: impl Into<String>, result: T) -> Self {
        Self {
            schema_version: 1,
            ok: true,
            command: command.into(),
            run_id: RunId::new(),
            result: Some(result),
            warnings: Vec::new(),
            error: None,
            suggested_actions: Vec::new(),
        }
    }

    pub fn failure(command: impl Into<String>, error: ApiError) -> Self {
        Self {
            schema_version: 1,
            ok: false,
            command: command.into(),
            run_id: RunId::new(),
            result: None,
            warnings: Vec::new(),
            error: Some(error),
            suggested_actions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default)]
    pub relevant_ids: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuggestedAction {
    pub action: String,
    pub description: String,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub mutates: bool,
    pub requires_confirmation: bool,
    #[serde(default)]
    pub reason_code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanRun {
    pub id: ScanId,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub status: ScanStatus,
    #[serde(default)]
    pub coverage_notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: AgentId,
    pub kind: AgentKind,
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootRecord {
    pub id: RootId,
    pub scan_id: ScanId,
    pub agent_id: Option<AgentId>,
    pub path: String,
    pub kind: String,
    pub status: RootStatus,
    pub explicit: bool,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillRecord {
    pub id: SkillId,
    pub identity_key: String,
    pub name: String,
    pub description: Option<String>,
    pub declared_source: Option<String>,
    pub declared_revision: Option<String>,
    pub content_digest: String,
    pub digest_version: u32,
    pub governance_state: GovernanceState,
    pub canonical_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlacementRecord {
    pub id: PlacementId,
    pub scan_id: ScanId,
    pub skill_id: SkillId,
    pub agent_id: Option<AgentId>,
    pub root_id: RootId,
    pub path: String,
    pub kind: PlacementKind,
    pub symlink_target: Option<String>,
    pub fingerprint: String,
    pub exposed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: EvidenceId,
    pub scan_id: ScanId,
    pub kind: EvidenceKind,
    pub quality: EvidenceQuality,
    pub subject_type: String,
    pub subject_id: String,
    pub path: Option<String>,
    pub digest: Option<String>,
    pub details: serde_json::Value,
    pub observed_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageEvent {
    pub evidence_id: EvidenceId,
    pub skill_id: SkillId,
    pub agent_id: AgentId,
    pub stage: UsageStage,
    pub quality: EvidenceQuality,
    pub occurred_at: i64,
    pub outcome: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RosterEntry {
    pub agent_id: AgentId,
    pub skill_id: SkillId,
    pub state: RosterState,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportRecord {
    pub id: ReportId,
    pub scan_id: ScanId,
    pub created_at: i64,
    pub summary: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FindingRecord {
    pub id: FindingId,
    pub report_id: ReportId,
    pub category: FindingCategory,
    pub severity: Severity,
    pub title: String,
    pub summary: String,
    pub details: serde_json::Value,
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanRecord {
    pub id: PlanId,
    pub scan_id: ScanId,
    pub report_id: Option<ReportId>,
    pub created_at: i64,
    pub status: PlanStatus,
    pub input: serde_json::Value,
    pub fingerprint: String,
    pub operations: Vec<PlanOperation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanOperation {
    pub id: OperationId,
    pub position: u32,
    pub target_path: String,
    pub expected_fingerprint: Option<String>,
    pub action: OperationAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationAction {
    CreateDirectory,
    CreateSymlink {
        source: String,
        expected_source_fingerprint: String,
    },
    RemoveSymlink,
    Copy {
        source: String,
    },
    MoveRecoverable {
        source: String,
    },
    WriteFile {
        content: String,
    },
    ReplaceFile {
        content: String,
    },
    SetRoster {
        agent_id: AgentId,
        skill_id: SkillId,
        state: RosterState,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReceiptRecord {
    pub id: ReceiptId,
    pub plan_id: PlanId,
    pub reverses_receipt_id: Option<ReceiptId>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub status: ReceiptStatus,
    pub verification: serde_json::Value,
    pub operation_results: Vec<OperationResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationResult {
    pub operation_id: OperationId,
    pub position: u32,
    pub status: String,
    pub before_state: serde_json::Value,
    pub after_state: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_ids_are_prefixed_and_unique() {
        let first = SkillId::new();
        let second = SkillId::new();
        assert!(first.as_str().starts_with("skill_"));
        assert_ne!(first, second);
    }

    #[test]
    fn opaque_id_parser_rejects_wrong_kind() {
        let scan = ScanId::new();
        assert!(SkillId::parse(scan.to_string()).is_err());
    }

    #[test]
    fn success_envelope_matches_agent_contract() {
        let value = serde_json::to_value(JsonEnvelope::success("status", 3)).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["ok"], true);
        assert_eq!(value["command"], "status");
        assert!(value["run_id"].as_str().unwrap().starts_with("run_"));
        assert_eq!(value["result"], 3);
        assert!(value["error"].is_null());
    }
}
