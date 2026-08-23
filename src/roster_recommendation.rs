use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow, bail};

use crate::change::RosterChange;
use crate::harness::AgentKind;
use crate::model::{FindingCategory, FindingRecord};
use crate::scan::{EvidenceQuality, ScanResult, UsageStage};

pub const MAX_CORE_BUDGET: usize = 50;
pub const LARGE_ROSTER_FINDING_KIND: &str = "large_default_roster";
pub const LARGE_ROSTER_FINDING_TITLE: &str = "Large default Rosters need review";

pub fn finding_kind(category: &FindingCategory, title: &str) -> Option<&'static str> {
    (*category == FindingCategory::Exposure && title == LARGE_ROSTER_FINDING_TITLE)
        .then_some(LARGE_ROSTER_FINDING_KIND)
}

pub fn is_large_roster_finding(finding: &FindingRecord) -> bool {
    finding
        .details
        .get("kind")
        .and_then(serde_json::Value::as_str)
        == Some(LARGE_ROSTER_FINDING_KIND)
        || (finding.details.get("kind").is_none()
            && finding_kind(&finding.category, &finding.title).is_some())
}

#[derive(Clone, Debug)]
pub struct RecommendationRequest {
    pub core_budget: usize,
    pub protected_skill_ids: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct CoreSelection {
    pub skill_id: String,
    pub name: String,
    pub reason: &'static str,
    pub evidence_scope: &'static str,
    pub evidence_agents: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct AgentRecommendation {
    pub agent: AgentKind,
    pub before_default_exposure: usize,
    pub unique_skill_count: usize,
    pub core_count: usize,
    pub on_demand_count: usize,
    pub positive_signal_count: usize,
    pub direct_signal_count: usize,
    pub cross_agent_signal_count: usize,
    pub fallback_core_count: usize,
    pub core_selections: Vec<CoreSelection>,
}

#[derive(Clone, Debug)]
pub struct RosterRecommendation {
    pub changes: Vec<RosterChange>,
    pub agents: Vec<AgentRecommendation>,
}

#[derive(Clone, Debug)]
struct Candidate {
    agent: AgentKind,
    skill_id: String,
    name: String,
    protected: bool,
    declared_core: bool,
    bootstrap: bool,
    direct_signal: UsageSignal,
    cross_agent_signal: UsageSignal,
    cross_agent_sources: BTreeSet<AgentKind>,
}

#[derive(Clone, Copy, Debug, Default)]
struct UsageSignal {
    quality_rank: u8,
    stage_rank: u8,
    event_count: u64,
    last_seen: u64,
}

impl UsageSignal {
    fn observe(
        &mut self,
        quality: EvidenceQuality,
        stage: UsageStage,
        event_count: u64,
        last_seen: u64,
    ) {
        let signal = (quality_rank(quality), stage_rank(stage));
        if signal > (self.quality_rank, self.stage_rank) {
            self.quality_rank = signal.0;
            self.stage_rank = signal.1;
        }
        self.event_count = self.event_count.saturating_add(event_count);
        self.last_seen = self.last_seen.max(last_seen);
    }

    const fn is_present(self) -> bool {
        self.stage_rank > 0
    }
}

impl Candidate {
    fn is_forced_core(&self) -> bool {
        self.protected || self.declared_core || self.bootstrap
    }

    fn reason(&self) -> &'static str {
        if self.protected {
            "protected_by_request"
        } else if self.declared_core {
            "declared_core"
        } else if self.bootstrap {
            "skillroster_bootstrap"
        } else if self.direct_signal.is_present() {
            signal_reason(self.direct_signal, false)
        } else if self.cross_agent_signal.is_present() {
            signal_reason(self.cross_agent_signal, true)
        } else {
            "stable_fallback"
        }
    }

    fn evidence_scope(&self) -> &'static str {
        if self.is_forced_core() {
            "forced"
        } else if self.direct_signal.is_present() {
            "target_agent"
        } else if self.cross_agent_signal.is_present() {
            "cross_agent"
        } else {
            "fallback"
        }
    }

