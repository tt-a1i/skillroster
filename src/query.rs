use crate::harness::AgentKind;
use crate::model::FindingRecord;
use crate::scan::{
    EvidenceQuality, LinkStatus, ScanResult, ScannedSkill, UsageStage, agents_with_usage,
    placements_by_skill, skill_search_text,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const SAME_NAME_DIVERGENT_FINDING_KIND: &str = "same_name_divergent_content";
pub const SAME_NAME_DIVERGENT_FINDING_TITLE: &str = "Same-name Skills have different content";
pub const SEMANTIC_OVERLAP_FINDING_TITLE: &str = "Semantic overlap candidate";
pub const STALE_ARCHIVE_FINDING_TITLE: &str = "Stale archive candidates require review";
pub const UNKNOWN_ARCHIVE_FINDING_TITLE: &str = "Archive candidacy is unknown";

const SEMANTIC_SHARED_TERM_PREVIEW_LIMIT: usize = 20;
const SEMANTIC_SHARED_TERM_CHARACTER_LIMIT: usize = 64;
const TASK_EXCLUSION_MARKERS: &[&str] = &["do not", "不要", "也不要"];
const TASK_EXCLUSION_EFFECT_PREVIEW_LIMIT: usize = 10;

fn normalize_skill_name(name: &str) -> String {
    name.trim().to_lowercase()
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SemanticOverlapBasis {
    pub metric: &'static str,
    pub score: f64,
    pub intersection_count: usize,
    pub union_count: usize,
    pub shared_terms: Vec<String>,
    pub shared_terms_truncated: bool,
}

fn semantic_overlap_basis_from_tokens(
    left: &BTreeSet<String>,
    right: &BTreeSet<String>,
) -> Option<SemanticOverlapBasis> {
    let shared = left.intersection(right).collect::<Vec<_>>();
    let intersection_count = shared.len();
    let union_count = left.union(right).count();
    if intersection_count < 3 || union_count == 0 {
        return None;
    }
    let shared_terms = shared
        .iter()
        .take(SEMANTIC_SHARED_TERM_PREVIEW_LIMIT)
        .map(|term| {
            term.chars()
                .take(SEMANTIC_SHARED_TERM_CHARACTER_LIMIT)
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let shared_terms_truncated = shared.len() > shared_terms.len()
        || shared
            .iter()
            .take(shared_terms.len())
            .any(|term| term.chars().count() > SEMANTIC_SHARED_TERM_CHARACTER_LIMIT);
    Some(SemanticOverlapBasis {
        metric: "routing_vocabulary_jaccard",
        score: intersection_count as f64 / union_count as f64,
        intersection_count,
        union_count,
        shared_terms,
        shared_terms_truncated,
    })
}

pub(crate) fn semantic_overlap_basis(
    left: &ScannedSkill,
    right: &ScannedSkill,
) -> Option<SemanticOverlapBasis> {
    semantic_overlap_basis_from_tokens(
        &tokens(&skill_search_text(left)),
        &tokens(&skill_search_text(right)),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    ConfiguredRootsInaccessible,
    ConfiguredRootsBounded,
    IncompletePackageFingerprints,
    BrokenSkillLinks,
    EscapingLinkSourceConfirmation,
    SameNameDivergentContent,
    DeclaredIdentityDivergentContent,
    DeclaredNameDirectoryMismatch,
    LargeDefaultRoster,
    FiveStageUsageEvidence,
    UsageCoverageIncomplete,
    ExactDuplicatePlacements,
    SemanticOverlapCandidate,
    MissingRoutingMetadata,
    ExecutableScriptsPresent,
    UnknownProvenance,
    UpstreamDriftUnverified,
    SourceVersionDivergence,
    ManagementStateReview,
    StaleArchiveCandidates,
    ArchiveCandidacyUnknown,
}

impl FindingKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfiguredRootsInaccessible => "configured_roots_inaccessible",
            Self::ConfiguredRootsBounded => "configured_roots_bounded",
            Self::IncompletePackageFingerprints => "incomplete_package_fingerprints",
            Self::BrokenSkillLinks => "broken_skill_links",
            Self::EscapingLinkSourceConfirmation => "escaping_link_source_confirmation",
            Self::SameNameDivergentContent => "same_name_divergent_content",
            Self::DeclaredIdentityDivergentContent => "declared_identity_divergent_content",
            Self::DeclaredNameDirectoryMismatch => "declared_name_directory_mismatch",
            Self::LargeDefaultRoster => "large_default_roster",
            Self::FiveStageUsageEvidence => "five_stage_usage_evidence",
            Self::UsageCoverageIncomplete => "usage_coverage_incomplete",
            Self::ExactDuplicatePlacements => "exact_duplicate_placements",
            Self::SemanticOverlapCandidate => "semantic_overlap_candidate",
            Self::MissingRoutingMetadata => "missing_routing_metadata",
            Self::ExecutableScriptsPresent => "executable_scripts_present",
            Self::UnknownProvenance => "unknown_provenance",
            Self::UpstreamDriftUnverified => "upstream_drift_unverified",
            Self::SourceVersionDivergence => "source_version_divergence",
            Self::ManagementStateReview => "management_state_review",
            Self::StaleArchiveCandidates => "stale_archive_candidates",
            Self::ArchiveCandidacyUnknown => "archive_candidacy_unknown",
        }
    }

    fn from_legacy_title(title: &str) -> Option<Self> {
        match title {
            "Some configured roots were inaccessible" => Some(Self::ConfiguredRootsInaccessible),
            "Some configured roots had bounded discovery" => Some(Self::ConfiguredRootsBounded),
            "Some Skill package fingerprints are incomplete" => {
                Some(Self::IncompletePackageFingerprints)
            }
            "Broken Skill links" => Some(Self::BrokenSkillLinks),
            crate::source_policy::ESCAPING_LINK_FINDING_TITLE => {
                Some(Self::EscapingLinkSourceConfirmation)
            }
            SAME_NAME_DIVERGENT_FINDING_TITLE => Some(Self::SameNameDivergentContent),
            "Declared identity has divergent local content" => {
                Some(Self::DeclaredIdentityDivergentContent)
            }
            "Declared Skill names differ from placement directories" => {
                Some(Self::DeclaredNameDirectoryMismatch)
            }
            "Large default Rosters need review" => Some(Self::LargeDefaultRoster),
            "Five-stage usage evidence" => Some(Self::FiveStageUsageEvidence),
            "Usage coverage is incomplete" => Some(Self::UsageCoverageIncomplete),
            "Exact duplicate Skill placements" => Some(Self::ExactDuplicatePlacements),
            SEMANTIC_OVERLAP_FINDING_TITLE => Some(Self::SemanticOverlapCandidate),
            "Skills lack routing metadata" => Some(Self::MissingRoutingMetadata),
            "Skill packages contain executable scripts" => Some(Self::ExecutableScriptsPresent),
            "Skill provenance is unknown" => Some(Self::UnknownProvenance),
            "Upstream update drift is not verified" => Some(Self::UpstreamDriftUnverified),
            "Declared source has version divergence" => Some(Self::SourceVersionDivergence),
            "Management state needs review" => Some(Self::ManagementStateReview),
            STALE_ARCHIVE_FINDING_TITLE => Some(Self::StaleArchiveCandidates),
            UNKNOWN_ARCHIVE_FINDING_TITLE => Some(Self::ArchiveCandidacyUnknown),
            _ => None,
        }
    }
}

pub(crate) fn finding_kind_from_stored_value(
    stored_kind: Option<&serde_json::Value>,
    legacy_title: &str,
) -> Option<FindingKind> {
    match stored_kind {
        Some(serde_json::Value::String(stored)) => {
            serde_json::from_value(serde_json::Value::String(stored.clone())).ok()
        }
        None | Some(serde_json::Value::Null) => FindingKind::from_legacy_title(legacy_title),
        Some(_) => None,
    }
}

pub fn stored_finding_kind(finding: &FindingRecord) -> Option<FindingKind> {
    finding_kind_from_stored_value(finding.details.get("kind"), &finding.title)
}

pub fn stored_finding_is(finding: &FindingRecord, kind: FindingKind) -> bool {
    stored_finding_kind(finding) == Some(kind)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCoverageBasis {
    #[default]
    SkillRootScan,
    SessionUsage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub id: String,
    pub kind: FindingKind,
    pub category: FindingCategory,
    pub severity: Severity,
    pub title: String,
    pub summary: String,
    pub affected_skill_ids: Vec<String>,
    pub affected_placement_ids: Vec<String>,
    /// Stable path/digest/root references that let callers drill into the fact.
    pub evidence: Vec<String>,
    pub evidence_quality: EvidenceQuality,
    pub coverage_basis: FindingCoverageBasis,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageCountUnit {
    Placements,
    Events,
}

#[derive(Clone, Debug, Serialize)]
pub struct UsageStageSummary {
    pub stage: UsageStage,
    pub count: u64,
    pub unit: UsageCountUnit,
    pub quality: EvidenceQuality,
    pub first_seen_unix: Option<u64>,
    pub last_seen_unix: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UsageCoverageSummary {
    pub supported_agent_count: usize,
    pub roots_present_agent_count: usize,
    pub sampled_agent_count: usize,
    pub complete_agent_count: usize,
    pub limited_agent_count: usize,
    pub missing_agent_count: usize,
    pub inaccessible_agent_count: usize,
    pub files_discovered: usize,
    pub files_observed: usize,
    pub files_partially_observed: usize,
    pub files_skipped: usize,
    pub bytes_observed: u64,
    pub lines_observed: usize,
    pub truncated: bool,
    pub discovery_truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ObservedSkillSignal {
    pub agent: String,
    pub skill_id: String,
    pub skill_name: String,
    pub stage: UsageStage,
    pub quality: EvidenceQuality,
    pub event_count: u64,
    pub last_seen_unix: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UsageOverview {
    pub stages: Vec<UsageStageSummary>,
    pub coverage: UsageCoverageSummary,
    pub observed_skills: Vec<ObservedSkillSignal>,
    pub observed_signal_count: usize,
    pub observed_skills_truncated: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PrimaryMetrics {
    pub independent_skills: usize,
    pub placements: usize,
    pub default_exposure: usize,
    pub agents_with_observed_usage: usize,
    pub agents_with_reliable_session_denominator: usize,
    pub agents_with_session_roots: usize,
    pub agents_with_sampled_session_data: usize,
    pub agents_with_limited_session_data: usize,
    pub agents_missing_session_roots: usize,
    pub agents_with_inaccessible_session_roots: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub metrics: PrimaryMetrics,
    pub findings: Vec<Finding>,
    pub category_counts: BTreeMap<String, usize>,
    pub files_changed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankingAdjustment {
    ProtectedOriginalTaskMatch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FindMatch {
    pub rank: usize,
    pub skill_id: String,
    pub name: String,
    pub score: f64,
    pub paths: Vec<String>,
    pub agents: Vec<String>,
    /// The scanner can prove source metadata, but Roster state is stored separately.
    pub roster_state: String,
    pub source: Option<String>,
    /// Provider identities for externally managed plugin placements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    /// True when at least one placement may participate in a governance Plan.
    pub governable: bool,
    /// Whether any placement path is structurally Agent-owned. `None` means a
    /// legacy Snapshot did not record complete ownership facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by_agent: Option<bool>,
    /// Distinct mutation scopes across the matched placements. An empty set on
    /// a legacy Snapshot means unknown, not mutable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutation_scopes: Vec<crate::scan::MutationScope>,
    pub match_reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_channel_rank: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub augmented_channel_rank: Option<usize>,
    /// Policy adjustments applied after reciprocal-rank fusion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranking_adjustments: Vec<RankingAdjustment>,
    pub evidence_quality: EvidenceQuality,
    /// Same declared name with distinct Skill identities is one ambiguous capability result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variant_skill_ids: Vec<String>,
    /// Same-name identities with provider and path facts kept correctly associated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<FindVariant>,
    pub variant_count: usize,
    pub variants_truncated: bool,
    /// Same-Snapshot analysis needed to resolve same-name content ambiguity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_finding: Option<VariantFindingReference>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct TaskExclusionEffects {
    pub affected_candidate_count: usize,
    pub items: Vec<TaskExclusionEffect>,
    pub items_truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TaskExclusionEffect {
    pub skill_id: String,
    pub name: String,
    pub name_token_count: usize,
    pub trigger_token_count: usize,
    pub description_token_count: usize,
}

pub(crate) struct FindMatchingResult {
    pub matches: Vec<FindMatch>,
    pub task_exclusion_effects: TaskExclusionEffects,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VariantFindingReference {
    pub state: VariantFindingState,
    pub reason_code: VariantFindingReason,
    pub snapshot_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    pub argv: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantFindingState {
    Available,
    SourceConfirmationRequired,
    RescanRequired,
    ReportRequired,
    FindingUnavailable,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantFindingReason {
    SameSnapshotVariantSetMatched,
    UntrustedVariantsRequireSourceConfirmation,
    RoutableVariantDriftDetected,
    CurrentSnapshotReportMissing,
    MatchingDivergentContentFindingMissing,
    MatchingEscapingLinkFindingMissing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FindVariant {
    pub skill_id: String,
    pub paths: Vec<String>,
    pub agents: Vec<String>,
    pub roster_state: String,
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    pub governable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by_agent: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutation_scopes: Vec<crate::scan::MutationScope>,
}

fn placement_authority_facts(
    placements: &[&crate::scan::SkillPlacement],
) -> (Option<bool>, Vec<crate::scan::MutationScope>) {
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
    (owned_by_agent, mutation_scopes)
}

#[derive(Clone, Debug)]
pub(crate) struct RetrievalQuery {
    text: String,
    positive_task_text: String,
    phrases: Vec<String>,
    excluded_task_phrases: Vec<String>,
}

impl RetrievalQuery {
    #[cfg(test)]
    pub(crate) fn from_parts<'a>(parts: impl IntoIterator<Item = &'a str>) -> Self {
        let phrases = parts
            .into_iter()
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let text = phrases.join(" ");
        Self {
            positive_task_text: text.clone(),
            text,
            phrases,
            excluded_task_phrases: Vec::new(),
        }
    }

    pub(crate) fn from_task_and_hints<'a>(
        task: &str,
        hints: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let (positive_task, excluded_task_phrases) = task_routing_sections(task);
        let positive_task_text = positive_task.clone();
        let phrases = std::iter::once(positive_task)
            .chain(hints.into_iter().map(str::trim).map(str::to_owned))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        Self {
            text: phrases.join(" "),
            positive_task_text,
            phrases,
            excluded_task_phrases,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn excluded_task_phrases(&self) -> &[String] {
        &self.excluded_task_phrases
    }
}

pub(crate) fn primary_metrics(scan: &ScanResult) -> PrimaryMetrics {
    PrimaryMetrics {
        independent_skills: scan.skills.len(),
        placements: scan.placements.len(),
        default_exposure: scan
            .placements
            .iter()
            .filter(|placement| placement.default_exposed)
            .count(),
        agents_with_observed_usage: agents_with_usage(scan).len(),
        agents_with_reliable_session_denominator: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.denominator_is_reliable())
            .count(),
        agents_with_session_roots: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.roots_present > 0)
            .count(),
        agents_with_sampled_session_data: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.files_observed > 0)
            .count(),
        agents_with_limited_session_data: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.roots_present > 0 && !coverage.denominator_is_reliable())
            .count(),
        agents_missing_session_roots: scan
            .coverage
            .iter()
            .filter(|coverage| {
                coverage.roots_present == 0
                    && coverage.roots_missing > 0
                    && coverage.roots_inaccessible == 0
            })
            .count(),
        agents_with_inaccessible_session_roots: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.roots_inaccessible > 0)
            .count(),
    }
}

pub fn build_report(scan: &ScanResult) -> Report {
    let metrics = primary_metrics(scan);
    let mut findings = Vec::new();
    inventory_findings(scan, &mut findings);
    layout_findings(scan, &mut findings);
    exposure_findings(scan, &mut findings);
    usage_findings(scan, &mut findings);
    overlap_findings(scan, &mut findings);
    routing_findings(scan, &mut findings);
    lifecycle_findings(scan, &mut findings);
    prioritize_report_findings(&mut findings, 3);

    let mut category_counts = BTreeMap::new();
    for finding in &findings {
        *category_counts
            .entry(category_name(finding.category).to_string())
            .or_insert(0) += 1;
    }
    Report {
        metrics,
        findings,
        category_counts,
        files_changed: false,
    }
}

fn prioritize_report_findings(findings: &mut Vec<Finding>, first_view_limit: usize) {
    findings.sort_by(|left, right| {
        right
            .severity
            .cmp(&left.severity)
            .then_with(|| {
                evidence_priority(right.evidence_quality)
                    .cmp(&evidence_priority(left.evidence_quality))
            })
            .then_with(|| {
                right
                    .affected_placement_ids
                    .len()
                    .cmp(&left.affected_placement_ids.len())
            })
            .then_with(|| {
                right
                    .affected_skill_ids
                    .len()
                    .cmp(&left.affected_skill_ids.len())
            })
            .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
            .then_with(|| left.id.cmp(&right.id))
    });
    if first_view_limit == 0 || findings.is_empty() {
        return;
    }

    let ranked = findings.clone();
    let mut selected_ids = BTreeSet::new();
    let mut selected_categories = BTreeSet::new();
    for severity in [
        Severity::High,
        Severity::Medium,
        Severity::Low,
        Severity::Info,
    ] {
        for finding in ranked.iter().filter(|finding| finding.severity == severity) {
            if selected_ids.len() == first_view_limit {
                break;
            }
            let category = category_name(finding.category);
            if selected_categories.insert(category) {
                selected_ids.insert(finding.id.clone());
            }
        }
        for finding in ranked.iter().filter(|finding| finding.severity == severity) {
            if selected_ids.len() == first_view_limit {
                break;
            }
            selected_ids.insert(finding.id.clone());
        }
        if selected_ids.len() == first_view_limit {
            break;
        }
    }

    let mut ordered = ranked
        .iter()
        .filter(|finding| selected_ids.contains(&finding.id))
        .cloned()
        .collect::<Vec<_>>();
    ordered.extend(
        ranked
            .into_iter()
            .filter(|finding| !selected_ids.contains(&finding.id)),
    );
    *findings = ordered;
}

fn evidence_priority(quality: EvidenceQuality) -> u8 {
    match quality {
        EvidenceQuality::Observed => 2,
        EvidenceQuality::Inferred => 1,
        EvidenceQuality::Unknown => 0,
    }
}

fn inventory_findings(scan: &ScanResult, findings: &mut Vec<Finding>) {
    let unavailable = scan
        .roots
        .iter()
        .filter(|root| root.kind == crate::scan::RootKind::Skills)
        .filter(|root| matches!(root.status, crate::scan::RootStatus::Inaccessible))
        .collect::<Vec<_>>();
    if !unavailable.is_empty() {
        push_finding(
            findings,
            FindingKind::ConfiguredRootsInaccessible,
            FindingCategory::Inventory,
            Severity::Medium,
            "Some configured roots were inaccessible",
            format!(
                "{} known or explicit roots could not be inspected; inventory is partial.",
                unavailable.len()
            ),
            Vec::new(),
            Vec::new(),
            unavailable
                .iter()
                .map(|root| {
                    format!(
                        "path:{}:{}",
                        root.agent.map(AgentKind::id).unwrap_or("shared"),
                        root.path.display()
                    )
                })
                .collect(),
            EvidenceQuality::Observed,
        );
    }

    let bounded = scan
        .roots
        .iter()
        .filter(|root| root.kind == crate::scan::RootKind::Skills)
        .filter(|root| root.status == crate::scan::RootStatus::Included && !root.discovery_complete)
        .collect::<Vec<_>>();
    if !bounded.is_empty() {
        push_finding(
            findings,
            FindingKind::ConfiguredRootsBounded,
            FindingCategory::Inventory,
            Severity::Medium,
            "Some configured roots had bounded discovery",
            format!(
                "{} known or explicit roots were included but not inspected to their full depth; inventory is partial.",
                bounded.len()
            ),
            Vec::new(),
            Vec::new(),
            bounded
                .iter()
                .map(|root| {
                    format!(
                        "path:{}:{}",
                        root.agent.map(AgentKind::id).unwrap_or("shared"),
                        root.path.display()
                    )
                })
                .collect(),
            EvidenceQuality::Observed,
        );
    }

    let incomplete_fingerprints = scan
        .placements
        .iter()
        .filter(|placement| {
            matches!(
                placement.link_status,
                LinkStatus::NotLink | LinkStatus::Valid
            ) && placement.fingerprint_completeness
                != crate::scan::FingerprintCompleteness::Complete
        })
        .collect::<Vec<_>>();
    if !incomplete_fingerprints.is_empty() {
        push_finding(
            findings,
            FindingKind::IncompletePackageFingerprints,
            FindingCategory::Inventory,
            Severity::Medium,
            "Some Skill package fingerprints are incomplete",
            format!(
                "{} readable placements have bounded, unreadable, or legacy-unknown package fingerprints and cannot support exact-content governance.",
                incomplete_fingerprints.len()
            ),
            incomplete_fingerprints
                .iter()
                .map(|placement| placement.skill_id.clone())
                .collect(),
            incomplete_fingerprints
                .iter()
                .map(|placement| placement.id.clone())
                .collect(),
            incomplete_fingerprints
                .iter()
                .map(|placement| format!("path:{}", placement.entrypoint.display()))
                .collect(),
            EvidenceQuality::Observed,
        );
    }
}

fn layout_findings(scan: &ScanResult, findings: &mut Vec<Finding>) {
    for (status, kind, title, severity) in [
        (
            LinkStatus::Broken,
            FindingKind::BrokenSkillLinks,
            "Broken Skill links",
            Severity::High,
        ),
        (
            LinkStatus::EscapesRoot,
            FindingKind::EscapingLinkSourceConfirmation,
            crate::source_policy::ESCAPING_LINK_FINDING_TITLE,
            Severity::High,
        ),
    ] {
        let placements = scan
            .placements
            .iter()
            .filter(|placement| placement.link_status == status)
            .collect::<Vec<_>>();
        if !placements.is_empty() {
            push_finding(
                findings,
                kind,
                FindingCategory::Layout,
                severity,
                title,
                format!("{} placements have this link condition.", placements.len()),
                placements
                    .iter()
                    .map(|placement| placement.skill_id.clone())
                    .collect(),
                placements
                    .iter()
                    .map(|placement| placement.id.clone())
                    .collect(),
                placements
                    .iter()
                    .map(|placement| format!("path:{}", placement.entrypoint.display()))
                    .collect(),
                EvidenceQuality::Observed,
            );
        }
    }

    let mut by_name = BTreeMap::<String, Vec<&ScannedSkill>>::new();
    for skill in &scan.skills {
        by_name
            .entry(normalize_skill_name(&skill.name))
            .or_default()
            .push(skill);
    }
    for (name, skills) in by_name {
        let content_identity_digests = skills
            .iter()
            .filter_map(|skill| skill.content_identity_digest.as_deref())
            .collect::<BTreeSet<_>>();
        let identity_complete = skills
            .iter()
            .all(|skill| skill.content_identity_digest.is_some());
        if skills.len() > 1 && identity_complete && content_identity_digests.len() > 1 {
            let affected_skill_ids = skills
                .iter()
                .map(|skill| skill.id.clone())
                .collect::<Vec<_>>();
            let affected_placement_ids = scan
                .placements
                .iter()
                .filter(|placement| affected_skill_ids.contains(&placement.skill_id))
                .map(|placement| placement.id.clone())
                .collect::<Vec<_>>();
            push_finding(
                findings,
                FindingKind::SameNameDivergentContent,
                FindingCategory::Layout,
                Severity::Medium,
                SAME_NAME_DIVERGENT_FINDING_TITLE,
                format!(
                    "{name} resolves to {} distinct routing content identities.",
                    content_identity_digests.len()
                ),
                affected_skill_ids,
                affected_placement_ids,
                skills
                    .iter()
                    .filter_map(|skill| {
                        skill
                            .content_identity_digest
                            .as_deref()
                            .map(|digest| format!("routing_content_digest:{digest}"))
                    })
                    .collect(),
                EvidenceQuality::Observed,
            );
        }
    }

    for (skill_id, placements) in placements_by_skill(scan) {
        let Some(skill) = scan.skills.iter().find(|skill| skill.id == skill_id) else {
            continue;
        };
        if skill.metadata.source.is_none() {
            continue;
        }
        let digests = placements
            .iter()
            .map(|placement| placement.content_digest.as_str())
            .collect::<BTreeSet<_>>();
        if digests.len() > 1 {
            let skill_name = skill.name.as_str();
            push_finding(
                findings,
                FindingKind::DeclaredIdentityDivergentContent,
                FindingCategory::Layout,
                Severity::High,
                "Declared identity has divergent local content",
                format!(
                    "{skill_name} has {} placement fingerprints under one declared identity.",
                    digests.len()
                ),
                vec![skill_id.to_string()],
                placements
                    .iter()
                    .map(|placement| placement.id.clone())
                    .collect(),
                placements
                    .iter()
                    .flat_map(|placement| {
                        [
                            format!("digest:{}", placement.content_digest),
                            format!("path:{}", placement.entrypoint.display()),
                        ]
                    })
                    .collect(),
                EvidenceQuality::Observed,
            );
        }
    }

    let mismatches = scan
        .placements
        .iter()
        .filter(|placement| placement.declared_name_matches_directory == Some(false))
        .collect::<Vec<_>>();
    if !mismatches.is_empty() {
        push_finding(
            findings,
            FindingKind::DeclaredNameDirectoryMismatch,
            FindingCategory::Layout,
            Severity::Low,
            "Declared Skill names differ from placement directories",
            format!(
                "{} placements declare a Skill name that differs from the containing directory; this is a structural mismatch, not a runtime-safety judgment.",
                mismatches.len()
            ),
            mismatches
                .iter()
                .map(|placement| placement.skill_id.clone())
                .collect(),
            mismatches
                .iter()
                .map(|placement| placement.id.clone())
                .collect(),
            mismatches
                .iter()
                .map(|placement| format!("path:{}", placement.entrypoint.display()))
                .collect(),
            EvidenceQuality::Observed,
        );
    }
}

fn exposure_findings(scan: &ScanResult, findings: &mut Vec<Finding>) {
    let mut exposed_by_agent = BTreeMap::<AgentKind, Vec<&crate::scan::SkillPlacement>>::new();
    for placement in scan
        .placements
        .iter()
        .filter(|placement| placement.default_exposed)
    {
        if let Some(agent) = placement.agent {
            exposed_by_agent.entry(agent).or_default().push(placement);
        }
    }
    let oversized = exposed_by_agent
        .into_iter()
        .filter(|(_, placements)| placements.len() > 50)
        .collect::<Vec<_>>();
    if oversized.is_empty() {
        return;
    }
    let breakdown = oversized
        .iter()
        .map(|(agent, placements)| format!("{}={}", agent.display_name(), placements.len()))
        .collect::<Vec<_>>()
        .join(", ");
    // This is a review threshold, not a claim that any Skill is useless.
    push_finding(
        findings,
        FindingKind::LargeDefaultRoster,
        FindingCategory::Exposure,
        Severity::Medium,
        "Large default Rosters need review",
        format!(
            "{} Agents exceed 50 default-exposed placements: {breakdown}; no archive decision is implied.",
            oversized.len()
        ),
        oversized
            .iter()
            .flat_map(|(_, placements)| placements.iter())
            .map(|placement| placement.skill_id.clone())
            .collect(),
        oversized
            .iter()
            .flat_map(|(_, placements)| placements.iter())
            .map(|placement| placement.id.clone())
            .collect(),
        oversized
            .iter()
            .flat_map(|(_, placements)| placements.iter())
            .map(|placement| format!("path:{}", placement.entrypoint.display()))
            .collect(),
        EvidenceQuality::Observed,
    );
}

pub(crate) fn usage_overview(scan: &ScanResult) -> UsageOverview {
    let stages = [
        UsageStage::Exposed,
        UsageStage::Matched,
        UsageStage::Loaded,
        UsageStage::Applied,
        UsageStage::Outcome,
    ]
    .into_iter()
    .map(|stage| {
        let observations = scan
            .usage
            .iter()
            .filter(|usage| usage.stage == stage)
            .collect::<Vec<_>>();
        let count = if stage == UsageStage::Exposed {
            scan.placements
                .iter()
                .filter(|placement| placement.default_exposed)
                .count() as u64
        } else {
            observations.iter().map(|usage| usage.event_count).sum()
        };
        let quality = if stage == UsageStage::Exposed
            || observations
                .iter()
                .any(|usage| usage.quality == EvidenceQuality::Observed)
        {
            EvidenceQuality::Observed
        } else if observations
            .iter()
            .any(|usage| usage.quality == EvidenceQuality::Inferred)
        {
            EvidenceQuality::Inferred
        } else {
            EvidenceQuality::Unknown
        };
        UsageStageSummary {
            stage,
            count,
            unit: if stage == UsageStage::Exposed {
                UsageCountUnit::Placements
            } else {
                UsageCountUnit::Events
            },
            quality,
            first_seen_unix: if stage == UsageStage::Exposed {
                None
            } else {
                observations
                    .iter()
                    .filter_map(|usage| usage.first_seen_unix)
                    .min()
            },
            last_seen_unix: if stage == UsageStage::Exposed {
                None
            } else {
                observations
                    .iter()
                    .filter_map(|usage| usage.last_seen_unix)
                    .max()
            },
        }
    })
    .collect::<Vec<_>>();

    let coverage = UsageCoverageSummary {
        supported_agent_count: AgentKind::ALL.len(),
        roots_present_agent_count: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.roots_present > 0)
            .count(),
        sampled_agent_count: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.files_observed > 0)
            .count(),
        complete_agent_count: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.denominator_is_reliable())
            .count(),
        limited_agent_count: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.roots_present > 0 && !coverage.denominator_is_reliable())
            .count(),
        missing_agent_count: scan
            .coverage
            .iter()
            .filter(|coverage| {
                coverage.roots_present == 0
                    && coverage.roots_missing > 0
                    && coverage.roots_inaccessible == 0
            })
            .count(),
        inaccessible_agent_count: scan
            .coverage
            .iter()
            .filter(|coverage| coverage.roots_inaccessible > 0)
            .count(),
        files_discovered: scan
            .coverage
            .iter()
            .map(|coverage| {
                coverage.files_discovered.max(
                    coverage
                        .files_observed
                        .saturating_add(coverage.files_skipped),
                )
            })
            .sum(),
        files_observed: scan
            .coverage
            .iter()
            .map(|coverage| coverage.files_observed)
            .sum(),
        files_partially_observed: scan
            .coverage
            .iter()
            .map(|coverage| coverage.files_partially_observed)
            .sum(),
        files_skipped: scan
            .coverage
            .iter()
            .map(|coverage| coverage.files_skipped)
            .sum(),
        bytes_observed: scan
            .coverage
            .iter()
            .map(|coverage| coverage.bytes_observed)
            .sum(),
        lines_observed: scan
            .coverage
            .iter()
            .map(|coverage| coverage.lines_observed)
            .sum(),
        truncated: scan.coverage.iter().any(|coverage| coverage.truncated),
        discovery_truncated: scan
            .coverage
            .iter()
            .any(|coverage| coverage.discovery_truncated),
    };

    let skill_names = scan
        .skills
        .iter()
        .map(|skill| (skill.id.as_str(), skill.name.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut grouped_signals = BTreeMap::<(String, String, UsageStage), ObservedSkillSignal>::new();
    for usage in scan
        .usage
        .iter()
        .filter(|usage| usage.stage != UsageStage::Exposed)
    {
        let Some(skill_name) = skill_names.get(usage.skill_id.as_str()) else {
            continue;
        };
        let key = (usage.agent.id().into(), usage.skill_id.clone(), usage.stage);
        let signal = grouped_signals
            .entry(key)
            .or_insert_with(|| ObservedSkillSignal {
                agent: usage.agent.id().into(),
                skill_id: usage.skill_id.clone(),
                skill_name: (*skill_name).into(),
                stage: usage.stage,
                quality: EvidenceQuality::Unknown,
                event_count: 0,
                last_seen_unix: None,
            });
        signal.quality = stronger_evidence_quality(signal.quality, usage.quality);
        signal.event_count = signal.event_count.saturating_add(usage.event_count);
        signal.last_seen_unix = signal.last_seen_unix.max(usage.last_seen_unix);
    }
    let mut observed_skills = grouped_signals.into_values().collect::<Vec<_>>();
    observed_skills.sort_by(|left, right| {
        usage_preview_priority(right.stage)
            .cmp(&usage_preview_priority(left.stage))
            .then_with(|| right.last_seen_unix.cmp(&left.last_seen_unix))
            .then_with(|| right.event_count.cmp(&left.event_count))
            .then_with(|| left.agent.cmp(&right.agent))
            .then_with(|| left.skill_name.cmp(&right.skill_name))
    });
    let observed_signal_count = observed_skills.len();
    let observed_skills = observed_skills.into_iter().take(5).collect::<Vec<_>>();

    UsageOverview {
        stages,
        coverage,
        observed_skills,
        observed_signal_count,
        observed_skills_truncated: observed_signal_count > 5,
    }
}

fn usage_preview_priority(stage: UsageStage) -> u8 {
    match stage {
        UsageStage::Loaded => 4,
        UsageStage::Applied => 3,
        UsageStage::Outcome => 2,
        UsageStage::Matched => 1,
        UsageStage::Exposed => 0,
    }
}

fn evidence_quality_name(quality: EvidenceQuality) -> &'static str {
    match quality {
        EvidenceQuality::Observed => "observed",
        EvidenceQuality::Inferred => "inferred",
        EvidenceQuality::Unknown => "unknown",
    }
}

fn stronger_evidence_quality(left: EvidenceQuality, right: EvidenceQuality) -> EvidenceQuality {
    match (left, right) {
        (EvidenceQuality::Observed, _) | (_, EvidenceQuality::Observed) => {
            EvidenceQuality::Observed
        }
        (EvidenceQuality::Inferred, _) | (_, EvidenceQuality::Inferred) => {
            EvidenceQuality::Inferred
        }
        _ => EvidenceQuality::Unknown,
    }
}

fn usage_findings(scan: &ScanResult, findings: &mut Vec<Finding>) {
    let overview = usage_overview(scan);
    let stage_summaries = overview
        .stages
        .iter()
        .map(|stage| {
            let first = stage
                .first_seen_unix
                .map_or_else(|| "unknown".into(), |value| value.to_string());
            let last = stage
                .last_seen_unix
                .map_or_else(|| "unknown".into(), |value| value.to_string());
            format!(
                "{}={} [{first}..{last}; {}]",
                usage_stage_name(stage.stage),
                stage.count,
                evidence_quality_name(stage.quality)
            )
        })
        .collect::<Vec<_>>();
    let evidence = scan
        .usage
        .iter()
        .map(crate::scan::UsageEvidence::evidence_reference)
        .chain(
            scan.coverage
                .iter()
                .map(|coverage| format!("coverage:{}", coverage.agent.id())),
        )
        .collect();
    let coverage = &overview.coverage;
    push_session_finding(
        findings,
        FindingKind::FiveStageUsageEvidence,
        FindingCategory::Usage,
        Severity::Info,
        "Five-stage usage evidence",
        format!(
            "{}. Coverage: roots {}/{supported}, sampled {}/{supported}, complete {}/{supported}, missing {}/{supported}, inaccessible {}/{supported}; files discovered={}, observed={}, partial={}, skipped={}; bytes={}, lines={}, truncated={}, discovery_truncated={}.",
            stage_summaries.join("; "),
            coverage.roots_present_agent_count,
            coverage.sampled_agent_count,
            coverage.complete_agent_count,
            coverage.missing_agent_count,
            coverage.inaccessible_agent_count,
            coverage.files_discovered,
            coverage.files_observed,
            coverage.files_partially_observed,
            coverage.files_skipped,
            coverage.bytes_observed,
            coverage.lines_observed,
            coverage.truncated,
            coverage.discovery_truncated,
            supported = coverage.supported_agent_count,
        ),
        scan.usage
            .iter()
            .map(|usage| usage.skill_id.clone())
            .collect(),
        Vec::new(),
        evidence,
        if coverage.complete_agent_count == coverage.supported_agent_count {
            EvidenceQuality::Observed
        } else {
            EvidenceQuality::Unknown
        },
    );

    let unreliable = scan
        .coverage
        .iter()
        .filter(|coverage| !coverage.denominator_is_reliable())
        .collect::<Vec<_>>();
    let incomplete_count = coverage
        .supported_agent_count
        .saturating_sub(coverage.complete_agent_count);
    if incomplete_count != 0 {
        push_session_finding(
            findings,
            FindingKind::UsageCoverageIncomplete,
            FindingCategory::Usage,
            Severity::Info,
            "Usage coverage is incomplete",
            format!(
                "A complete observable-session denominator is unavailable for {incomplete_count}/{} supported Agents: {} session roots are missing, {} are inaccessible, and {} present roots have bounded or incomplete samples. Recent observed events remain usable; absence of evidence is not evidence of non-use.",
                coverage.supported_agent_count,
                coverage.missing_agent_count,
                coverage.inaccessible_agent_count,
                coverage.limited_agent_count,
            ),
            Vec::new(),
            Vec::new(),
            unreliable
                .iter()
                .map(|coverage| format!("coverage:{}", coverage.agent.id()))
                .collect(),
            EvidenceQuality::Unknown,
        );
    }
}

fn overlap_findings(scan: &ScanResult, findings: &mut Vec<Finding>) -> (usize, usize) {
    for (skill_id, placements) in placements_by_skill(scan) {
        let Some(skill) = scan.skills.iter().find(|skill| skill.id == skill_id) else {
            continue;
        };
        if placements.iter().any(|placement| {
            placement.fingerprint_completeness != crate::scan::FingerprintCompleteness::Complete
        }) {
            continue;
        }
        let mut by_digest = BTreeMap::<&str, Vec<&crate::scan::SkillPlacement>>::new();
        for placement in placements {
            by_digest
                .entry(placement.content_digest.as_str())
                .or_default()
                .push(placement);
        }
        for (digest, exact_placements) in by_digest {
            if exact_placements.len() < 2 {
                continue;
            }
            let physical_sources = exact_placements
                .iter()
                .map(|placement| physical_source_identity(placement, &scan.placements))
                .collect::<BTreeSet<_>>();
            if physical_sources.len() < 2 {
                continue;
            }
            push_finding(
                findings,
                FindingKind::ExactDuplicatePlacements,
                FindingCategory::Overlap,
                Severity::Medium,
                "Exact duplicate Skill placements",
                format!(
                    "{} has {} placements across {} distinct physical sources with the same normalized content digest.",
                    skill.name,
                    exact_placements.len(),
                    physical_sources.len()
                ),
                vec![skill.id.clone()],
                exact_placements
                    .iter()
                    .map(|placement| placement.id.clone())
                    .collect(),
                std::iter::once(format!("digest:{digest}"))
                    .chain(
                        exact_placements
                            .iter()
                            .map(|placement| format!("path:{}", placement.entrypoint.display())),
                    )
                    .collect(),
                EvidenceQuality::Observed,
            );
        }
    }

    // Semantic similarity is deliberately candidate evidence only. It never
    // authorizes consolidation, deletion, or an automatic Plan.
    let vocabularies = scan
        .skills
        .iter()
        .map(|skill| tokens(&skill_search_text(skill)))
        .collect::<Vec<_>>();
    let normalized_names = scan
        .skills
        .iter()
        .map(|skill| normalize_skill_name(&skill.name))
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut pair_comparison_count = 0usize;
    for (index, left) in scan.skills.iter().enumerate() {
        let left_tokens = &vocabularies[index];
        for (right_index, right) in scan.skills.iter().enumerate().skip(index + 1) {
            pair_comparison_count = pair_comparison_count.saturating_add(1);
            if normalized_names[index] == normalized_names[right_index] {
                continue;
            }
            if left.content_digest == right.content_digest {
                continue;
            }
            let right_tokens = &vocabularies[right_index];
            let Some(basis) = semantic_overlap_basis_from_tokens(left_tokens, right_tokens) else {
                continue;
            };
            let similarity = basis.score;
            if similarity >= 0.45 {
                candidates.push((similarity, left, right));
            }
        }
    }
    candidates.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.1.id.cmp(&right.1.id))
            .then_with(|| left.2.id.cmp(&right.2.id))
    });
    for (similarity, left, right) in candidates.into_iter().take(25) {
        push_finding(
            findings,
            FindingKind::SemanticOverlapCandidate,
            FindingCategory::Overlap,
            Severity::Low,
            SEMANTIC_OVERLAP_FINDING_TITLE,
            format!(
                "{} and {} share routing vocabulary (Jaccard {:.2}); this is review-only candidate evidence, not a confirmed duplicate.",
                left.name, right.name, similarity
            ),
            vec![left.id.clone(), right.id.clone()],
            Vec::new(),
            vec![
                format!("digest:{}", left.content_digest),
                format!("digest:{}", right.content_digest),
            ],
            EvidenceQuality::Inferred,
        );
    }
    (vocabularies.len(), pair_comparison_count)
}

fn physical_source_identity(
    start: &crate::scan::SkillPlacement,
    placements: &[crate::scan::SkillPlacement],
) -> PathBuf {
    if start.physical_directory.is_some() {
        return start.physical_directory_or_logical().to_path_buf();
    }
    let mut current = start;
    let mut visited = BTreeSet::new();
    loop {
        if current.link_status != LinkStatus::Valid {
            return current.directory.clone();
        }
        if !visited.insert(current.directory.clone()) {
            return current.directory.clone();
        }
        let Some(target) = current.link_target.as_deref() else {
            return current.directory.clone();
        };
        if let Some(next) = placements
            .iter()
            .find(|placement| placement.directory == target || placement.entrypoint == target)
        {
            current = next;
            continue;
        }
        return skill_directory_for_link_target(target);
    }
}

fn skill_directory_for_link_target(target: &Path) -> PathBuf {
    if target.file_name().is_some_and(|name| name == "SKILL.md") {
        target.parent().unwrap_or(target).to_path_buf()
    } else {
        target.to_path_buf()
    }
}

fn routing_findings(scan: &ScanResult, findings: &mut Vec<Finding>) {
    let weak = scan
        .skills
        .iter()
        .filter(|skill| {
            skill
                .metadata
                .description
                .as_deref()
                .is_none_or(str::is_empty)
                && skill.metadata.triggers.is_empty()
        })
        .collect::<Vec<_>>();
    if !weak.is_empty() {
        push_finding(
            findings,
            FindingKind::MissingRoutingMetadata,
            FindingCategory::Routing,
            Severity::Low,
            "Skills lack routing metadata",
            format!(
                "{} Skills have neither a declared description nor triggers, reducing deterministic search recall.",
                weak.len()
            ),
            weak.iter().map(|skill| skill.id.clone()).collect(),
            Vec::new(),
            evidence_paths_for_skills(scan, &weak),
            EvidenceQuality::Observed,
        );
    }
}

fn lifecycle_findings(scan: &ScanResult, findings: &mut Vec<Finding>) {
    let executable = scan
        .placements
        .iter()
        .filter(|placement| !placement.executable_files.is_empty())
        .collect::<Vec<_>>();
    if !executable.is_empty() {
        let file_count = executable
            .iter()
            .map(|placement| placement.executable_files.len())
            .sum::<usize>();
        push_finding(
            findings,
            FindingKind::ExecutableScriptsPresent,
            FindingCategory::Lifecycle,
            Severity::Info,
            "Skill packages contain executable scripts",
            format!(
                "{file_count} executable or script-extension files were observed across {} placements. Presence alone does not establish that code is safe, unsafe, or executed.",
                executable.len()
            ),
            executable
                .iter()
                .map(|placement| placement.skill_id.clone())
                .collect(),
            executable
                .iter()
                .map(|placement| placement.id.clone())
                .collect(),
            executable
                .iter()
                .map(|placement| format!("path:{}", placement.entrypoint.display()))
                .collect(),
            EvidenceQuality::Observed,
        );
    }

    let unknown_source = scan
        .skills
        .iter()
        .filter(|skill| skill.metadata.source.is_none())
        .collect::<Vec<_>>();
    if !unknown_source.is_empty() {
        push_finding(
            findings,
            FindingKind::UnknownProvenance,
            FindingCategory::Lifecycle,
            Severity::Low,
            "Skill provenance is unknown",
            format!(
                "{} Skills do not declare a source; update drift cannot be evaluated reliably.",
                unknown_source.len()
            ),
            unknown_source
                .iter()
                .map(|skill| skill.id.clone())
                .collect(),
            Vec::new(),
            evidence_paths_for_skills(scan, &unknown_source),
            EvidenceQuality::Observed,
        );
    }

    let sourced = scan
        .skills
        .iter()
        .filter(|skill| skill.metadata.source.is_some())
        .collect::<Vec<_>>();
    if !sourced.is_empty() {
        let source_examples = sourced
            .iter()
            .take(5)
            .map(|skill| {
                format!(
                    "{}@{}",
                    skill.metadata.source.as_deref().unwrap_or("unknown"),
                    skill
                        .metadata
                        .version
                        .as_deref()
                        .or(skill.metadata.revision.as_deref())
                        .unwrap_or("unknown")
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        push_finding(
            findings,
            FindingKind::UpstreamDriftUnverified,
            FindingCategory::Lifecycle,
            Severity::Info,
            "Upstream update drift is not verified",
            format!(
                "{} Skills declare a source ({source_examples}), but this local scan did not query upstream state; update drift remains unknown.",
                sourced.len(),
            ),
            sourced.iter().map(|skill| skill.id.clone()).collect(),
            Vec::new(),
            sourced
                .iter()
                .map(|skill| format!("digest:{}", skill.content_digest))
                .collect(),
            EvidenceQuality::Unknown,
        );
    }

    let mut by_source = BTreeMap::<&str, Vec<&ScannedSkill>>::new();
    for skill in scan
        .skills
        .iter()
        .filter(|skill| skill.metadata.source.is_some())
    {
        by_source
            .entry(skill.metadata.source.as_deref().unwrap_or_default())
            .or_default()
            .push(skill);
    }
    for (source, skills) in by_source {
        let revisions = skills
            .iter()
            .filter_map(|skill| {
                skill
                    .metadata
                    .version
                    .as_deref()
                    .or(skill.metadata.revision.as_deref())
            })
            .collect::<BTreeSet<_>>();
        if revisions.len() < 2 {
            continue;
        }
        push_finding(
            findings,
            FindingKind::SourceVersionDivergence,
            FindingCategory::Lifecycle,
            Severity::Medium,
            "Declared source has version divergence",
            format!(
                "Source {source} appears at {} declared versions or revisions across local Skills; this reports local divergence and does not determine which revision should be used.",
                revisions.len()
            ),
            skills.iter().map(|skill| skill.id.clone()).collect(),
            Vec::new(),
            evidence_paths_for_skills(scan, &skills),
            EvidenceQuality::Observed,
        );
    }

    let multi_placement = placements_by_skill(scan)
        .into_iter()
        .filter(|(_, placements)| placements.len() > 1)
        .collect::<Vec<_>>();
    if !multi_placement.is_empty() {
        push_finding(
            findings,
            FindingKind::ManagementStateReview,
            FindingCategory::Lifecycle,
            Severity::Low,
            "Management state needs review",
            format!(
                "{} Skills have multiple placements. The scan observes layout but cannot infer managed/hosted ownership without an applied governance record.",
                multi_placement.len()
            ),
            multi_placement
                .iter()
                .map(|(skill_id, _)| (*skill_id).to_string())
                .collect(),
            multi_placement
                .iter()
                .flat_map(|(_, placements)| placements.iter().map(|item| item.id.clone()))
                .collect(),
            multi_placement
                .iter()
                .flat_map(|(_, placements)| {
                    placements
                        .iter()
                        .map(|item| format!("path:{}", item.entrypoint.display()))
                })
                .collect(),
            EvidenceQuality::Unknown,
        );
    }

    let reliable_agents = scan
        .coverage
        .iter()
        .filter(|coverage| coverage.denominator_is_reliable())
        .map(|coverage| coverage.agent)
        .collect::<BTreeSet<_>>();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let stale_cutoff = now.saturating_sub(180 * 24 * 60 * 60);
    let stale_candidates = scan
        .skills
        .iter()
        .filter(|skill| {
            let placement_agents = scan
                .placements
                .iter()
                .filter(|placement| placement.skill_id == skill.id)
                .filter_map(|placement| placement.agent)
                .collect::<Vec<_>>();
            skill
                .modified_at_unix
                .is_some_and(|time| time < stale_cutoff)
                && !scan.usage.iter().any(|usage| usage.skill_id == skill.id)
                && !placement_agents.is_empty()
                && placement_agents
                    .iter()
                    .all(|agent| reliable_agents.contains(agent))
        })
        .collect::<Vec<_>>();
    if !stale_candidates.is_empty() {
        push_session_finding(
            findings,
            FindingKind::StaleArchiveCandidates,
            FindingCategory::Lifecycle,
            Severity::Low,
            STALE_ARCHIVE_FINDING_TITLE,
            format!(
                "{} Skills are older than 180 days with no observed use in reliable covered Agent windows. This is candidate evidence only.",
                stale_candidates.len()
            ),
            stale_candidates
                .iter()
                .map(|skill| skill.id.clone())
                .collect(),
            Vec::new(),
            stale_candidates
                .iter()
                .map(|skill| format!("digest:{}", skill.content_digest))
                .collect(),
            EvidenceQuality::Inferred,
        );
    } else if reliable_agents.len() < AgentKind::ALL.len() {
        push_session_finding(
            findings,
            FindingKind::ArchiveCandidacyUnknown,
            FindingCategory::Lifecycle,
            Severity::Info,
            UNKNOWN_ARCHIVE_FINDING_TITLE,
            "Coverage is insufficient to treat missing usage as evidence for archive. No archive action was suggested.",
            Vec::new(),
            Vec::new(),
            scan.coverage
                .iter()
                .map(|coverage| format!("coverage:{}", coverage.agent.id()))
                .collect(),
            EvidenceQuality::Unknown,
        );
    }
}

fn usage_stage_name(stage: UsageStage) -> &'static str {
    match stage {
        UsageStage::Exposed => "Exposed",
        UsageStage::Matched => "Matched",
        UsageStage::Loaded => "Loaded",
        UsageStage::Applied => "Applied",
        UsageStage::Outcome => "Outcome",
    }
}

fn evidence_paths_for_skills(scan: &ScanResult, skills: &[&ScannedSkill]) -> Vec<String> {
    let wanted = skills
        .iter()
        .map(|skill| skill.id.as_str())
        .collect::<BTreeSet<_>>();
    scan.placements
        .iter()
        .filter(|placement| wanted.contains(placement.skill_id.as_str()))
        .map(|placement| format!("path:{}", placement.entrypoint.display()))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn push_finding(
    findings: &mut Vec<Finding>,
    kind: FindingKind,
    category: FindingCategory,
    severity: Severity,
    title: impl Into<String>,
    summary: impl Into<String>,
    affected_skill_ids: Vec<String>,
    affected_placement_ids: Vec<String>,
    evidence: Vec<String>,
    evidence_quality: EvidenceQuality,
) {
    push_finding_with_coverage(
        findings,
        kind,
        category,
        severity,
        title,
        summary,
        affected_skill_ids,
        affected_placement_ids,
        evidence,
        evidence_quality,
        FindingCoverageBasis::SkillRootScan,
    );
}

#[allow(clippy::too_many_arguments)]
fn push_session_finding(
    findings: &mut Vec<Finding>,
    kind: FindingKind,
    category: FindingCategory,
    severity: Severity,
    title: impl Into<String>,
    summary: impl Into<String>,
    affected_skill_ids: Vec<String>,
    affected_placement_ids: Vec<String>,
    evidence: Vec<String>,
    evidence_quality: EvidenceQuality,
) {
    push_finding_with_coverage(
        findings,
        kind,
        category,
        severity,
        title,
        summary,
        affected_skill_ids,
        affected_placement_ids,
        evidence,
        evidence_quality,
        FindingCoverageBasis::SessionUsage,
    );
}

#[allow(clippy::too_many_arguments)]
fn push_finding_with_coverage(
    findings: &mut Vec<Finding>,
    kind: FindingKind,
    category: FindingCategory,
    severity: Severity,
    title: impl Into<String>,
    summary: impl Into<String>,
    mut affected_skill_ids: Vec<String>,
    mut affected_placement_ids: Vec<String>,
    mut evidence: Vec<String>,
    evidence_quality: EvidenceQuality,
    coverage_basis: FindingCoverageBasis,
) {
    affected_skill_ids.sort();
    affected_skill_ids.dedup();
    affected_placement_ids.sort();
    affected_placement_ids.dedup();
    evidence.sort();
    evidence.dedup();
    let title = title.into();
    let id_basis = format!(
        "{}\0{}\0{}\0{}",
        category_name(category),
        kind.as_str(),
        affected_skill_ids.join(","),
        affected_placement_ids.join(",")
    );
    findings.push(Finding {
        id: format!("finding_{}", fnv1a64(id_basis.as_bytes())),
        kind,
        category,
        severity,
        title,
        summary: summary.into(),
        affected_skill_ids,
        affected_placement_ids,
        evidence,
        evidence_quality,
        coverage_basis,
    });
}

fn category_name(category: FindingCategory) -> &'static str {
    match category {
        FindingCategory::Inventory => "inventory",
        FindingCategory::Layout => "layout",
        FindingCategory::Exposure => "exposure",
        FindingCategory::Usage => "usage",
        FindingCategory::Overlap => "overlap",
        FindingCategory::Routing => "routing",
        FindingCategory::Lifecycle => "lifecycle",
    }
}

pub fn find(scan: &ScanResult, task: &str, limit: usize) -> Vec<FindMatch> {
    let query = RetrievalQuery::from_task_and_hints(task, std::iter::empty::<&str>());
    find_matching(scan, &query, limit, None, None)
}

pub(crate) fn find_matching(
    scan: &ScanResult,
    query: &RetrievalQuery,
    limit: usize,
    candidate_ids: Option<&BTreeSet<String>>,
    variant_eligible_ids: Option<&BTreeSet<String>>,
) -> Vec<FindMatch> {
    find_matching_with_evidence(scan, query, limit, candidate_ids, variant_eligible_ids).matches
}

pub(crate) fn find_matching_with_evidence(
    scan: &ScanResult,
    query: &RetrievalQuery,
    limit: usize,
    candidate_ids: Option<&BTreeSet<String>>,
    variant_eligible_ids: Option<&BTreeSet<String>>,
) -> FindMatchingResult {
    let query_text = query.text().trim().to_lowercase();
    if query_text.is_empty() || limit == 0 {
        return FindMatchingResult {
            matches: Vec::new(),
            task_exclusion_effects: TaskExclusionEffects::default(),
        };
    }
    let query_tokens = tokens(&query_text);
    let positive_task_tokens = tokens(&query.positive_task_text);
    let excluded_task_tokens = tokens(
        &query
            .excluded_task_phrases
            .iter()
            .map(|phrase| task_exclusion_body(phrase))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let excluded_only_task_tokens = excluded_task_tokens
        .difference(&positive_task_tokens)
        .cloned()
        .collect::<BTreeSet<_>>();
    let query_phrases = query
        .phrases
        .iter()
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let placement_groups = placements_by_skill(scan);
    let mut variants_by_name = BTreeMap::<String, Vec<String>>::new();
    for skill in scan
        .skills
        .iter()
        .filter(|skill| variant_eligible_ids.is_none_or(|ids| ids.contains(&skill.id)))
    {
        variants_by_name
            .entry(normalize_skill_name(&skill.name))
            .or_default()
            .push(skill.id.clone());
    }
    for variants in variants_by_name.values_mut() {
        variants.sort();
        variants.dedup();
    }
    let mut task_exclusion_effects = Vec::new();
    let mut matches = scan
        .skills
        .iter()
        .filter(|skill| candidate_ids.is_none_or(|ids| ids.contains(&skill.id)))
        .filter_map(|skill| {
            let name = skill.name.to_lowercase();
            let description = skill
                .metadata
                .description
                .as_deref()
                .unwrap_or_default()
                .to_lowercase();
            let (positive_description, excluded_description) =
                description_routing_sections(&description);
            let triggers = skill.metadata.triggers.join(" ").to_lowercase();
            let all_text = skill_search_text(skill).to_lowercase();
            let name_tokens = tokens(&name);
            let trigger_tokens = tokens(&triggers);
            let description_tokens = tokens(&positive_description);
            let excluded_description_tokens = tokens(&excluded_description);
            let all_text_tokens = tokens(&all_text);
            let name_overlap = query_tokens.intersection(&name_tokens).count();
            let trigger_overlap = query_tokens.intersection(&trigger_tokens).count();
            let description_overlap = query_tokens.intersection(&description_tokens).count();
            let task_excluded_name_overlap =
                excluded_only_task_tokens.intersection(&name_tokens).count();
            let task_excluded_trigger_overlap = excluded_only_task_tokens
                .intersection(&trigger_tokens)
                .count();
            let task_excluded_description_overlap = excluded_only_task_tokens
                .intersection(&description_tokens)
                .count();
            if task_excluded_name_overlap > 0
                || task_excluded_trigger_overlap > 0
                || task_excluded_description_overlap > 0
            {
                task_exclusion_effects.push(TaskExclusionEffect {
                    skill_id: skill.id.clone(),
                    name: skill.name.clone(),
                    name_token_count: task_excluded_name_overlap,
                    trigger_token_count: task_excluded_trigger_overlap,
                    description_token_count: task_excluded_description_overlap,
                });
                return None;
            }
            let excluded_description_overlap = query_tokens
                .intersection(&excluded_description_tokens)
                .count();
            let exclusion_penalty_tokens = if excluded_description_overlap >= 2 {
                excluded_description_overlap
            } else {
                0
            };
            let overlap = query_tokens.intersection(&all_text_tokens).count();
            let cjk_description_overlap = query_tokens
                .intersection(&description_tokens)
                .filter(|token| contains_cjk(token))
                .count();
            let cjk_all_text_overlap = query_tokens
                .intersection(&all_text_tokens)
                .filter(|token| contains_cjk(token))
                .count();
            let mut score = name_overlap as f64 * 24.0
                + trigger_overlap as f64 * 18.0
                + description_overlap as f64 * 12.0
                + overlap as f64 * 3.0
                - exclusion_penalty_tokens as f64 * 18.0;
            let mut reasons = Vec::new();
            if query_phrases.contains(&name) {
                score += 100.0;
                reasons.push("exact_name".into());
            } else if query_phrases.iter().any(|phrase| name.contains(phrase)) {
                score += 45.0;
                reasons.push("name_phrase".into());
            }
            if !triggers.is_empty() && query_phrases.iter().any(|phrase| triggers.contains(phrase))
            {
                score += 35.0;
                reasons.push("declared_trigger".into());
            }
            if !positive_description.is_empty()
                && query_phrases
                    .iter()
                    .any(|phrase| positive_description.contains(phrase))
            {
                score += 25.0;
                reasons.push("description_phrase".into());
            }
            if name_overlap > 0 {
                reasons.push(format!("name_tokens:{name_overlap}"));
            }
            if trigger_overlap > 0 {
                reasons.push(format!("trigger_tokens:{trigger_overlap}"));
            }
            if description_overlap > 0 {
                reasons.push(format!("description_tokens:{description_overlap}"));
            }
            if cjk_description_overlap > 0 {
                reasons.push(format!("cjk_description_bigrams:{cjk_description_overlap}"));
            }
            if exclusion_penalty_tokens > 0 {
                reasons.push(format!(
                    "excluded_description_tokens:{exclusion_penalty_tokens}"
                ));
            }
            if overlap > 0 {
                reasons.push(format!("all_text_tokens:{overlap}"));
            }
            if cjk_all_text_overlap > 0 {
                reasons.push(format!("cjk_all_text_bigrams:{cjk_all_text_overlap}"));
            }
            let observed_usage = scan.usage.iter().any(|usage| {
                usage.skill_id == skill.id && usage.quality == EvidenceQuality::Observed
            });
            if observed_usage {
                score += 2.0;
                reasons.push("observed_local_usage".into());
            }
            if score <= 0.0 {
                return None;
            }
            let all_variant_skill_ids = variants_by_name.get(&normalize_skill_name(&name));
            let variant_count = all_variant_skill_ids.map_or(1, Vec::len);
            let variants_truncated = variant_count > 10;
            let variant_skill_ids = if variant_count > 1 {
                std::iter::once(skill.id.clone())
                    .chain(
                        all_variant_skill_ids
                            .into_iter()
                            .flat_map(|variant_ids| variant_ids.iter())
                            .filter(|variant_id| *variant_id != &skill.id)
                            .cloned(),
                    )
                    .take(10)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let placements = placement_groups
                .get(skill.id.as_str())
                .cloned()
                .unwrap_or_default();
            let mut paths = placements
                .iter()
                .map(|placement| placement.entrypoint.display().to_string())
                .collect::<Vec<_>>();
            paths.sort();
            paths.dedup();
            let mut agents = placements
                .iter()
                .filter_map(|placement| placement.agent.map(AgentKind::id))
                .map(str::to_string)
                .collect::<Vec<_>>();
            agents.sort();
            agents.dedup();
            let mut providers = placements
                .iter()
                .filter_map(|placement| placement.provider.clone())
                .collect::<Vec<_>>();
            providers.sort();
            providers.dedup();
            let governable = placements.iter().any(|placement| placement.is_mutable());
            let (owned_by_agent, mutation_scopes) = placement_authority_facts(&placements);
            let variants = if variant_count > 1 {
                variant_skill_ids
                    .iter()
                    .filter_map(|variant_id| {
                        let variant_skill = scan
                            .skills
                            .iter()
                            .find(|candidate| candidate.id == *variant_id)?;
                        let variant_placements = placement_groups
                            .get(variant_id.as_str())
                            .cloned()
                            .unwrap_or_default();
                        let mut paths = variant_placements
                            .iter()
                            .map(|placement| placement.entrypoint.display().to_string())
                            .collect::<Vec<_>>();
                        paths.sort();
                        paths.dedup();
                        let mut agents = variant_placements
                            .iter()
                            .filter_map(|placement| placement.agent.map(AgentKind::id))
                            .map(str::to_owned)
                            .collect::<Vec<_>>();
                        agents.sort();
                        agents.dedup();
                        let mut providers = variant_placements
                            .iter()
                            .filter_map(|placement| placement.provider.clone())
                            .collect::<Vec<_>>();
                        providers.sort();
                        providers.dedup();
                        let (owned_by_agent, mutation_scopes) =
                            placement_authority_facts(&variant_placements);
                        Some(FindVariant {
                            skill_id: variant_id.clone(),
                            paths,
                            agents,
                            roster_state: "unknown".into(),
                            source: variant_skill.metadata.source.clone(),
                            providers,
                            governable: variant_placements
                                .iter()
                                .any(|placement| placement.is_mutable()),
                            owned_by_agent,
                            mutation_scopes,
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            Some(FindMatch {
                rank: 0,
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                score,
                paths,
                agents,
                roster_state: "unknown".into(),
                source: skill.metadata.source.clone(),
                providers,
                governable,
                owned_by_agent,
                mutation_scopes,
                match_reasons: reasons,
                task_channel_rank: None,
                augmented_channel_rank: None,
                ranking_adjustments: Vec::new(),
                evidence_quality: if observed_usage {
                    EvidenceQuality::Observed
                } else {
                    EvidenceQuality::Inferred
                },
                variant_skill_ids,
                variants,
                variant_count,
                variants_truncated,
                variant_finding: None,
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.skill_id.cmp(&right.skill_id))
    });
    let mut capability_indexes: BTreeMap<String, usize> = BTreeMap::new();
    let mut capabilities: Vec<FindMatch> = Vec::new();
    for matched in matches {
        let capability = matched.name.trim().to_lowercase();
        if capability_indexes.contains_key(&capability) {
            continue;
        }
        capability_indexes.insert(capability, capabilities.len());
        capabilities.push(matched);
    }
    for matched in &mut capabilities {
        if matched.variant_count > 1 {
            matched
                .match_reasons
                .push(format!("name_variants:{}", matched.variant_count));
        }
    }
    trim_low_confidence_tail(&mut capabilities, query_tokens.len());
    capabilities.truncate(limit);
    for (index, matched) in capabilities.iter_mut().enumerate() {
        matched.rank = index + 1;
    }
    task_exclusion_effects.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.skill_id.cmp(&right.skill_id))
    });
    let affected_candidate_count = task_exclusion_effects.len();
    let items_truncated = affected_candidate_count > TASK_EXCLUSION_EFFECT_PREVIEW_LIMIT;
    task_exclusion_effects.truncate(TASK_EXCLUSION_EFFECT_PREVIEW_LIMIT);
    FindMatchingResult {
        matches: capabilities,
        task_exclusion_effects: TaskExclusionEffects {
            affected_candidate_count,
            items: task_exclusion_effects,
            items_truncated,
        },
    }
}

pub(crate) fn fuse_retrieval_channels(
    task_matches: Vec<FindMatch>,
    augmented_matches: Vec<FindMatch>,
    task: &RetrievalQuery,
    limit: usize,
) -> Vec<FindMatch> {
    // Find fuses small, already-ranked candidate pools. A large web-search-style
    // offset flattens rank positions enough for weak overlap to beat the Agent's
    // high-ranked capability hint.
    const RECIPROCAL_RANK_OFFSET: f64 = 1.0;
    const AUGMENTED_CHANNEL_WEIGHT: f64 = 3.0;
    const PROTECTED_TASK_MAX_RANK: usize = 3;

    if limit == 0 {
        return Vec::new();
    }

    struct FusedMatch {
        matched: FindMatch,
        task_rank: Option<usize>,
        augmented_rank: Option<usize>,
        fused_score: f64,
        has_protectable_task_evidence: bool,
    }

    let mut fused = BTreeMap::<String, FusedMatch>::new();
    for matched in task_matches {
        let capability = matched.name.trim().to_lowercase();
        let rank = matched.rank;
        let has_protectable_task_evidence = has_protectable_task_evidence(&matched);
        fused.insert(
            capability,
            FusedMatch {
                matched,
                task_rank: Some(rank),
                augmented_rank: None,
                fused_score: 1.0 / (RECIPROCAL_RANK_OFFSET + rank as f64),
                has_protectable_task_evidence,
            },
        );
    }
    for mut matched in augmented_matches {
        let capability = matched.name.trim().to_lowercase();
        let rank = matched.rank;
        if let Some(existing) = fused.get_mut(&capability) {
            matched
                .match_reasons
                .extend(existing.matched.match_reasons.iter().cloned());
            matched.match_reasons.sort();
            matched.match_reasons.dedup();
            existing.matched = matched;
            existing.augmented_rank = Some(rank);
            existing.fused_score +=
                AUGMENTED_CHANNEL_WEIGHT / (RECIPROCAL_RANK_OFFSET + rank as f64);
        } else {
            fused.insert(
                capability,
                FusedMatch {
                    matched,
                    task_rank: None,
                    augmented_rank: Some(rank),
                    fused_score: AUGMENTED_CHANNEL_WEIGHT / (RECIPROCAL_RANK_OFFSET + rank as f64),
                    has_protectable_task_evidence: false,
                },
            );
        }
    }

    let mut fused = fused.into_values().collect::<Vec<_>>();
    fused.sort_by(|left, right| {
        right
            .fused_score
            .partial_cmp(&left.fused_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                left.augmented_rank
                    .unwrap_or(usize::MAX)
                    .cmp(&right.augmented_rank.unwrap_or(usize::MAX))
            })
            .then_with(|| {
                left.task_rank
                    .unwrap_or(usize::MAX)
                    .cmp(&right.task_rank.unwrap_or(usize::MAX))
            })
            .then_with(|| left.matched.name.cmp(&right.matched.name))
            .then_with(|| left.matched.skill_id.cmp(&right.matched.skill_id))
    });
    let leading_match_is_weak_augmented_only = fused.first().is_some_and(|matched| {
        matched.task_rank.is_none() && !has_direct_hint_evidence(&matched.matched)
    });
    let protected_task_capabilities = fused
        .iter()
        .filter(|fused| fused.task_rank == Some(1) && fused.has_protectable_task_evidence)
        .map(|fused| fused.matched.name.trim().to_lowercase())
        .collect::<BTreeSet<_>>();
    let mut fused = fused
        .into_iter()
        .map(|mut fused| {
            fused.matched.match_reasons.sort();
            fused.matched.match_reasons.dedup();
            fused.matched.task_channel_rank = fused.task_rank;
            fused.matched.augmented_channel_rank = fused.augmented_rank;
            fused.matched.score = (fused.fused_score * 100_000.0).round() / 100.0;
            fused.matched
        })
        .collect::<Vec<_>>();
    let protected_task_matches = fused
        .iter()
        .filter(|matched| protected_task_capabilities.contains(&matched.name.trim().to_lowercase()))
        .cloned()
        .collect::<Vec<_>>();
    // This order defines the stable ranking: trim the fixed pool, restore and
    // promote the strongest task match, then take the requested prefix.
    trim_low_confidence_tail(&mut fused, tokens(task.text()).len());
    for matched in protected_task_matches {
        if !fused
            .iter()
            .any(|candidate| candidate.name.eq_ignore_ascii_case(&matched.name))
        {
            fused.push(matched);
        }
    }
    for protected in &protected_task_capabilities {
        let Some(index) = fused
            .iter()
            .position(|matched| matched.name.trim().eq_ignore_ascii_case(protected))
        else {
            continue;
        };
        let protected_rank_index = if leading_match_is_weak_augmented_only {
            0
        } else {
            PROTECTED_TASK_MAX_RANK - 1
        };
        if index > protected_rank_index {
            let matched = fused.remove(index);
            fused.insert(protected_rank_index, matched);
            fused[protected_rank_index]
                .ranking_adjustments
                .push(RankingAdjustment::ProtectedOriginalTaskMatch);
        }
    }
    fused.truncate(limit);
    for (index, matched) in fused.iter_mut().enumerate() {
        matched.rank = index + 1;
    }
    fused
}

fn tokens(text: &str) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    let mut word = String::new();
    let mut cjk_run = Vec::new();
    for character in text.chars() {
        if is_cjk(character) {
            insert_word_token(&mut output, &mut word);
            cjk_run.push(character);
        } else if character.is_alphanumeric() {
            insert_cjk_bigrams(&mut output, &mut cjk_run);
            word.push(character);
        } else {
            insert_word_token(&mut output, &mut word);
            insert_cjk_bigrams(&mut output, &mut cjk_run);
        }
    }
    insert_word_token(&mut output, &mut word);
    insert_cjk_bigrams(&mut output, &mut cjk_run);
    output
}

fn insert_word_token(output: &mut BTreeSet<String>, word: &mut String) {
    if word.chars().count() >= 2 {
        let normalized = normalize_token(word.trim());
        if !is_search_stopword(&normalized) {
            output.insert(normalized);
        }
    }
    word.clear();
}

fn insert_cjk_bigrams(output: &mut BTreeSet<String>, run: &mut Vec<char>) {
    for pair in run.windows(2) {
        let token = pair.iter().collect::<String>();
        if !is_search_stopword(&token) {
            output.insert(token);
        }
    }
    run.clear();
}

pub(crate) fn contains_cjk(text: &str) -> bool {
    text.chars().any(is_cjk)
}

fn is_cjk(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0x20000..=0x2fa1f
    )
}

fn is_search_stopword(token: &str) -> bool {
    matches!(
        token,
        "an" | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "by"
            | "for"
            | "from"
            | "in"
            | "into"
            | "is"
            | "it"
            | "not"
            | "of"
            | "on"
            | "or"
            | "that"
            | "the"
            | "this"
            | "to"
            | "use"
            | "using"
            | "with"
            | "一个"
            | "一份"
            | "一下"
            | "一点"
            | "什么"
            | "使用"
            | "创建"
            | "可以"
            | "已经"
            | "当前"
            | "帮我"
            | "怎么"
            | "需要"
            | "看看"
            | "这个"
            | "那个"
            | "进行"
    )
}

pub(crate) fn candidate_search_text(text: &str) -> String {
    let mut seen = BTreeSet::new();
    let mut terms = Vec::new();
    for term in text
        .split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|term| !term.is_empty())
    {
        let original = term.to_lowercase();
        if seen.insert(original.clone()) {
            terms.push(original.clone());
        }
        let normalized = normalize_token(&original);
        if seen.insert(normalized.clone()) {
            terms.push(normalized);
        }
    }
    terms.join(" ")
}

fn normalize_token(token: &str) -> String {
    let mut normalized = token.to_lowercase();
    if normalized.is_ascii()
        && normalized.len() > 3
        && normalized.ends_with('s')
        && !normalized.ends_with("ss")
        && !normalized.ends_with("us")
        && !normalized.ends_with("is")
    {
        normalized.pop();
    }
    normalized
}

fn description_routing_sections(description: &str) -> (String, String) {
    const EXCLUSION_MARKERS: &[&str] = &[
        "do not use",
        "don't use",
        "never use",
        "must not use",
        "should not use",
        "not for",
        "不应触发",
    ];
    let description = description.to_lowercase();
    let mut positive = Vec::new();
    let mut excluded = Vec::new();
    for section in description.split(['.', '!', '?', ';', '\n', '。', '！', '？', '；']) {
        let section = section.trim();
        if section.is_empty() {
            continue;
        }
        if let Some(boundary) = EXCLUSION_MARKERS
            .iter()
            .filter_map(|marker| section.find(marker))
            .min()
        {
            let desired = section[..boundary].trim().trim_end_matches(',').trim();
            if !desired.is_empty() {
                positive.push(desired);
            }
            excluded.push(section[boundary..].trim());
        } else {
            positive.push(section);
        }
    }
    (positive.join(" "), excluded.join(" "))
}

fn task_routing_sections(task: &str) -> (String, Vec<String>) {
    let mut positive = Vec::new();
    let mut excluded = Vec::new();
    for section in task.split(['.', '!', '?', ';', ',', '\n', '。', '！', '？', '；', '，']) {
        let section = section.trim();
        if section.is_empty() {
            continue;
        }
        if task_exclusion_marker(section).is_some() {
            excluded.push(section.to_owned());
        } else {
            positive.push(section);
        }
    }
    if excluded.is_empty() {
        (task.trim().to_owned(), excluded)
    } else {
        (positive.join(" "), excluded)
    }
}

fn task_exclusion_body(section: &str) -> &str {
    task_exclusion_marker(section).map_or(section, |marker| section[marker.len()..].trim())
}

fn task_exclusion_marker(section: &str) -> Option<&'static str> {
    TASK_EXCLUSION_MARKERS.iter().copied().find(|marker| {
        if marker.is_ascii() {
            section
                .get(..marker.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(marker))
                && section[marker.len()..]
                    .chars()
                    .next()
                    .is_none_or(|next| !next.is_alphanumeric())
        } else {
            section.starts_with(marker)
        }
    })
}

fn trim_low_confidence_tail(matches: &mut Vec<FindMatch>, query_token_count: usize) {
    if matches.is_empty() {
        return;
    }
    let cutoff = (matches[0].score * 0.5).max(3.0);
    matches.retain(|matched| matched.score >= cutoff);
    if query_token_count == 1
        && matches
            .first()
            .is_some_and(|matched| !has_strong_lexical_evidence(matched))
    {
        matches.truncate(3);
    }
}

fn has_strong_lexical_evidence(matched: &FindMatch) -> bool {
    matched.match_reasons.iter().any(|reason| {
        matches!(
            reason.as_str(),
            "exact_name" | "name_phrase" | "declared_trigger" | "description_phrase"
        ) || reason.starts_with("name_tokens:")
            || reason.starts_with("trigger_tokens:")
            || reason.starts_with("description_tokens:")
    })
}

fn has_protectable_task_evidence(matched: &FindMatch) -> bool {
    let has_direct_metadata_evidence = has_direct_metadata_reason(matched)
        || ["name_tokens:", "trigger_tokens:", "description_tokens:"]
            .iter()
            .any(|prefix| match_reason_count(matched, prefix).is_some_and(|count| count >= 2));
    let has_correlated_cjk_evidence = match_reason_count(matched, "cjk_description_bigrams:")
        .is_some_and(|count| count >= 1)
        && match_reason_count(matched, "cjk_all_text_bigrams:").is_some_and(|count| count >= 3);
    has_direct_metadata_evidence || has_correlated_cjk_evidence
}

fn has_direct_hint_evidence(matched: &FindMatch) -> bool {
    let has_complete_single_token_name = tokens(&matched.name).len() == 1
        && match_reason_count(matched, "name_tokens:").is_some_and(|count| count == 1);
    has_direct_metadata_reason(matched)
        || has_complete_single_token_name
        || match_reason_count(matched, "trigger_tokens:").is_some_and(|count| count >= 2)
}

fn has_direct_metadata_reason(matched: &FindMatch) -> bool {
    matched.match_reasons.iter().any(|reason| {
        matches!(
            reason.as_str(),
            "exact_name" | "name_phrase" | "declared_trigger" | "description_phrase"
        )
    })
}

fn match_reason_count(matched: &FindMatch, prefix: &str) -> Option<usize> {
    matched.match_reasons.iter().find_map(|reason| {
        reason
            .strip_prefix(prefix)
            .and_then(|count| count.parse::<usize>().ok())
    })
}

fn fnv1a64(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finding_identity_and_behavior_do_not_depend_on_display_title() {
        let mut findings = Vec::new();
        for title in ["Original title", "Copy-edited title"] {
            push_finding(
                &mut findings,
                FindingKind::ExactDuplicatePlacements,
                FindingCategory::Overlap,
                Severity::Medium,
                title,
                "same facts",
                vec!["skill_one".into()],
                vec!["placement_one".into()],
                Vec::new(),
                EvidenceQuality::Observed,
            );
        }

        assert_eq!(findings[0].id, findings[1].id);
        assert_eq!(findings[0].kind, findings[1].kind);
        assert_ne!(findings[0].title, findings[1].title);
    }

    #[test]
    fn stored_finding_kind_prefers_typed_details_and_bounds_legacy_title_fallback() {
        let mut finding = FindingRecord {
            id: crate::model::FindingId::new(),
            report_id: crate::model::ReportId::new(),
            category: crate::model::FindingCategory::Exposure,
            severity: crate::model::Severity::Warning,
            title: "A translated display title".into(),
            summary: String::new(),
            details: serde_json::json!({"kind": "large_default_roster"}),
            evidence_ids: Vec::new(),
        };

        assert!(stored_finding_is(&finding, FindingKind::LargeDefaultRoster));
        finding.details = serde_json::json!({"kind": null});
        finding.title = "Large default Rosters need review".into();
        assert!(stored_finding_is(&finding, FindingKind::LargeDefaultRoster));
        finding.details = serde_json::json!({"kind": 7});
        assert!(!stored_finding_is(
            &finding,
            FindingKind::LargeDefaultRoster
        ));
        finding.details = serde_json::json!({});
        finding.title = "A translated display title".into();
        assert!(!stored_finding_is(
            &finding,
            FindingKind::LargeDefaultRoster
        ));
        finding.title = "Large default Rosters need review".into();
        assert!(stored_finding_is(&finding, FindingKind::LargeDefaultRoster));
    }

    fn matched(name: &str, rank: usize, reasons: &[&str]) -> FindMatch {
        FindMatch {
            rank,
            skill_id: format!("skill_{name}"),
            name: name.into(),
            score: 1.0,
            paths: Vec::new(),
            agents: Vec::new(),
            roster_state: "unknown".into(),
            source: None,
            providers: Vec::new(),
            governable: true,
            owned_by_agent: Some(true),
            mutation_scopes: vec![crate::scan::MutationScope::Mutable],
            match_reasons: reasons.iter().map(|reason| (*reason).into()).collect(),
            task_channel_rank: None,
            augmented_channel_rank: None,
            ranking_adjustments: Vec::new(),
            evidence_quality: EvidenceQuality::Inferred,
            variant_skill_ids: Vec::new(),
            variants: Vec::new(),
            variant_count: 1,
            variants_truncated: false,
            variant_finding: None,
        }
    }

    #[test]
    fn inventory_coverage_ignores_session_root_availability() {
        let root = |kind| crate::scan::RootObservation {
            agent: Some(AgentKind::Codex),
            kind,
            path: PathBuf::from("/fixture/unavailable"),
            status: crate::scan::RootStatus::Inaccessible,
            explicit: false,
            detail: None,
            discovery_complete: true,
        };
        let mut scan = ScanResult {
            roots: vec![root(crate::scan::RootKind::Sessions)],
            ..ScanResult::default()
        };
        let session_only = build_report(&scan);
        assert!(
            session_only
                .findings
                .iter()
                .all(|finding| finding.title != "Some configured roots were inaccessible")
        );

        scan.roots.push(root(crate::scan::RootKind::Skills));
        let with_skill_root = build_report(&scan);
        let inventory = with_skill_root
            .findings
            .iter()
            .find(|finding| finding.title == "Some configured roots were inaccessible")
            .unwrap();
        assert_eq!(
            inventory.coverage_basis,
            FindingCoverageBasis::SkillRootScan
        );
    }

    #[test]
    fn inventory_reports_included_roots_with_bounded_discovery() {
        let scan = ScanResult {
            roots: vec![crate::scan::RootObservation {
                agent: Some(AgentKind::Codex),
                kind: crate::scan::RootKind::Skills,
                path: PathBuf::from("/fixture/bounded"),
                status: crate::scan::RootStatus::Included,
                explicit: false,
                detail: Some("Skill discovery was bounded at depth 5".into()),
                discovery_complete: false,
            }],
            ..ScanResult::default()
        };

        let report = build_report(&scan);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.title == "Some configured roots had bounded discovery")
            .unwrap();

        assert_eq!(finding.coverage_basis, FindingCoverageBasis::SkillRootScan);
        assert_eq!(finding.evidence_quality, EvidenceQuality::Observed);
    }
    use crate::scan::{ScanOptions, scan};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> (PathBuf, ScanResult) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skillroster-query-{nonce}"));
        for (directory, contents) in [
            (
                "research-a",
                "---\nname: research\ndescription: Search and verify primary sources\ntriggers: [research, verify]\n---\nInvestigate facts.",
            ),
            (
                "research-b",
                "---\nname: research\ndescription: Search and verify primary sources\ntriggers: [research, verify]\n---\nInvestigate facts.",
            ),
            ("unknown", "---\nname: mystery\n---\nNarrow operation."),
        ] {
            let path = root.join(directory);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("SKILL.md"), contents).unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: crate::harness::AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let scan = scan(&options).unwrap();
        (root, scan)
    }

    #[test]
    fn report_distinguishes_exact_duplicates_and_unknown_usage() {
        let (root, scan) = fixture();
        let report = build_report(&scan);
        assert!(report.findings.iter().any(|finding| {
            finding.category == FindingCategory::Overlap
                && finding.evidence_quality == EvidenceQuality::Observed
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.category == FindingCategory::Usage
                && finding.evidence_quality == EvidenceQuality::Unknown
        }));
        let usage = report
            .findings
            .iter()
            .find(|finding| finding.title == "Five-stage usage evidence")
            .unwrap();
        assert!(
            usage
                .summary
                .contains(&format!("Exposed={}", scan.placements.len()))
        );
        for stage in ["Matched", "Loaded", "Applied", "Outcome"] {
            assert!(usage.summary.contains(&format!("{stage}=0")));
        }
        let overview = usage_overview(&scan);
        assert_eq!(overview.stages.len(), 5);
        assert_eq!(overview.stages[0].stage, UsageStage::Exposed);
        assert_eq!(overview.stages[0].count, scan.placements.len() as u64);
        assert_eq!(overview.stages[0].unit, UsageCountUnit::Placements);
        assert_eq!(overview.stages[0].first_seen_unix, None);
        assert_eq!(overview.stages[0].last_seen_unix, None);
        assert!(
            overview.stages[1..]
                .iter()
                .all(|stage| stage.unit == UsageCountUnit::Events)
        );
        assert_eq!(
            overview.coverage.supported_agent_count,
            AgentKind::ALL.len()
        );
        assert!(report.findings.iter().any(|finding| {
            finding.title == "Archive candidacy is unknown"
                && finding.evidence_quality == EvidenceQuality::Unknown
        }));
        assert!(!report.files_changed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn usage_overview_groups_repeated_agent_skill_stage_signals() {
        let (root, mut scan) = fixture();
        let skill_id = scan.skills[0].id.clone();
        let skill_name = scan.skills[0].name.clone();
        for (event_count, quality, last_seen_unix) in [
            (2, EvidenceQuality::Inferred, Some(10)),
            (3, EvidenceQuality::Observed, Some(20)),
        ] {
            scan.usage.push(crate::scan::UsageEvidence {
                agent: AgentKind::Codex,
                skill_id: skill_id.clone(),
                stage: UsageStage::Loaded,
                quality,
                event_count,
                first_seen_unix: Some(1),
                last_seen_unix,
                month_start_unix: None,
                source_path_digest: format!("source-{event_count}"),
            });
        }
        let mut same_name_variant = scan.skills[0].clone();
        same_name_variant.id = "skill_distinct-same-name".into();
        scan.skills.push(same_name_variant.clone());
        scan.usage.push(crate::scan::UsageEvidence {
            agent: AgentKind::Codex,
            skill_id: same_name_variant.id.clone(),
            stage: UsageStage::Loaded,
            quality: EvidenceQuality::Observed,
            event_count: 7,
            first_seen_unix: Some(2),
            last_seen_unix: Some(30),
            month_start_unix: None,
            source_path_digest: "source-distinct".into(),
        });

        let overview = usage_overview(&scan);

        assert_eq!(overview.observed_signal_count, 2);
        assert_eq!(overview.observed_skills.len(), 2);
        let repeated_source_signal = overview
            .observed_skills
            .iter()
            .find(|signal| signal.skill_id == skill_id)
            .expect("original Skill identity");
        assert_eq!(repeated_source_signal.skill_name, skill_name);
        assert_eq!(repeated_source_signal.event_count, 5);
        assert_eq!(repeated_source_signal.quality, EvidenceQuality::Observed);
        assert_eq!(repeated_source_signal.last_seen_unix, Some(20));
        let distinct_identity = overview
            .observed_skills
            .iter()
            .find(|signal| signal.skill_id == same_name_variant.id)
            .expect("same-name distinct Skill identity");
        assert_eq!(distinct_identity.skill_name, skill_name);
        assert_eq!(distinct_identity.event_count, 7);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exact_duplicate_ignores_one_physical_source_shared_by_valid_links() {
        let (root, mut scan) = fixture();
        let source = scan
            .placements
            .iter()
            .find(|placement| placement.directory.ends_with("research-a"))
            .unwrap()
            .clone();
        let mut direct_link = source.clone();
        direct_link.id = "placement_direct_link".into();
        direct_link.directory = root.join("linked-direct");
        direct_link.entrypoint = direct_link.directory.join("SKILL.md");
        direct_link.link_target = Some(source.directory.clone());
        direct_link.link_status = LinkStatus::Valid;
        let mut indirect_link = source.clone();
        indirect_link.id = "placement_indirect_link".into();
        indirect_link.directory = root.join("linked-indirect");
        indirect_link.entrypoint = indirect_link.directory.join("SKILL.md");
        indirect_link.link_target = Some(direct_link.directory.clone());
        indirect_link.link_status = LinkStatus::Valid;
        scan.placements
            .retain(|placement| placement.skill_id != source.skill_id || placement.id == source.id);
        scan.placements.push(direct_link);
        scan.placements.push(indirect_link);

        let report = build_report(&scan);

        assert!(!report.findings.iter().any(|finding| {
            finding.title == "Exact duplicate Skill placements"
                && finding.affected_skill_ids.contains(&source.skill_id)
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exact_duplicate_keeps_multiple_physical_sources_with_shared_links() {
        let (root, mut scan) = fixture();
        let source = scan
            .placements
            .iter()
            .find(|placement| placement.directory.ends_with("research-a"))
            .unwrap()
            .clone();
        let mut shared_link = source.clone();
        shared_link.id = "placement_shared_link".into();
        shared_link.directory = root.join("linked-copy");
        shared_link.entrypoint = shared_link.directory.join("SKILL.md");
        shared_link.link_target = Some(source.directory.clone());
        shared_link.link_status = LinkStatus::Valid;
        scan.placements.push(shared_link);

        let report = build_report(&scan);
        let finding = report
            .findings
            .iter()
            .find(|finding| {
                finding.title == "Exact duplicate Skill placements"
                    && finding.affected_skill_ids.contains(&source.skill_id)
            })
            .unwrap();

        assert_eq!(finding.affected_placement_ids.len(), 3);
        assert!(finding.summary.contains("2 distinct physical sources"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_package_fingerprints_never_become_exact_duplicates() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skillroster-bounded-duplicate-{nonce}"));
        for (name, bytes) in [
            ("copy-a", 16 * 1024 * 1024 + 1),
            ("copy-b", 16 * 1024 * 1024 + 2),
        ] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join("SKILL.md"),
                "---\nname: bounded-copy\n---\nSame entrypoint.",
            )
            .unwrap();
            fs::File::create(directory.join("asset.bin"))
                .unwrap()
                .set_len(bytes)
                .unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let scan = scan(&options).unwrap();

        assert_eq!(scan.placements.len(), 2);
        assert!(scan.placements.iter().all(|placement| {
            placement.fingerprint_completeness == crate::scan::FingerprintCompleteness::Bounded
        }));
        let report = build_report(&scan);
        let incomplete = report
            .findings
            .iter()
            .find(|finding| finding.title == "Some Skill package fingerprints are incomplete")
            .unwrap();
        assert_eq!(incomplete.affected_placement_ids.len(), 2);
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| { finding.title == "Exact duplicate Skill placements" })
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn semantic_overlap_is_candidate_evidence_and_never_a_confirmed_duplicate() {
        let (root, mut scan) = fixture();
        let original = scan.skills[0].clone();
        let original_id = original.id.clone();
        let mut candidate = original.clone();
        candidate.id = "skill_semantic_candidate".into();
        let candidate_id = candidate.id.clone();
        candidate.name = "source-check".into();
        candidate.content_digest = "different_digest".into();
        let basis = semantic_overlap_basis(&original, &candidate).unwrap();
        assert_eq!(basis.metric, "routing_vocabulary_jaccard");
        assert!(basis.score >= 0.45);
        assert!(basis.intersection_count >= 3);
        assert!(basis.union_count >= basis.intersection_count);
        assert!(!basis.shared_terms.is_empty());
        scan.skills = vec![original, candidate];
        scan.placements.clear();
        scan.usage.clear();

        let report = build_report(&scan);
        let finding = report
            .findings
            .iter()
            .find(|finding| {
                finding.title == "Semantic overlap candidate"
                    && finding.affected_skill_ids.contains(&original_id)
                    && finding.affected_skill_ids.contains(&candidate_id)
            })
            .unwrap();
        assert_eq!(finding.evidence_quality, EvidenceQuality::Inferred);
        assert!(finding.summary.contains("review-only candidate evidence"));
        assert!(finding.summary.contains("not a confirmed duplicate"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn semantic_overlap_excludes_same_name_variants() {
        let (root, mut scan) = fixture();
        let original = scan.skills[0].clone();
        let mut same_name_variant = original.clone();
        same_name_variant.id = "skill_same_name_variant".into();
        same_name_variant.name = format!("  {}  ", original.name.to_uppercase());
        same_name_variant.content_digest = "different_digest".into();
        same_name_variant.content_identity_digest = Some("different_identity_digest".into());
        scan.skills.push(same_name_variant.clone());

        let report = build_report(&scan);

        assert!(report.findings.iter().any(|finding| {
            finding.title == SAME_NAME_DIVERGENT_FINDING_TITLE
                && finding.affected_skill_ids.contains(&original.id)
                && finding.affected_skill_ids.contains(&same_name_variant.id)
        }));
        assert!(!report.findings.iter().any(|finding| {
            finding.title == "Semantic overlap candidate"
                && finding.affected_skill_ids.contains(&original.id)
                && finding.affected_skill_ids.contains(&same_name_variant.id)
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn semantic_overlap_work_stays_bounded_for_a_realistic_inventory() {
        let (_, mut scan) = fixture();
        let representative = scan.skills[0].clone();
        let shared_body = std::iter::repeat_n(
            "shared architecture workflow browser database agent skill governance local evidence",
            500,
        )
        .collect::<Vec<_>>()
        .join(" ");
        scan.skills = (0..193)
            .map(|index| {
                let mut skill = representative.clone();
                skill.id = format!("realistic-{index:03}");
                skill.name = format!("realistic-{index:03}");
                skill.content_digest = format!("digest-{index:03}");
                skill.normalized_text = format!("{shared_body} unique-{index:03}");
                skill
            })
            .collect();
        scan.placements.clear();
        scan.usage.clear();

        let mut findings = Vec::new();
        let (vocabulary_count, pair_comparison_count) = overlap_findings(&scan, &mut findings);

        assert_eq!(vocabulary_count, 193);
        assert_eq!(pair_comparison_count, 193 * 192 / 2);
        assert_eq!(
            findings
                .iter()
                .filter(|finding| finding.title == "Semantic overlap candidate")
                .count(),
            25
        );
    }

    #[test]
    fn sourced_skill_reports_local_revision_but_keeps_upstream_drift_unknown() {
        let (root, mut scan) = fixture();
        scan.skills[0].metadata.source = Some("github:owner/repo".into());
        scan.skills[0].metadata.version = Some("v1.2.3".into());
        let report = build_report(&scan);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.title == "Upstream update drift is not verified")
            .unwrap();
        assert_eq!(finding.evidence_quality, EvidenceQuality::Unknown);
        assert!(finding.summary.contains("did not query upstream state"));
        assert!(
            finding
                .evidence
                .iter()
                .any(|item| item.starts_with("digest:"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn report_exposes_issue_nine_structural_and_version_findings() {
        let (root, mut scan) = fixture();
        let first_id = scan.skills[0].id.clone();
        let first_placement = scan
            .placements
            .iter_mut()
            .find(|placement| placement.skill_id == first_id)
            .unwrap();
        first_placement.executable_files = vec![first_placement.directory.join("run.sh")];
        first_placement.declared_name_matches_directory = Some(false);
        scan.skills[0].metadata.source = Some("github:owner/repo".into());
        scan.skills[0].metadata.version = Some("v1".into());

        let mut divergent = scan.skills[0].clone();
        divergent.id = "skill_divergent_version".into();
        divergent.metadata.version = Some("v2".into());
        divergent.content_digest = "different-version-digest".into();
        let mut divergent_placement = scan.placements[0].clone();
        divergent_placement.id = "placement_divergent_version".into();
        divergent_placement.skill_id = divergent.id.clone();
        divergent_placement.content_digest = divergent.content_digest.clone();
        scan.skills.push(divergent);
        scan.placements.push(divergent_placement);

        let report = build_report(&scan);
        for title in [
            "Skill packages contain executable scripts",
            "Declared Skill names differ from placement directories",
            "Declared source has version divergence",
            "Upstream update drift is not verified",
        ] {
            let finding = report
                .findings
                .iter()
                .find(|finding| finding.title == title)
                .unwrap_or_else(|| panic!("missing Finding: {title}"));
            assert!(!finding.evidence.is_empty());
            assert!(!finding.affected_skill_ids.is_empty());
        }
        let scripts = report
            .findings
            .iter()
            .find(|finding| finding.title == "Skill packages contain executable scripts")
            .unwrap();
        assert!(scripts.summary.contains("does not establish"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_candidate_requires_old_content_and_reliable_agent_coverage() {
        let (root, mut scan) = fixture();
        let skill_id = scan.skills[0].id.clone();
        scan.skills[0].modified_at_unix = Some(1);
        let placement = scan
            .placements
            .iter_mut()
            .find(|placement| placement.skill_id == skill_id)
            .unwrap();
        placement.agent = Some(AgentKind::Codex);
        scan.coverage.push(crate::scan::SessionCoverage {
            agent: AgentKind::Codex,
            roots_present: 1,
            roots_missing: 0,
            roots_inaccessible: 0,
            files_discovered: 2,
            files_observed: 2,
            files_partially_observed: 0,
            files_skipped: 0,
            denominator_reliable: true,
            bytes_observed: 100,
            lines_observed: 10,
            truncated: false,
            discovery_truncated: false,
            first_seen_unix: Some(10),
            last_seen_unix: Some(20),
            limitations: Some(Vec::new()),
        });

        let report = build_report(&scan);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.title == "Stale archive candidates require review")
            .unwrap();
        assert_eq!(finding.evidence_quality, EvidenceQuality::Inferred);
        assert!(finding.summary.contains("candidate evidence only"));

        scan.coverage[0].limitations = None;
        let legacy_report = build_report(&scan);
        assert_eq!(
            legacy_report
                .metrics
                .agents_with_reliable_session_denominator,
            0
        );
        assert!(
            !legacy_report
                .findings
                .iter()
                .any(|finding| finding.title == "Stale archive candidates require review")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn find_ranks_declared_trigger_and_returns_evidence_paths() {
        let (root, scan) = fixture();
        let matches = find(&scan, "verify", 3);
        assert_eq!(matches[0].name, "research");
        assert!(
            matches[0]
                .match_reasons
                .contains(&"declared_trigger".into())
        );
        assert_eq!(matches[0].paths.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn find_keeps_hint_phrases_separate_from_the_preserved_task() {
        let (root, scan) = fixture();
        let query = RetrievalQuery::from_parts(["调查事实", "verify"]);
        let matches = find_matching(&scan, &query, 3, None, None);

        assert_eq!(matches[0].name, "research");
        assert!(
            matches[0]
                .match_reasons
                .contains(&"declared_trigger".into())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn find_prefers_task_terms_in_name_over_incidental_body_mentions() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skillroster-routing-fields-{nonce}"));
        for (directory, contents) in [
            (
                "aaa-general-agent",
                "---\nname: aaa-general-agent\ndescription: General coding assistant\n---\nCan also discuss database migration among many unrelated tasks.",
            ),
            (
                "database-migration",
                "---\nname: database-migration\ndescription: Plan and review safe schema changes\n---\nUse for production data changes.",
            ),
            (
                "review",
                "---\nname: review\ndescription: Review code\n---\nGeneric review helper.",
            ),
            (
                "github-code-review",
                "---\nname: github-code-review\ndescription: Review pull requests and publish inline comments\n---\nInspect a pull request diff.",
            ),
        ] {
            let path = root.join(directory);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("SKILL.md"), contents).unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: crate::harness::AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let scan = scan(&options).unwrap();

        let matches = find(&scan, "database migration", 2);

        assert_eq!(matches[0].name, "database-migration");
        assert!(matches[0].match_reasons.contains(&"name_tokens:2".into()));
        let review_matches = find(&scan, "review a pull request", 2);
        assert_eq!(review_matches[0].name, "github-code-review");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn find_prefers_dedicated_surfaces_and_removes_a_low_confidence_tail() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skillroster-routing-quality-{nonce}"));
        for (directory, contents) in [
            (
                "presentations",
                "---\nname: Presentations\ndescription: Read, create, or edit PowerPoint and Google Slides decks. Use for presentation and slide requests.\n---\nDedicated deck workflow.",
            ),
            (
                "template-creator",
                "---\nname: template-creator\ndescription: Create a reusable template from a reference presentation or spreadsheet. Do not use for one-off creation.\n---\nGeneric artifact template workflow.",
            ),
            (
                "spreadsheets",
                "---\nname: Spreadsheets\ndescription: Create, edit, and analyze standalone spreadsheet files and workbooks. Do not use for a live Microsoft Excel session.\n---\nDedicated file workflow.",
            ),
            (
                "excel-live-control",
                "---\nname: excel-live-control\ndescription: Control an open Microsoft Excel workbook in a connected live session. Do not use for standalone spreadsheet files.\n---\nDedicated live application workflow.",
            ),
            (
                "control-browser",
                "---\nname: control-browser\ndescription: Control browser automation in an existing logged-in session.\n---\nInspect and operate web pages.",
            ),
            (
                "agent-session-miner",
                "---\nname: agent-session-miner\ndescription: Mine local agent session history.\n---\nA browser can appear incidentally in session logs.",
            ),
        ] {
            let path = root.join(directory);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("SKILL.md"), contents).unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: crate::harness::AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let scan = scan(&options).unwrap();

        let presentation = find(&scan, "create product release presentation slides", 10);
        assert_eq!(presentation[0].name, "Presentations");

        let spreadsheet = find(
            &scan,
            "analyze a standalone spreadsheet file and workbook",
            10,
        );
        assert_eq!(spreadsheet[0].name, "Spreadsheets");
        assert!(
            spreadsheet
                .iter()
                .all(|matched| matched.name != "excel-live-control")
        );

        let browser = find(
            &scan,
            "control browser automation in a logged-in session",
            10,
        );
        assert_eq!(browser[0].name, "control-browser");
        assert!(
            browser
                .iter()
                .all(|matched| matched.name != "agent-session-miner")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hint_fusion_preserves_a_strong_task_match_missing_from_the_augmented_pool() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skillroster-hint-fusion-{nonce}"));
        for (directory, contents) in [
            (
                "native-task",
                "---\nname: native-task\ndescription: 原始任务专用能力\n---\n",
            ),
            (
                "hint-one",
                "---\nname: hint-one\ndescription: English capability paraphrase\n---\n",
            ),
            (
                "hint-two",
                "---\nname: hint-two\ndescription: English capability paraphrase helper\n---\n",
            ),
            (
                "hint-three",
                "---\nname: hint-three\ndescription: English capability paraphrase workflow\n---\n",
            ),
        ] {
            let path = root.join(directory);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("SKILL.md"), contents).unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: crate::harness::AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let scan = scan(&options).unwrap();

        let task_query = RetrievalQuery::from_parts(["原始任务专用能力"]);
        let task_matches = find_matching(&scan, &task_query, 3, None, None);
        let augmented_matches = find(&scan, "English capability paraphrase", 3);
        assert_eq!(task_matches[0].name, "native-task");
        assert!(
            augmented_matches
                .iter()
                .all(|matched| matched.name != "native-task")
        );

        let fused = fuse_retrieval_channels(task_matches, augmented_matches, &task_query, 3);
        let native = fused
            .iter()
            .find(|matched| matched.name == "native-task")
            .expect("a strong original-task match must remain in the bounded result");
        assert_eq!(native.task_channel_rank, Some(1));
        assert_eq!(native.augmented_channel_rank, None);
        assert!(native.rank <= 3);
        assert_eq!(fused.len(), 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hint_fusion_keeps_a_strong_native_top_match_ahead_of_hint_only_matches() {
        let task_matches = vec![
            matched("native-task", 1, &["description_tokens:2"]),
            matched("weak-overlap", 2, &["description_tokens:1"]),
        ];
        let augmented_matches = vec![
            matched("hint-only-overlap", 1, &["description_tokens:3"]),
            matched("weak-overlap", 2, &["all_text_tokens:5"]),
            matched("native-task", 3, &["all_text_tokens:5"]),
        ];
        let task_query = RetrievalQuery::from_parts(["原始任务"]);

        let fused = fuse_retrieval_channels(task_matches, augmented_matches, &task_query, 3);
        let rank = |name: &str| {
            fused
                .iter()
                .find(|matched| matched.name == name)
                .map(|matched| matched.rank)
                .expect("expected bounded capability")
        };

        assert_eq!(rank("native-task"), 1);
        assert!(rank("hint-only-overlap") < rank("weak-overlap"));
    }

    #[test]
    fn hint_fusion_does_not_promote_weak_task_evidence_using_hint_reasons() {
        let task_matches = vec![matched("weak-task", 1, &["all_text_tokens:1"])];
        let augmented_matches = vec![
            matched("direct-hint", 1, &["declared_trigger"]),
            matched("weak-task", 2, &["description_phrase"]),
        ];
        let task_query = RetrievalQuery::from_parts(["incidental"]);

        let fused = fuse_retrieval_channels(task_matches, augmented_matches, &task_query, 2);

        assert_eq!(fused[0].name, "direct-hint");
        assert_eq!(fused[1].name, "weak-task");
        assert_eq!(fused[1].task_channel_rank, Some(1));
        assert_eq!(fused[1].augmented_channel_rank, Some(2));
    }

    #[test]
    fn hint_fusion_treats_correlated_cjk_description_and_body_evidence_as_strong() {
        let task_matches = vec![matched(
            "native-task",
            1,
            &["cjk_description_bigrams:1", "cjk_all_text_bigrams:3"],
        )];
        let augmented_matches = vec![
            matched("hint-only", 1, &["description_tokens:3"]),
            matched("other", 2, &["description_tokens:2"]),
            matched("native-task", 3, &["all_text_tokens:3"]),
        ];
        let task_query = RetrievalQuery::from_parts(["把中文改得自然克制一些"]);

        let fused = fuse_retrieval_channels(task_matches, augmented_matches, &task_query, 3);

        assert_eq!(fused[0].name, "native-task");
        assert_eq!(fused[0].task_channel_rank, Some(1));
        assert_eq!(fused[0].augmented_channel_rank, Some(3));
        assert!(matches!(
            fused[0].ranking_adjustments.as_slice(),
            [RankingAdjustment::ProtectedOriginalTaskMatch]
        ));
        assert_eq!(
            serde_json::to_value(&fused[0]).unwrap()["ranking_adjustments"],
            serde_json::json!(["protected_original_task_match"])
        );
        assert!(fused[1].ranking_adjustments.is_empty());
        assert!(fused[2].ranking_adjustments.is_empty());
    }

    #[test]
    fn token_matching_normalizes_common_ascii_plurals() {
        assert_eq!(
            tokens("presentations slides spreadsheets skills agents"),
            BTreeSet::from([
                "agent".to_owned(),
                "presentation".to_owned(),
                "skill".to_owned(),
                "slide".to_owned(),
                "spreadsheet".to_owned(),
            ])
        );
        assert_eq!(
            tokens("inspect and govern Skills for Agents with evidence"),
            BTreeSet::from([
                "agent".to_owned(),
                "evidence".to_owned(),
                "govern".to_owned(),
                "inspect".to_owned(),
                "skill".to_owned(),
            ])
        );
        assert_eq!(candidate_search_text("publish blogs"), "publish blogs blog");
    }

    #[test]
    fn token_matching_segments_cjk_runs_into_overlapping_bigrams() {
        let segmented = tokens("把中文改自然一点 AI");

        for expected in ["把中", "中文", "文改", "改自", "自然", "然一", "ai"] {
            assert!(segmented.contains(expected), "missing token {expected:?}");
        }
        assert!(!segmented.contains("一点"));
        assert!(!segmented.contains("把中文改自然一点"));
    }

    #[test]
    fn single_token_body_only_matches_have_a_bounded_low_confidence_tail() {
        let (_, mut scan) = fixture();
        let representative = scan.skills[0].clone();
        scan.skills = (0..8)
            .map(|index| {
                let mut skill = representative.clone();
                skill.id = format!("incidental-{index}");
                skill.name = format!("incidental-{index}");
                skill.metadata.description = Some("generic helper".into());
                skill.normalized_text = "mentions archive incidentally".into();
                skill
            })
            .collect();
        scan.usage.push(crate::scan::UsageEvidence {
            agent: AgentKind::Codex,
            skill_id: "incidental-0".into(),
            stage: UsageStage::Loaded,
            quality: EvidenceQuality::Observed,
            event_count: 1,
            first_seen_unix: Some(1),
            last_seen_unix: Some(1),
            month_start_unix: None,
            source_path_digest: "fixture".into(),
        });

        let matches = find(&scan, "archive", 100);

        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].score, 5.0);
        assert!(
            matches[0]
                .match_reasons
                .contains(&"observed_local_usage".into())
        );
    }

    #[test]
    fn description_boundaries_keep_the_desired_clause_and_separate_the_exclusion() {
        let (desired, excluded) = description_routing_sections(
            "Use for standalone spreadsheet files, not for a live Excel session.",
        );

        assert_eq!(desired, "use for standalone spreadsheet files");
        assert_eq!(excluded, "not for a live excel session");
    }

    #[test]
    fn task_routing_sections_separate_independent_cjk_and_english_constraints() {
        let (positive, excluded) = task_routing_sections(
            "检查工作树问题，不要修改代码; do not run tests，也不要创建 issue",
        );

        assert_eq!(positive, "检查工作树问题");
        assert_eq!(
            excluded,
            vec!["不要修改代码", "do not run tests", "也不要创建 issue"]
        );
    }

    #[test]
    fn task_routing_sections_require_a_complete_do_not_marker() {
        for task in [
            "do nothing unusual; inspect worktree problems",
            "diagnose why tests do not pass",
        ] {
            let (positive, excluded) = task_routing_sections(task);

            assert_eq!(positive, task);
            assert!(excluded.is_empty());
        }
    }

    #[test]
    fn task_exclusions_filter_conflicting_hints_and_bound_effect_evidence() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skillroster-task-exclusions-{nonce}"));
        let inspect = root.join("inspect-worktree");
        fs::create_dir_all(&inspect).unwrap();
        fs::write(
            inspect.join("SKILL.md"),
            "---\nname: inspect-worktree\ndescription: Inspect worktree problems.\n---\n",
        )
        .unwrap();
        for index in 0..12 {
            let editor = root.join(format!("editor-{index:02}"));
            fs::create_dir_all(&editor).unwrap();
            fs::write(
                editor.join("SKILL.md"),
                format!("---\nname: editor-{index:02}\ndescription: Modify.\n---\n"),
            )
            .unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let scan = scan(&options).unwrap();
        let query = RetrievalQuery::from_task_and_hints(
            "Inspect worktree problems; do not modify",
            ["editor-00"],
        );

        let result = find_matching_with_evidence(&scan, &query, 3, None, None);

        assert_eq!(result.matches[0].name, "inspect-worktree");
        assert_eq!(result.task_exclusion_effects.affected_candidate_count, 12);
        assert_eq!(result.task_exclusion_effects.items.len(), 10);
        assert!(result.task_exclusion_effects.items_truncated);
        assert_eq!(result.task_exclusion_effects.items[0].name, "editor-00");
        assert_eq!(result.task_exclusion_effects.items[9].name, "editor-09");
        assert!(
            result
                .task_exclusion_effects
                .items
                .iter()
                .all(|effect| effect.description_token_count == 1)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn task_exclusion_controls_preserve_exact_cjk_and_english_scores() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skillroster-task-controls-{nonce}"));
        for (directory, contents) in [
            (
                "inspect-worktree",
                "---\nname: inspect-worktree\ndescription: 检查工作树问题。\n---\n",
            ),
            (
                "modify-code",
                "---\nname: modify-code\ndescription: 修改当前工作树中的代码并运行测试。\n---\n",
            ),
            (
                "inspect-en",
                "---\nname: inspect-en\ndescription: Inspect worktree problems.\n---\n",
            ),
            (
                "modify-en",
                "---\nname: modify-en\ndescription: Modify current worktree code and run tests.\n---\n",
            ),
        ] {
            let skill = root.join(directory);
            fs::create_dir_all(&skill).unwrap();
            fs::write(skill.join("SKILL.md"), contents).unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let scan = scan(&options).unwrap();

        for (baseline_task, constrained_task, expected_name, expected_score) in [
            (
                "检查工作树问题",
                "检查工作树问题，不要修改当前工作树中的代码并运行测试",
                "inspect-worktree",
                115.0,
            ),
            (
                "Inspect worktree problems",
                "Inspect worktree problems; do not modify current worktree code or run tests",
                "inspect-en",
                94.0,
            ),
        ] {
            let baseline_query =
                RetrievalQuery::from_task_and_hints(baseline_task, std::iter::empty::<&str>());
            let constrained_query =
                RetrievalQuery::from_task_and_hints(constrained_task, std::iter::empty::<&str>());
            let baseline = find_matching(&scan, &baseline_query, 2, None, None);
            let constrained = find_matching(&scan, &constrained_query, 2, None, None);

            assert_eq!(baseline[0].name, expected_name);
            assert_eq!(baseline[0].score, expected_score);
            assert_eq!(constrained[0].name, baseline[0].name);
            assert_eq!(constrained[0].score, baseline[0].score);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exclusion_evidence_ignores_shared_stopwords() {
        let query = tokens("govern a Skill roster into Core and On-demand states");
        let excluded = tokens(
            "not for installing Skills or migrating, distributing, synchronizing, or repairing shared Skill directories and symlinks",
        );

        assert_eq!(
            query
                .intersection(&excluded)
                .cloned()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["skill".to_owned()])
        );
    }

    #[test]
    fn find_returns_one_ranked_capability_for_same_name_variants() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skillroster-routing-variants-{nonce}"));
        for (directory, description, body) in [
            (
                "essay-a",
                "Write a technical essay",
                "Draft technical essays with diagrams.",
            ),
            (
                "essay-b",
                "Manage financial spreadsheets",
                "Build deterministic workbook formulas.",
            ),
        ] {
            let path = root.join(directory);
            fs::create_dir_all(&path).unwrap();
            fs::write(
                path.join("SKILL.md"),
                format!("---\nname: tech-essay-writer\ndescription: {description}\n---\n{body}"),
            )
            .unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: crate::harness::AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let mut scan = scan(&options).unwrap();
        let diagram_skill_id = scan
            .skills
            .iter()
            .find(|skill| skill.normalized_text.contains("diagrams"))
            .unwrap()
            .id
            .clone();
        for skill in &mut scan.skills {
            if skill.id != diagram_skill_id {
                skill.name = "  TECH-ESSAY-WRITER  ".into();
            }
        }

        let matches = find(&scan, "diagrams", 10);

        assert_eq!(
            matches
                .iter()
                .filter(|matched| matched.name == "tech-essay-writer")
                .count(),
            1
        );
        let matched = matches
            .iter()
            .find(|matched| matched.name == "tech-essay-writer")
            .unwrap();
        assert_eq!(matched.variant_count, 2);
        assert_eq!(matched.variant_skill_ids.len(), 2);
        assert_eq!(matched.variants.len(), 2);
        assert!(
            matched
                .variants
                .iter()
                .all(|variant| variant.paths.len() == 1)
        );
        assert!(matched.match_reasons.contains(&"name_variants:2".into()));

        let eligible = BTreeSet::from([matched.skill_id.clone()]);
        let query = RetrievalQuery::from_parts(["diagrams"]);
        let filtered = find_matching(&scan, &query, 10, Some(&eligible), Some(&eligible));
        let filtered_match = filtered
            .iter()
            .find(|candidate| candidate.name == "tech-essay-writer")
            .unwrap();
        assert_eq!(filtered_match.variant_count, 1);
        assert!(!filtered_match.variants_truncated);
        assert!(filtered_match.variant_skill_ids.is_empty());
        assert!(filtered_match.variants.is_empty());

        let all_routable = scan
            .skills
            .iter()
            .map(|skill| skill.id.clone())
            .collect::<BTreeSet<_>>();
        let partial_match =
            find_matching(&scan, &query, 10, Some(&eligible), Some(&all_routable)).remove(0);
        assert_eq!(partial_match.variant_count, 2);
        assert_eq!(partial_match.variants.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_name_variant_details_are_bounded_and_keep_the_representative() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("skillroster-bounded-variants-{nonce}"));
        for index in 0..12 {
            let path = root.join(format!("variant-{index:02}"));
            fs::create_dir_all(&path).unwrap();
            let body = if index == 11 {
                "diagram specialist"
            } else {
                "unrelated capability"
            };
            fs::write(
                path.join("SKILL.md"),
                format!(
                    "---\nname: shared-capability\ndescription: variant {index}\n---\n{body}\n"
                ),
            )
            .unwrap();
        }
        let mut options = ScanOptions::for_home(root.join("home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: crate::harness::AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let scan = scan(&options).unwrap();

        let matched = find(&scan, "diagram specialist", 1).remove(0);

        assert_eq!(matched.variant_count, 12);
        assert!(matched.variants_truncated);
        assert_eq!(matched.variant_skill_ids.len(), 10);
        assert_eq!(matched.variants.len(), 10);
        assert!(
            matched
                .variant_skill_ids
                .iter()
                .any(|skill_id| skill_id == &matched.skill_id)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn report_first_view_prioritizes_systemic_and_category_diverse_findings() {
        fn finding(
            category: FindingCategory,
            severity: Severity,
            title: &str,
            skill_count: usize,
            placement_count: usize,
        ) -> Finding {
            Finding {
                id: format!("finding_{title}"),
                kind: FindingKind::ManagementStateReview,
                category,
                severity,
                title: title.into(),
                summary: title.into(),
                affected_skill_ids: (0..skill_count)
                    .map(|index| format!("skill_{title}_{index}"))
                    .collect(),
                affected_placement_ids: (0..placement_count)
                    .map(|index| format!("placement_{title}_{index}"))
                    .collect(),
                evidence: Vec::new(),
                evidence_quality: EvidenceQuality::Observed,
                coverage_basis: FindingCoverageBasis::SkillRootScan,
            }
        }

        let mut findings = vec![
            finding(
                FindingCategory::Overlap,
                Severity::Medium,
                "duplicate-a",
                1,
                6,
            ),
            finding(
                FindingCategory::Overlap,
                Severity::Medium,
                "duplicate-b",
                1,
                5,
            ),
            finding(
                FindingCategory::Layout,
                Severity::High,
                "unsafe-links",
                16,
                16,
            ),
            finding(
                FindingCategory::Exposure,
                Severity::Medium,
                "large-roster",
                128,
                128,
            ),
            finding(
                FindingCategory::Layout,
                Severity::Medium,
                "name-conflicts",
                8,
                8,
            ),
            finding(FindingCategory::Usage, Severity::Info, "coverage", 0, 0),
        ];

        prioritize_report_findings(&mut findings, 3);

        assert_eq!(
            findings
                .iter()
                .take(3)
                .map(|finding| finding.title.as_str())
                .collect::<Vec<_>>(),
            vec!["unsafe-links", "large-roster", "duplicate-a"]
        );
    }

    #[test]
    fn exposure_is_one_systemic_finding_with_an_agent_breakdown() {
        let mut scan = ScanResult::default();
        for (agent, count) in [
            (crate::harness::AgentKind::Codex, 52_usize),
            (crate::harness::AgentKind::ClaudeCode, 51_usize),
        ] {
            scan.placements
                .extend((0..count).map(|index| crate::scan::SkillPlacement {
                    id: format!("placement_{}_{index}", agent.id()),
                    skill_id: format!("skill_{}_{index}", agent.id()),
                    agent: Some(agent),
                    root: PathBuf::from(format!("/{}/skills", agent.id())),
                    directory: PathBuf::from(format!("/{}/skills/{index}", agent.id())),
                    entrypoint: PathBuf::from(format!("/{}/skills/{index}/SKILL.md", agent.id())),
                    physical_directory: None,
                    content_digest: format!("digest_{index}"),
                    entrypoint_digest: None,
                    fingerprint_completeness: crate::scan::FingerprintCompleteness::Complete,
                    fingerprint_detail: None,
                    link_target: None,
                    link_status: crate::scan::LinkStatus::NotLink,
                    default_exposed: true,
                    owned_by_agent: Some(true),
                    mutation_scope: Some(crate::scan::MutationScope::Mutable),
                    governable: true,
                    provider: None,
                    executable_files: Vec::new(),
                    declared_name_matches_directory: Some(true),
                }));
        }
        let mut findings = Vec::new();

        exposure_findings(&scan, &mut findings);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].affected_placement_ids.len(), 103);
        assert!(findings[0].summary.contains("Codex=52"));
        assert!(findings[0].summary.contains("Claude Code=51"));
    }

    #[test]
    fn maintained_routing_set_has_complete_top_three_recall() {
        #[derive(serde::Deserialize)]
        struct Case {
            task: String,
            skill: String,
            #[serde(default)]
            hints: Vec<String>,
        }

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/routing-skills");
        let mut options = ScanOptions::for_home(PathBuf::from("/nonexistent-fixture-home"));
        options
            .explicit_skill_roots
            .push(crate::scan::ExplicitSkillRoot {
                agent: crate::harness::AgentKind::Codex,
                path: root.clone(),
            });
        options.include_session_evidence = false;
        let scan = scan(&options).unwrap();
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../tests/fixtures/routing-eval.json")).unwrap();
        let hits = cases
            .iter()
            .filter(|case| {
                let retrieval_query = std::iter::once(case.task.as_str())
                    .chain(case.hints.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" ");
                find(&scan, &retrieval_query, 3)
                    .iter()
                    .any(|item| item.name == case.skill)
            })
            .count();
        assert_eq!(hits, cases.len(), "Top-3 recall was {hits}/{}", cases.len());
    }
}
