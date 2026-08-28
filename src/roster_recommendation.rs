use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

use crate::change::RosterChange;
use crate::harness::AgentKind;
#[cfg(test)]
use crate::model::FindingCategory;
use crate::model::FindingRecord;
use crate::scan::{EvidenceQuality, ScanResult, UsageStage};

pub const MAX_CORE_BUDGET: usize = 50;

#[derive(Debug)]
pub struct SharedPhysicalCoreBudgetExceeded {
    pub agent: AgentKind,
    pub core_count: usize,
    pub core_budget: usize,
}

impl std::fmt::Display for SharedPhysicalCoreBudgetExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "shared physical Core propagation gives Agent {} {} Core Skills; core_budget {} is too small",
            self.agent.id(),
            self.core_count,
            self.core_budget
        )
    }
}

impl std::error::Error for SharedPhysicalCoreBudgetExceeded {}

#[derive(Debug)]
pub struct PhysicalMutationIdentityRescanRequired {
    pub placement_id: String,
}

impl std::fmt::Display for PhysicalMutationIdentityRescanRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Snapshot has no observed physical mutation identity for Placement {}; run skillroster scan",
            self.placement_id
        )
    }
}

impl std::error::Error for PhysicalMutationIdentityRescanRequired {}

pub fn is_large_roster_finding(finding: &FindingRecord) -> bool {
    crate::query::stored_finding_is(finding, crate::query::FindingKind::LargeDefaultRoster)
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

    fn has_positive_signal(&self) -> bool {
        self.direct_signal.is_present() || self.cross_agent_signal.is_present()
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

    let mut core_by_agent = BTreeMap::new();
    for (agent, candidates) in &by_agent {
        let forced_count = candidates
            .values()
            .filter(|candidate| candidate.is_forced_core())
            .count();
        if forced_count > request.core_budget {
            bail!(
                "Agent {} has {forced_count} protected, declared-Core, or bootstrap Skills; core_budget {} is too small",
                agent.id(),
                request.core_budget
            );
        }
        let mut ranked = candidates.values().collect::<Vec<_>>();
        ranked.sort_by(|left, right| candidate_order(left, right));
        core_by_agent.insert(
            *agent,
            ranked
                .iter()
                .take(request.core_budget)
                .map(|candidate| candidate.skill_id.clone())
                .collect::<BTreeSet<_>>(),
        );
    }
    let shared_core_agents =
        reconcile_shared_physical_states(scan, &by_agent, &mut core_by_agent, request.core_budget)?;

    let mut changes = Vec::new();
    let mut agents = Vec::new();
    for (agent, candidates) in by_agent {
        let before_default_exposure = scan
            .placements
            .iter()
            .filter(|placement| placement.agent == Some(agent) && placement.default_exposed)
            .count();
        let mut candidates = candidates.into_values().collect::<Vec<_>>();
        candidates.sort_by(candidate_order);
        let core_ids = &core_by_agent[&agent];
        let core_selections = candidates
            .iter()
            .filter(|candidate| core_ids.contains(&candidate.skill_id))
            .map(|candidate| CoreSelection {
                skill_id: candidate.skill_id.clone(),
                name: candidate.name.clone(),
                reason: if shared_core_agents.contains_key(&(agent, candidate.skill_id.clone())) {
                    "shared_physical_forced_core"
                } else {
                    candidate.reason()
                },
                evidence_scope: if shared_core_agents
                    .contains_key(&(agent, candidate.skill_id.clone()))
                {
                    "forced"
                } else {
                    candidate.evidence_scope()
                },
                evidence_agents: shared_core_agents
                    .get(&(agent, candidate.skill_id.clone()))
                    .cloned()
                    .unwrap_or_else(|| candidate.evidence_agents()),
            })
            .collect::<Vec<_>>();
        let positive_signal_count = candidates
            .iter()
            .filter(|candidate| candidate.has_positive_signal())
            .count();
        let direct_signal_count = candidates
            .iter()
            .filter(|candidate| candidate.direct_signal.is_present())
            .count();
        let cross_agent_signal_count = candidates
            .iter()
            .filter(|candidate| {
                !candidate.direct_signal.is_present() && candidate.cross_agent_signal.is_present()
            })
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

fn reconcile_shared_physical_states(
    scan: &ScanResult,
    candidates: &BTreeMap<AgentKind, BTreeMap<String, Candidate>>,
    core_by_agent: &mut BTreeMap<AgentKind, BTreeSet<String>>,
    core_budget: usize,
) -> Result<BTreeMap<(AgentKind, String), Vec<String>>> {
    let mut physical_groups = BTreeMap::<(String, PathBuf), BTreeSet<AgentKind>>::new();
    for placement in scan
        .placements
        .iter()
        .filter(|placement| placement.default_exposed)
    {
        let Some(agent) = placement.agent else {
            continue;
        };
        let physical_identity = match scan.observed_physical_mutation_path(placement) {
            Some(path) => path.to_path_buf(),
            None => {
                return Err(PhysicalMutationIdentityRescanRequired {
                    placement_id: placement.id.clone(),
                }
                .into());
            }
        };
        physical_groups
            .entry((placement.skill_id.clone(), physical_identity))
            .or_default()
            .insert(agent);
    }

    let mut groups_by_skill = BTreeMap::<String, Vec<BTreeSet<AgentKind>>>::new();
    for ((skill_id, _), agents) in physical_groups
        .into_iter()
        .filter(|(_, agents)| agents.len() > 1)
    {
        let components = groups_by_skill.entry(skill_id).or_default();
        let mut merged = agents;
        let mut index = 0;
        while index < components.len() {
            if components[index].is_disjoint(&merged) {
                index += 1;
            } else {
                merged.extend(components.remove(index));
                index = 0;
            }
        }
        components.push(merged);
    }

    let mut shared_core_agents = BTreeMap::new();
    for (skill_id, components) in groups_by_skill {
        for agents in components {
            let states = agents
                .iter()
                .map(|agent| {
                    core_by_agent
                        .get(agent)
                        .is_none_or(|core| core.contains(&skill_id))
                })
                .collect::<BTreeSet<_>>();
            if states.len() == 1 {
                continue;
            }
            let forced_agents = agents
                .iter()
                .filter(|agent| {
                    candidates
                        .get(agent)
                        .and_then(|items| items.get(&skill_id))
                        .is_some_and(Candidate::is_forced_core)
                })
                .copied()
                .collect::<BTreeSet<_>>();
            let choose_core = !forced_agents.is_empty()
                || agents.iter().any(|agent| !candidates.contains_key(agent));
            let mut evidence_agents = agents
                .iter()
                .map(|agent| agent.id().to_owned())
                .collect::<Vec<_>>();
            evidence_agents.sort();
            for agent in agents.iter().filter(|agent| candidates.contains_key(agent)) {
                let core = core_by_agent
                    .get_mut(agent)
                    .expect("candidate Agent has a Core selection");
                if choose_core {
                    core.insert(skill_id.clone());
                    shared_core_agents.insert((*agent, skill_id.clone()), evidence_agents.clone());
                } else {
                    core.remove(&skill_id);
                }
            }
        }
    }
    for (agent, core) in core_by_agent {
        if core.len() > core_budget {
            return Err(SharedPhysicalCoreBudgetExceeded {
                agent: *agent,
                core_count: core.len(),
                core_budget,
            }
            .into());
        }
    }

    Ok(shared_core_agents)
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
    use std::fs;
    use std::path::PathBuf;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::model::{EvidenceId, FindingId, ReportId, Severity};
    use crate::scan::{
        LinkStatus, ScanOptions, ScannedSkill, SkillMetadata, SkillPlacement, UsageEvidence, scan,
    };

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
            month_start_unix: None,
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
    fn planning_signal_counts_include_candidates_beyond_the_core_budget() {
        let mut scan = oversized_scan(51);
        for index in 0..51 {
            scan.usage.push(usage(
                AgentKind::Codex,
                &format!("skill_{index:03}"),
                UsageStage::Loaded,
                EvidenceQuality::Observed,
            ));
        }

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

        assert_eq!(recommendation.agents[0].core_selections.len(), 1);
        assert_eq!(recommendation.agents[0].positive_signal_count, 51);
        assert_eq!(recommendation.agents[0].direct_signal_count, 51);
        assert_eq!(recommendation.agents[0].cross_agent_signal_count, 0);
        assert_eq!(recommendation.agents[0].fallback_core_count, 0);
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
            content_identity_digest: None,
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

    #[test]
    fn shared_physical_rank_divergence_reconciles_to_on_demand() {
        let mut scan = shared_oversized_scan(51);
        scan.usage.push(usage(
            AgentKind::Codex,
            "skill_050",
            UsageStage::Loaded,
            EvidenceQuality::Observed,
        ));
        scan.usage.push(usage(
            AgentKind::ClaudeCode,
            "skill_049",
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

        assert!(
            recommendation
                .changes
                .iter()
                .all(|change| change.state == "on_demand")
        );
        assert!(recommendation.agents.iter().all(|agent| {
            agent.core_count == 0 && agent.on_demand_count == 51 && agent.core_selections.is_empty()
        }));
    }

    #[test]
    fn shared_physical_forced_core_propagates_and_recomputes_summaries() {
        let scan = shared_oversized_scan(51);
        let recommendation = recommend(
            &finding(&scan),
            &scan,
            &BTreeSet::from([(AgentKind::Codex, "skill_050".into())]),
            &RecommendationRequest {
                core_budget: 1,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap();

        for agent in &recommendation.agents {
            assert_eq!(agent.core_count, 1);
            assert_eq!(agent.on_demand_count, 50);
            assert_eq!(agent.core_selections.len(), 1);
            assert_eq!(agent.core_selections[0].skill_id, "skill_050");
            assert_eq!(
                agent.core_selections[0].reason,
                "shared_physical_forced_core"
            );
            assert_eq!(agent.core_selections[0].evidence_scope, "forced");
            assert_eq!(
                agent.core_selections[0].evidence_agents,
                ["claude-code", "codex"]
            );
        }
        assert!(recommendation.changes.iter().all(|change| {
            (change.skill_id == "skill_050" && change.state == "core")
                || (change.skill_id != "skill_050" && change.state == "on_demand")
        }));
    }

    #[test]
    fn shared_physical_forced_core_propagation_respects_each_agent_budget() {
        let scan = shared_oversized_scan(51);
        let error = recommend(
            &finding(&scan),
            &scan,
            &BTreeSet::from([
                (AgentKind::Codex, "skill_050".into()),
                (AgentKind::ClaudeCode, "skill_049".into()),
            ]),
            &RecommendationRequest {
                core_budget: 1,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap_err();

        assert!(
            error
                .downcast_ref::<SharedPhysicalCoreBudgetExceeded>()
                .is_some()
        );
        assert!(
            error
                .to_string()
                .contains("shared physical Core propagation gives Agent")
        );
        assert!(
            error
                .to_string()
                .contains("2 Core Skills; core_budget 1 is too small")
        );
    }

    #[test]
    fn shared_physical_out_of_scope_exposure_is_retained_or_fails_budget() {
        let mut scan = oversized_scan(51);
        let shared = scan.placements.last_mut().unwrap();
        shared.physical_directory = Some(PathBuf::from("/tmp/home/.shared/skill_050"));
        let mut claude = shared.clone();
        claude.id = "claude_shared".into();
        claude.agent = Some(AgentKind::ClaudeCode);
        claude.root = PathBuf::from("/tmp/home/.claude/skills");
        claude.directory = claude.root.join("skill_050");
        claude.entrypoint = claude.directory.join("SKILL.md");
        scan.placements.push(claude);
        scan.freeze_observed_physical_mutation_paths();
        let mut roster_finding = finding(&scan);
        roster_finding.details["affected_placement_ids"] = json!(
            scan.placements
                .iter()
                .filter(|placement| placement.agent == Some(AgentKind::Codex))
                .map(|placement| placement.id.as_str())
                .collect::<Vec<_>>()
        );

        let error = recommend(
            &roster_finding,
            &scan,
            &BTreeSet::new(),
            &RecommendationRequest {
                core_budget: 50,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap_err();

        let exceeded = error
            .downcast_ref::<SharedPhysicalCoreBudgetExceeded>()
            .unwrap();
        assert_eq!(exceeded.agent, AgentKind::Codex);
        assert_eq!(exceeded.core_count, 51);
        assert_eq!(exceeded.core_budget, 50);
    }

    #[test]
    fn shared_physical_forced_core_does_not_spill_into_an_independent_component() {
        let scan = two_component_shared_scan(51);
        let recommendation = recommend(
            &finding(&scan),
            &scan,
            &BTreeSet::from([(AgentKind::Codex, "skill_050".into())]),
            &RecommendationRequest {
                core_budget: 1,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap();

        let selected = recommendation
            .agents
            .iter()
            .map(|agent| {
                (
                    agent.agent,
                    (
                        agent.core_selections[0].skill_id.as_str(),
                        agent.core_selections[0].reason,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            selected[&AgentKind::Codex],
            ("skill_050", "shared_physical_forced_core")
        );
        assert_eq!(
            selected[&AgentKind::ClaudeCode],
            ("skill_050", "shared_physical_forced_core")
        );
        assert_eq!(
            selected[&AgentKind::Hermes],
            ("skill_000", "stable_fallback")
        );
        assert_eq!(
            selected[&AgentKind::Cursor],
            ("skill_000", "stable_fallback")
        );
    }

    #[test]
    fn shared_physical_demotion_does_not_spill_into_an_independent_component() {
        let mut scan = two_component_shared_scan(51);
        scan.usage.push(usage(
            AgentKind::Codex,
            "skill_050",
            UsageStage::Loaded,
            EvidenceQuality::Observed,
        ));
        scan.usage.push(usage(
            AgentKind::ClaudeCode,
            "skill_049",
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

        for agent in [AgentKind::Codex, AgentKind::ClaudeCode] {
            let summary = recommendation
                .agents
                .iter()
                .find(|summary| summary.agent == agent)
                .unwrap();
            assert_eq!(summary.core_count, 0);
            assert!(summary.core_selections.is_empty());
        }
        let independent = [AgentKind::Hermes, AgentKind::Cursor].map(|agent| {
            recommendation
                .agents
                .iter()
                .find(|summary| summary.agent == agent)
                .unwrap()
        });
        assert!(independent.iter().all(|summary| summary.core_count == 1));
        assert_eq!(
            independent[0].core_selections[0].skill_id,
            independent[1].core_selections[0].skill_id
        );
    }

    #[cfg(unix)]
    #[test]
    fn aliased_roots_with_a_shared_symlink_entry_produce_a_ready_plan() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let shared_root = home.join(".shared-skills");
        let linked_store = shared_root.join(".store/skill-050");
        fs::create_dir_all(&linked_store).unwrap();
        fs::write(
            linked_store.join("SKILL.md"),
            "---\nname: skill-050\n---\nshared linked fixture\n",
        )
        .unwrap();
        for index in 0..50 {
            let skill = shared_root.join(format!("skill-{index:03}"));
            fs::create_dir_all(&skill).unwrap();
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: skill-{index:03}\n---\nfixture\n"),
            )
            .unwrap();
        }
        symlink(".store/skill-050", shared_root.join("skill-050")).unwrap();
        for root in [home.join(".codex/skills"), home.join(".claude/skills")] {
            fs::create_dir_all(root.parent().unwrap()).unwrap();
            symlink(&shared_root, root).unwrap();
        }

        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let mut snapshot = scan(&options).unwrap();
        let ids = snapshot
            .skills
            .iter()
            .map(|skill| (skill.name.as_str(), skill.id.clone()))
            .collect::<BTreeMap<_, _>>();
        let skill_049 = ids["skill-049"].clone();
        let skill_050 = ids["skill-050"].clone();
        snapshot.usage.push(usage(
            AgentKind::Codex,
            &skill_050,
            UsageStage::Loaded,
            EvidenceQuality::Observed,
        ));
        snapshot.usage.push(usage(
            AgentKind::ClaudeCode,
            &skill_049,
            UsageStage::Loaded,
            EvidenceQuality::Observed,
        ));

        let recommendation = recommend(
            &finding(&snapshot),
            &snapshot,
            &BTreeSet::new(),
            &RecommendationRequest {
                core_budget: 1,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap();
        for skill_id in [&skill_049, &skill_050] {
            assert_eq!(
                recommendation
                    .changes
                    .iter()
                    .filter(|change| &change.skill_id == skill_id)
                    .map(|change| change.state.as_str())
                    .collect::<BTreeSet<_>>()
                    .len(),
                1
            );
        }

        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let plan = crate::roster_plan::derive(&snapshot, &state, &recommendation.changes).unwrap();
        assert!(!plan.operations.is_empty());
        assert!(shared_root.join("skill-050").is_symlink());

        let replacement_root = home.join(".replacement-skills");
        fs::create_dir(&replacement_root).unwrap();
        let claude_root = home.join(".claude/skills");
        fs::remove_file(&claude_root).unwrap();
        symlink(&replacement_root, &claude_root).unwrap();
        let after_retarget = recommend(
            &finding(&snapshot),
            &snapshot,
            &BTreeSet::new(),
            &RecommendationRequest {
                core_budget: 1,
                protected_skill_ids: BTreeSet::new(),
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&after_retarget.changes).unwrap(),
            serde_json::to_value(&recommendation.changes).unwrap()
        );
        assert!(
            crate::roster_plan::derive(&snapshot, &state, &after_retarget.changes).is_err(),
            "Plan must independently reject current filesystem drift"
        );
    }

    fn oversized_scan(count: usize) -> ScanResult {
        let skills = (0..count)
            .map(|index| ScannedSkill {
                id: format!("skill_{index:03}"),
                name: format!("skill-{index:03}"),
                metadata: SkillMetadata::default(),
                content_digest: format!("sha256:{index:03}"),
                content_identity_digest: None,
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
                entrypoint_digest: None,
                fingerprint_completeness: crate::scan::FingerprintCompleteness::Complete,
                fingerprint_detail: None,
                link_target: None,
                link_status: LinkStatus::NotLink,
                default_exposed: true,
                owned_by_agent: Some(true),
                mutation_scope: Some(crate::scan::MutationScope::Mutable),
                governable: true,
                provider: None,
                executable_files: Vec::new(),
                declared_name_matches_directory: Some(true),
            })
            .collect();
        let mut scan = ScanResult {
            skills,
            placements,
            ..ScanResult::default()
        };
        scan.freeze_observed_physical_mutation_paths();
        scan
    }

    fn shared_oversized_scan(count: usize) -> ScanResult {
        let mut scan = oversized_scan(count);
        for placement in &mut scan.placements {
            placement.physical_directory = Some(PathBuf::from(format!(
                "/tmp/home/.shared/{}",
                placement.skill_id
            )));
        }
        let claude = scan
            .placements
            .iter()
            .cloned()
            .map(|mut placement| {
                placement.id = format!("claude_{}", placement.id);
                placement.agent = Some(AgentKind::ClaudeCode);
                placement.root = PathBuf::from("/tmp/home/.claude/skills");
                placement.directory = placement.root.join(&placement.skill_id);
                placement.entrypoint = placement.directory.join("SKILL.md");
                placement
            })
            .collect::<Vec<_>>();
        scan.placements.extend(claude);
        scan.freeze_observed_physical_mutation_paths();
        scan
    }

    fn two_component_shared_scan(count: usize) -> ScanResult {
        let mut scan = shared_oversized_scan(count);
        let source = scan
            .placements
            .iter()
            .filter(|placement| placement.agent == Some(AgentKind::Codex))
            .cloned()
            .collect::<Vec<_>>();
        for agent in [AgentKind::Hermes, AgentKind::Cursor] {
            scan.placements
                .extend(source.iter().cloned().map(|mut placement| {
                    placement.id = format!("{}_{}", agent.id(), placement.id);
                    placement.agent = Some(agent);
                    placement.root = PathBuf::from(format!("/tmp/home/.{}/skills", agent.id()));
                    placement.directory = placement.root.join(&placement.skill_id);
                    placement.entrypoint = placement.directory.join("SKILL.md");
                    placement.physical_directory = Some(PathBuf::from(format!(
                        "/tmp/home/.other-shared/{}",
                        placement.skill_id
                    )));
                    placement
                }));
        }
        scan.freeze_observed_physical_mutation_paths();
        scan
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
            month_start_unix: None,
            source_path_digest: "sha256:usage".into(),
        }
    }

    fn finding(scan: &ScanResult) -> FindingRecord {
        let mut counts = BTreeMap::new();
        for agent in scan
            .placements
            .iter()
            .filter(|placement| placement.default_exposed)
            .filter_map(|placement| placement.agent)
        {
            *counts.entry(agent).or_insert(0_usize) += 1;
        }
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
                    .filter(|placement| placement.default_exposed)
                    .filter(|placement| {
                        placement
                            .agent
                            .and_then(|agent| counts.get(&agent))
                            .is_some_and(|count| *count > MAX_CORE_BUDGET)
                    })
                    .map(|placement| placement.id.as_str())
                    .collect::<Vec<_>>()
            }),
            evidence_ids: vec![EvidenceId::parse("evidence_large-roster").unwrap()],
        }
    }
}