    fn evidence_agents(&self) -> Vec<String> {
        match self.evidence_scope() {
            "target_agent" => vec![self.agent.id().to_owned()],
            "cross_agent" => self
                .cross_agent_sources
                .iter()
                .map(|agent| agent.id().to_owned())
                .collect(),
            _ => Vec::new(),
        }
    }
}

fn signal_reason(signal: UsageSignal, cross_agent: bool) -> &'static str {
    match (cross_agent, signal.quality_rank, signal.stage_rank) {
        (false, 2, 4) => "observed_outcome",
        (false, 2, 3) => "observed_applied",
        (false, 2, 2) => "observed_loaded",
        (false, 2, 1) => "observed_matched",
        (false, 1, 4) => "inferred_outcome",
        (false, 1, 3) => "inferred_applied",
        (false, 1, 2) => "inferred_loaded",
        (false, 1, 1) => "inferred_matched",
        (false, _, 4) => "unknown_quality_outcome",
        (false, _, 3) => "unknown_quality_applied",
        (false, _, 2) => "unknown_quality_loaded",
        (false, _, 1) => "unknown_quality_matched",
        (true, 2, 4) => "cross_agent_observed_outcome",
        (true, 2, 3) => "cross_agent_observed_applied",
        (true, 2, 2) => "cross_agent_observed_loaded",
        (true, 2, 1) => "cross_agent_observed_matched",
        (true, 1, 4) => "cross_agent_inferred_outcome",
        (true, 1, 3) => "cross_agent_inferred_applied",
        (true, 1, 2) => "cross_agent_inferred_loaded",
        (true, 1, 1) => "cross_agent_inferred_matched",
        (true, _, 4) => "cross_agent_unknown_quality_outcome",
        (true, _, 3) => "cross_agent_unknown_quality_applied",
        (true, _, 2) => "cross_agent_unknown_quality_loaded",
        (true, _, 1) => "cross_agent_unknown_quality_matched",
        (_, _, _) => "stable_fallback",
    }
}

