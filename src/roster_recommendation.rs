use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow, bail};

use crate::change::RosterChange;
use crate::harness::AgentKind;
use crate::model::{FindingCategory, FindingRecord};
use crate::scan::{EvidenceQuality, ScanResult, UsageStage};

pub const MAX_CORE_BUDGET: usize = 50;

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
}

#[derive(Clone, Debug)]
pub struct AgentRecommendation {
    pub agent: AgentKind,
    pub before_default_exposure: usize,
    pub unique_skill_count: usize,
    pub core_count: usize,
    pub on_demand_count: usize,
    pub positive_signal_count: usize,
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
    quality_rank: u8,
    stage_rank: u8,
    event_count: u64,
    last_seen: u64,
}

impl Candidate {
    fn is_forced_core(&self) -> bool {
        self.protected || self.declared_core || self.bootstrap
    }

    fn has_positive_signal(&self) -> bool {
        self.stage_rank > 0
    }

    fn reason(&self) -> &'static str {
        if self.protected {
            "protected_by_request"
        } else if self.declared_core {
            "declared_core"
        } else if self.bootstrap {
            "skillroster_bootstrap"
        } else {
            match (self.quality_rank, self.stage_rank) {
                (2, 4) => "observed_outcome",
                (2, 3) => "observed_applied",
                (2, 2) => "observed_loaded",
                (2, 1) => "observed_matched",
                (1, 4) => "inferred_outcome",
                (1, 3) => "inferred_applied",
                (1, 2) => "inferred_loaded",
                (1, 1) => "inferred_matched",
                (_, 4) => "unknown_quality_outcome",
                (_, 3) => "unknown_quality_applied",
                (_, 2) => "unknown_quality_loaded",
                (_, 1) => "unknown_quality_matched",
                _ => "stable_fallback",
            }
        }
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
                    quality_rank: 0,
                    stage_rank: 0,
                    event_count: 0,
                    last_seen: 0,
                });
        }
    }
    for usage in &scan.usage {
        if usage.stage == UsageStage::Exposed {
            continue;
        }
        let Some(candidate) = by_agent
            .get_mut(&usage.agent)
            .and_then(|skills| skills.get_mut(&usage.skill_id))
        else {
            continue;
        };
        let signal = (quality_rank(usage.quality), stage_rank(usage.stage));
        if signal > (candidate.quality_rank, candidate.stage_rank) {
            candidate.quality_rank = signal.0;
            candidate.stage_rank = signal.1;
        }
        candidate.event_count = candidate.event_count.saturating_add(usage.event_count);
        candidate.last_seen = candidate
            .last_seen
            .max(usage.last_seen_unix.unwrap_or_default());
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
            })
            .collect::<Vec<_>>();
        let positive_signal_count = candidates
            .iter()
            .filter(|candidate| candidate.has_positive_signal())
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
    if finding.category != FindingCategory::Exposure
        || finding.title != "Large default Rosters need review"
    {
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
        .then_with(|| right.quality_rank.cmp(&left.quality_rank))
        .then_with(|| right.stage_rank.cmp(&left.stage_rank))
        .then_with(|| right.event_count.cmp(&left.event_count))
        .then_with(|| right.last_seen.cmp(&left.last_seen))
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
        assert_eq!(recommendation.agents[0].fallback_core_count, 0);
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
                content_digest: skill.content_digest.clone(),
                link_target: None,
                link_status: LinkStatus::NotLink,
                default_exposed: true,
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