pub fn recommend(
    finding: &FindingRecord,
    scan: &ScanResult,
    declared_core: &BTreeSet<(AgentKind, String)>,
    request: &RecommendationRequest,
) -> Result<RosterRecommendation> {
    if !(1..=MAX_CORE_BUDGET).contains(&request.core_budget) {
        bail!("core_budget must be between 1 and {MAX_CORE_BUDGET}");
    }
    let scope = exposure_finding_scope(finding, scan)?;
    let scoped_skills = scope
        .values()
        .flatten()
        .map(|candidate| candidate.skill_id.as_str())
        .collect::<BTreeSet<_>>();
    for skill_id in &request.protected_skill_ids {
        if !scoped_skills.contains(skill_id.as_str()) {
            bail!(
                "protected Skill {skill_id} is outside Finding {}",
                finding.id
            );
        }
    }

    let skill_names = scan
        .skills
        .iter()
        .map(|skill| (skill.id.as_str(), skill.name.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut by_agent = BTreeMap::<AgentKind, BTreeMap<String, Candidate>>::new();
    for (agent, placements) in scope {
        for placement in placements {
            let name = skill_names
                .get(placement.skill_id.as_str())
                .ok_or_else(|| anyhow!("Skill {} is missing from Snapshot", placement.skill_id))?;
            by_agent
                .entry(agent)
                .or_default()
                .entry(placement.skill_id.clone())
                .or_insert_with(|| Candidate {
                    agent,
                    skill_id: placement.skill_id.clone(),
                    name: (*name).to_owned(),
                    protected: request.protected_skill_ids.contains(&placement.skill_id),
                    declared_core: declared_core.contains(&(agent, placement.skill_id.clone())),
                    bootstrap: name.eq_ignore_ascii_case("skillroster"),
                    direct_signal: UsageSignal::default(),
                    cross_agent_signal: UsageSignal::default(),
                    cross_agent_sources: BTreeSet::new(),
                });
        }
    }
    for usage in &scan.usage {
        if usage.stage == UsageStage::Exposed {
            continue;
        }
        for (target_agent, skills) in &mut by_agent {
            let Some(candidate) = skills.get_mut(&usage.skill_id) else {
                continue;
            };
            let signal = if *target_agent == usage.agent {
                &mut candidate.direct_signal
            } else {
                candidate.cross_agent_sources.insert(usage.agent);
                &mut candidate.cross_agent_signal
            };
            signal.observe(
                usage.quality,
                usage.stage,
                usage.event_count,
                usage.last_seen_unix.unwrap_or_default(),
            );
        }
    }

    let mut changes = Vec::new();
    let mut agents = Vec::new();
    for (agent, candidates) in by_agent {
        let before_default_exposure = scan
            .placements
            .iter()
            .filter(|placement| placement.agent == Some(agent) && placement.default_exposed)
            .count();
        let mut candidates = candidates.into_values().collect::<Vec<_>>();
        let forced_count = candidates
            .iter()
            .filter(|candidate| candidate.is_forced_core())
            .count();
        if forced_count > request.core_budget {
            bail!(
                "Agent {} has {forced_count} protected, declared-Core, or bootstrap Skills; core_budget {} is too small",
                agent.id(),
                request.core_budget
            );
        }
        candidates.sort_by(candidate_order);
        let core_ids = candidates
            .iter()
            .take(request.core_budget)
            .map(|candidate| candidate.skill_id.as_str())
            .collect::<BTreeSet<_>>();
        let core_selections = candidates
            .iter()
            .take(request.core_budget)
            .map(|candidate| CoreSelection {
                skill_id: candidate.skill_id.clone(),
                name: candidate.name.clone(),
                reason: candidate.reason(),
                evidence_scope: candidate.evidence_scope(),
                evidence_agents: candidate.evidence_agents(),
            })
            .collect::<Vec<_>>();
        let positive_signal_count = core_selections
            .iter()
            .filter(|selection| matches!(selection.evidence_scope, "target_agent" | "cross_agent"))
            .count();
        let direct_signal_count = core_selections
            .iter()
            .filter(|selection| selection.evidence_scope == "target_agent")
            .count();
        let cross_agent_signal_count = core_selections
            .iter()
            .filter(|selection| selection.evidence_scope == "cross_agent")
            .count();
        let fallback_core_count = core_selections
            .iter()
            .filter(|selection| selection.reason == "stable_fallback")
            .count();
        for candidate in &candidates {
            changes.push(RosterChange {
                agent: candidate.agent.id().to_owned(),
                skill_id: candidate.skill_id.clone(),
                state: if core_ids.contains(candidate.skill_id.as_str()) {
                    "core"
                } else {
                    "on_demand"
                }
                .to_owned(),
            });
        }
        agents.push(AgentRecommendation {
            agent,
            before_default_exposure,
            unique_skill_count: candidates.len(),
            core_count: core_ids.len(),
            on_demand_count: candidates.len().saturating_sub(core_ids.len()),
            positive_signal_count,
            direct_signal_count,
            cross_agent_signal_count,
            fallback_core_count,
            core_selections,
        });
    }
    changes
        .sort_by(|left, right| (&left.agent, &left.skill_id).cmp(&(&right.agent, &right.skill_id)));
    Ok(RosterRecommendation { changes, agents })
}

fn exposure_finding_scope<'a>(
    finding: &FindingRecord,
    scan: &'a ScanResult,
) -> Result<BTreeMap<AgentKind, Vec<&'a crate::scan::SkillPlacement>>> {
    if !is_large_roster_finding(finding) {
        bail!("Finding {} is not a large-Roster Finding", finding.id);
    }
    let ids = finding
        .details
        .get("affected_placement_ids")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("Finding {} has no affected_placement_ids", finding.id))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("Finding {} has an invalid placement ID", finding.id))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let mut expected = BTreeMap::<AgentKind, Vec<&crate::scan::SkillPlacement>>::new();
    for placement in scan
        .placements
        .iter()
        .filter(|placement| placement.default_exposed)
    {
        if let Some(agent) = placement.agent {
            expected.entry(agent).or_default().push(placement);
        }
    }
    expected.retain(|_, placements| placements.len() > MAX_CORE_BUDGET);
    let expected_ids = expected
        .values()
        .flatten()
        .map(|placement| placement.id.as_str())
        .collect::<BTreeSet<_>>();
    let actual_ids = ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
    if actual_ids != expected_ids {
        bail!(
            "Finding {} no longer covers every oversized default Roster placement",
            finding.id
        );
    }
    Ok(expected)
}

fn candidate_order(left: &Candidate, right: &Candidate) -> Ordering {
    right
        .is_forced_core()
        .cmp(&left.is_forced_core())
        .then_with(|| right.protected.cmp(&left.protected))
        .then_with(|| right.declared_core.cmp(&left.declared_core))
        .then_with(|| right.bootstrap.cmp(&left.bootstrap))
        .then_with(|| {
            right
                .direct_signal
                .is_present()
                .cmp(&left.direct_signal.is_present())
        })
        .then_with(|| {
            right
                .direct_signal
                .quality_rank
                .cmp(&left.direct_signal.quality_rank)
        })
        .then_with(|| {
            right
                .direct_signal
                .stage_rank
                .cmp(&left.direct_signal.stage_rank)
        })
        .then_with(|| {
            right
                .direct_signal
                .event_count
                .cmp(&left.direct_signal.event_count)
        })
        .then_with(|| {
            right
                .direct_signal
                .last_seen
                .cmp(&left.direct_signal.last_seen)
        })
        .then_with(|| {
            right
                .cross_agent_signal
                .is_present()
                .cmp(&left.cross_agent_signal.is_present())
        })
        .then_with(|| {
            right
                .cross_agent_signal
                .quality_rank
                .cmp(&left.cross_agent_signal.quality_rank)
        })
        .then_with(|| {
            right
                .cross_agent_signal
                .stage_rank
                .cmp(&left.cross_agent_signal.stage_rank)
        })
        .then_with(|| {
            right
                .cross_agent_signal
                .event_count
                .cmp(&left.cross_agent_signal.event_count)
        })
        .then_with(|| {
            right
                .cross_agent_signal
                .last_seen
                .cmp(&left.cross_agent_signal.last_seen)
        })
        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        .then_with(|| left.skill_id.cmp(&right.skill_id))
}

const fn quality_rank(quality: EvidenceQuality) -> u8 {
    match quality {
        EvidenceQuality::Observed => 2,
        EvidenceQuality::Inferred => 1,
        EvidenceQuality::Unknown => 0,
    }
}

const fn stage_rank(stage: UsageStage) -> u8 {
    match stage {
        UsageStage::Exposed => 0,
        UsageStage::Matched => 1,
        UsageStage::Loaded => 2,
        UsageStage::Applied => 3,
        UsageStage::Outcome => 4,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::model::{EvidenceId, FindingId, ReportId, Severity};
    use crate::scan::{LinkStatus, ScannedSkill, SkillMetadata, SkillPlacement, UsageEvidence};

    #[test]
    fn recommendation_keeps_only_forced_and_evidence_ranked_core_skills() {
        let mut scan = oversized_scan(60);
        scan.skills[0].name = "skillroster".into();
        scan.usage.push(UsageEvidence {
            agent: AgentKind::Codex,
            skill_id: "skill_003".into(),
            stage: UsageStage::Outcome,
            quality: EvidenceQuality::Observed,
            event_count: 3,
            first_seen_unix: Some(10),
            last_seen_unix: Some(20),
            source_path_digest: "sha256:usage".into(),
        });
        let recommendation = recommend(
            &finding(&scan),
            &scan,
            &BTreeSet::from([(AgentKind::Codex, "skill_002".into())]),
            &RecommendationRequest {
                core_budget: 4,
                protected_skill_ids: BTreeSet::from(["skill_001".into()]),
            },
        )
        .unwrap();

        let core = recommendation
            .changes
            .iter()
            .filter(|change| change.state == "core")
            .map(|change| change.skill_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            core,
            BTreeSet::from(["skill_000", "skill_001", "skill_002", "skill_003"])
        );
        assert!(
            recommendation
                .changes
                .iter()
                .filter(|change| !core.contains(change.skill_id.as_str()))
                .all(|change| change.state == "on_demand")
        );
        assert_eq!(recommendation.agents[0].positive_signal_count, 1);
        assert_eq!(recommendation.agents[0].direct_signal_count, 1);
        assert_eq!(recommendation.agents[0].cross_agent_signal_count, 0);
        assert_eq!(recommendation.agents[0].fallback_core_count, 0);
    }

    #[test]
    fn exact_skill_usage_from_another_agent_outranks_fallback() {
        let mut scan = oversized_scan(51);
        scan.skills[0].name = "alpha".into();
        scan.skills[50].name = "zeta".into();
        scan.usage.push(usage(
            AgentKind::Cursor,
            "skill_050",
            UsageStage::Loaded,
            EvidenceQuality::Observed,
        ));

        let recommendation = recommend(
            &finding(&scan),
            &scan,
            &BTreeSet::new(),
            &RecommendationRequest {
                core_budget: 1,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap();

        let selection = &recommendation.agents[0].core_selections[0];
        assert_eq!(selection.skill_id, "skill_050");
        assert_eq!(selection.reason, "cross_agent_observed_loaded");
        assert_eq!(selection.evidence_scope, "cross_agent");
        assert_eq!(selection.evidence_agents, ["cursor"]);
        assert_eq!(recommendation.agents[0].positive_signal_count, 1);
        assert_eq!(recommendation.agents[0].direct_signal_count, 0);
        assert_eq!(recommendation.agents[0].cross_agent_signal_count, 1);
        assert_eq!(recommendation.agents[0].fallback_core_count, 0);
    }

    #[test]
    fn target_agent_signal_outranks_stronger_cross_agent_signal() {
        let mut scan = oversized_scan(51);
        scan.usage.push(usage(
            AgentKind::Codex,
            "skill_049",
            UsageStage::Matched,
            EvidenceQuality::Observed,
        ));
        scan.usage.push(usage(
            AgentKind::Cursor,
            "skill_050",
            UsageStage::Outcome,
            EvidenceQuality::Observed,
        ));

        let recommendation = recommend(
            &finding(&scan),
            &scan,
            &BTreeSet::new(),
            &RecommendationRequest {
                core_budget: 1,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap();

        let selection = &recommendation.agents[0].core_selections[0];
        assert_eq!(selection.skill_id, "skill_049");
        assert_eq!(selection.reason, "observed_matched");
        assert_eq!(selection.evidence_scope, "target_agent");
        assert_eq!(selection.evidence_agents, ["codex"]);
    }

    #[test]
    fn same_name_with_a_different_skill_id_does_not_transfer_usage() {
        let mut scan = oversized_scan(51);
        scan.skills[0].name = "zeta".into();
        scan.skills[1].name = "alpha".into();
        scan.skills.push(ScannedSkill {
            id: "foreign_skill".into(),
            name: "zeta".into(),
            metadata: SkillMetadata::default(),
            content_digest: "sha256:foreign".into(),
            digest_algorithm: "sha256".into(),
            summary: String::new(),
            normalized_text: String::new(),
            modified_at_unix: None,
        });
        scan.usage.push(usage(
            AgentKind::Cursor,
            "foreign_skill",
            UsageStage::Outcome,
            EvidenceQuality::Observed,
        ));

        let recommendation = recommend(
            &finding(&scan),
            &scan,
            &BTreeSet::new(),
            &RecommendationRequest {
                core_budget: 1,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap();

        let selection = &recommendation.agents[0].core_selections[0];
        assert_eq!(selection.skill_id, "skill_001");
        assert_eq!(selection.reason, "stable_fallback");
        assert_eq!(selection.evidence_scope, "fallback");
        assert!(selection.evidence_agents.is_empty());
    }

    #[test]
    fn missing_usage_is_stable_fallback_not_archive_evidence() {
        let mut scan = oversized_scan(51);
        scan.skills[0].name = "zeta".into();
        scan.skills[1].name = "alpha".into();
        let recommendation = recommend(
            &finding(&scan),
            &scan,
            &BTreeSet::new(),
            &RecommendationRequest {
                core_budget: 1,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap();

        assert_eq!(recommendation.agents[0].core_selections[0].name, "alpha");
        assert_eq!(
            recommendation.agents[0].core_selections[0].reason,
            "stable_fallback"
        );
        assert!(
            recommendation
                .changes
                .iter()
                .all(|change| matches!(change.state.as_str(), "core" | "on_demand"))
        );
    }

    #[test]
    fn recommendation_rejects_a_finding_with_incomplete_scope() {
        let scan = oversized_scan(51);
        let mut finding = finding(&scan);
        finding.details["affected_placement_ids"]
            .as_array_mut()
            .unwrap()
            .pop();

        let error = recommend(
            &finding,
            &scan,
            &BTreeSet::new(),
            &RecommendationRequest {
                core_budget: 50,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no longer covers every oversized")
        );
    }

    fn oversized_scan(count: usize) -> ScanResult {
        let skills = (0..count)
            .map(|index| ScannedSkill {
                id: format!("skill_{index:03}"),
                name: format!("skill-{index:03}"),
                metadata: SkillMetadata::default(),
                content_digest: format!("sha256:{index:03}"),
                digest_algorithm: "sha256".into(),
                summary: String::new(),
                normalized_text: String::new(),
                modified_at_unix: None,
            })
            .collect::<Vec<_>>();
        let placements = skills
            .iter()
            .enumerate()
            .map(|(index, skill)| SkillPlacement {
                id: format!("placement_{index:03}"),
                skill_id: skill.id.clone(),
                agent: Some(AgentKind::Codex),
                root: PathBuf::from("/tmp/home/.codex/skills"),
                directory: PathBuf::from(format!("/tmp/home/.codex/skills/skill-{index:03}")),
                entrypoint: PathBuf::from(format!(
                    "/tmp/home/.codex/skills/skill-{index:03}/SKILL.md"
                )),
                physical_directory: None,
                content_digest: skill.content_digest.clone(),
                link_target: None,
                link_status: LinkStatus::NotLink,
                default_exposed: true,
                governable: true,
                provider: None,
                executable_files: Vec::new(),
                declared_name_matches_directory: Some(true),
            })
            .collect();
        ScanResult {
            skills,
            placements,
            ..ScanResult::default()
        }
    }

    fn usage(
        agent: AgentKind,
        skill_id: &str,
        stage: UsageStage,
        quality: EvidenceQuality,
    ) -> UsageEvidence {
        UsageEvidence {
            agent,
            skill_id: skill_id.into(),
            stage,
            quality,
            event_count: 1,
            first_seen_unix: Some(10),
            last_seen_unix: Some(20),
            source_path_digest: "sha256:usage".into(),
        }
    }

    fn finding(scan: &ScanResult) -> FindingRecord {
        FindingRecord {
            id: FindingId::parse("finding_large-roster").unwrap(),
            report_id: ReportId::parse("report_large-roster").unwrap(),
            category: FindingCategory::Exposure,
            severity: Severity::Warning,
            title: "Large default Rosters need review".into(),
            summary: String::new(),
            details: json!({
                "affected_placement_ids": scan
                    .placements
                    .iter()
                    .map(|placement| placement.id.as_str())
                    .collect::<Vec<_>>()
            }),
            evidence_ids: vec![EvidenceId::parse("evidence_large-roster").unwrap()],
        }
    }
}
