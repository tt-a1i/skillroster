use crate::harness::AgentKind;
use crate::scan::{
    EvidenceQuality, LinkStatus, ScanResult, ScannedSkill, UsageStage, agents_with_usage,
    placements_by_skill, skill_search_text,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub category: FindingCategory,
    pub severity: Severity,
    pub title: String,
    pub summary: String,
    pub affected_skill_ids: Vec<String>,
    pub affected_placement_ids: Vec<String>,
    /// Stable path/digest/root references that let callers drill into the fact.
    pub evidence: Vec<String>,
    pub evidence_quality: EvidenceQuality,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PrimaryMetrics {
    pub independent_skills: usize,
    pub placements: usize,
    pub default_exposure: usize,
    pub agents_with_observed_usage: usize,
    pub agents_with_reliable_session_denominator: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub metrics: PrimaryMetrics,
    pub findings: Vec<Finding>,
    pub category_counts: BTreeMap<String, usize>,
    pub files_changed: bool,
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
    pub match_reasons: Vec<String>,
    pub evidence_quality: EvidenceQuality,
}

pub fn build_report(scan: &ScanResult) -> Report {
    let metrics = PrimaryMetrics {
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
            .filter(|coverage| coverage.denominator_reliable)
            .count(),
    };
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
            .then_with(|| left.title.cmp(&right.title))
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
        .filter(|root| matches!(root.status, crate::scan::RootStatus::Inaccessible))
        .collect::<Vec<_>>();
    if !unavailable.is_empty() {
        push_finding(
            findings,
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
                .map(|root| format!("path:{}", root.path.display()))
                .collect(),
            EvidenceQuality::Observed,
        );
    }
}

fn layout_findings(scan: &ScanResult, findings: &mut Vec<Finding>) {
    for (status, title, severity) in [
        (LinkStatus::Broken, "Broken Skill links", Severity::High),
        (
            LinkStatus::EscapesRoot,
            "Skill links escape an approved root",
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
            .entry(skill.name.to_lowercase())
            .or_default()
            .push(skill);
    }
    for (name, skills) in by_name {
        let digests = skills
            .iter()
            .map(|skill| skill.content_digest.as_str())
            .collect::<BTreeSet<_>>();
        if skills.len() > 1 && digests.len() > 1 {
            push_finding(
                findings,
                FindingCategory::Layout,
                Severity::Medium,
                "Same-name Skills have different content",
                format!(
                    "{name} resolves to {} distinct content digests.",
                    digests.len()
                ),
                skills.iter().map(|skill| skill.id.clone()).collect(),
                Vec::new(),
                skills
                    .iter()
                    .map(|skill| format!("digest:{}", skill.content_digest))
                    .collect(),
                EvidenceQuality::Observed,
            );
        }
    }

    for (skill_id, placements) in placements_by_skill(scan) {
        let digests = placements
            .iter()
            .map(|placement| placement.content_digest.as_str())
            .collect::<BTreeSet<_>>();
        if digests.len() > 1 {
            let skill_name = scan
                .skills
                .iter()
                .find(|skill| skill.id == skill_id)
                .map(|skill| skill.name.as_str())
                .unwrap_or("unknown Skill");
            push_finding(
                findings,
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

fn usage_findings(scan: &ScanResult, findings: &mut Vec<Finding>) {
    let exposed = scan
        .placements
        .iter()
        .filter(|placement| placement.default_exposed)
        .count() as u64;
    let mut stage_summaries = Vec::new();
    for stage in [
        UsageStage::Exposed,
        UsageStage::Matched,
        UsageStage::Loaded,
        UsageStage::Applied,
        UsageStage::Outcome,
    ] {
        let observations = scan
            .usage
            .iter()
            .filter(|usage| usage.stage == stage)
            .collect::<Vec<_>>();
        let count = if stage == UsageStage::Exposed {
            exposed
        } else {
            observations.iter().map(|usage| usage.event_count).sum()
        };
        let first = observations
            .iter()
            .filter_map(|usage| usage.first_seen_unix)
            .min()
            .map_or_else(|| "unknown".into(), |value| value.to_string());
        let last = observations
            .iter()
            .filter_map(|usage| usage.last_seen_unix)
            .max()
            .map_or_else(|| "unknown".into(), |value| value.to_string());
        let quality = if stage == UsageStage::Exposed
            || observations
                .iter()
                .any(|usage| usage.quality == EvidenceQuality::Observed)
        {
            "observed"
        } else if observations
            .iter()
            .any(|usage| usage.quality == EvidenceQuality::Inferred)
        {
            "inferred"
        } else {
            "unknown"
        };
        stage_summaries.push(format!(
            "{}={count} [{first}..{last}; {quality}]",
            usage_stage_name(stage)
        ));
    }
    let evidence = scan
        .usage
        .iter()
        .map(|usage| {
            format!(
                "usage:{}:{}:{:?}:{}",
                usage.agent.id(),
                usage.skill_id,
                usage.stage,
                usage.source_path_digest
            )
        })
        .chain(
            scan.coverage
                .iter()
                .map(|coverage| format!("coverage:{}", coverage.agent.id())),
        )
        .collect();
    let reliable_count = scan
        .coverage
        .iter()
        .filter(|coverage| coverage.denominator_reliable)
        .count();
    let coverage_files = scan
        .coverage
        .iter()
        .map(|coverage| coverage.files_observed)
        .sum::<usize>();
    let coverage_skipped = scan
        .coverage
        .iter()
        .map(|coverage| coverage.files_skipped)
        .sum::<usize>();
    let coverage_bytes = scan
        .coverage
        .iter()
        .map(|coverage| coverage.bytes_observed)
        .sum::<u64>();
    let coverage_lines = scan
        .coverage
        .iter()
        .map(|coverage| coverage.lines_observed)
        .sum::<usize>();
    let coverage_truncated = scan.coverage.iter().any(|coverage| coverage.truncated);
    push_finding(
        findings,
        FindingCategory::Usage,
        Severity::Info,
        "Five-stage usage evidence",
        format!(
            "{}. Coverage: reliable {reliable_count}/{} Agents, files={coverage_files}, skipped={coverage_skipped}, bytes={coverage_bytes}, lines={coverage_lines}, truncated={coverage_truncated}.",
            stage_summaries.join("; "),
            AgentKind::ALL.len(),
        ),
        scan.usage
            .iter()
            .map(|usage| usage.skill_id.clone())
            .collect(),
        Vec::new(),
        evidence,
        if reliable_count == AgentKind::ALL.len() {
            EvidenceQuality::Observed
        } else {
            EvidenceQuality::Unknown
        },
    );

    let unreliable = scan
        .coverage
        .iter()
        .filter(|coverage| !coverage.denominator_reliable)
        .collect::<Vec<_>>();
    let reliable_agents = scan
        .coverage
        .iter()
        .filter(|coverage| coverage.denominator_reliable)
        .map(|coverage| coverage.agent)
        .collect::<BTreeSet<_>>();
    let missing_count = AgentKind::ALL.len().saturating_sub(reliable_agents.len());
    if missing_count != 0 {
        push_finding(
            findings,
            FindingCategory::Usage,
            Severity::Info,
            "Usage coverage is incomplete",
            format!(
                "A reliable session denominator is unavailable for {}/{} supported Agents; absence of evidence is not evidence of non-use.",
                missing_count,
                AgentKind::ALL.len()
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

fn overlap_findings(scan: &ScanResult, findings: &mut Vec<Finding>) {
    for (skill_id, placements) in placements_by_skill(scan) {
        let Some(skill) = scan.skills.iter().find(|skill| skill.id == skill_id) else {
            continue;
        };
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
            push_finding(
                findings,
                FindingCategory::Overlap,
                Severity::Medium,
                "Exact duplicate Skill placements",
                format!(
                    "{} has {} placements with the same normalized content digest.",
                    skill.name,
                    exact_placements.len()
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
    let mut candidates = Vec::new();
    for (index, left) in scan.skills.iter().enumerate() {
        for right in scan.skills.iter().skip(index + 1) {
            if left.content_digest == right.content_digest {
                continue;
            }
            let left_tokens = tokens(&skill_search_text(left));
            let right_tokens = tokens(&skill_search_text(right));
            let intersection = left_tokens.intersection(&right_tokens).count();
            let union = left_tokens.union(&right_tokens).count();
            if intersection < 3 || union == 0 {
                continue;
            }
            let similarity = intersection as f64 / union as f64;
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
            FindingCategory::Overlap,
            Severity::Low,
            "Semantic overlap candidate",
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
        .filter(|coverage| coverage.denominator_reliable)
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
        push_finding(
            findings,
            FindingCategory::Lifecycle,
            Severity::Low,
            "Stale archive candidates require review",
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
        push_finding(
            findings,
            FindingCategory::Lifecycle,
            Severity::Info,
            "Archive candidacy is unknown",
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
    category: FindingCategory,
    severity: Severity,
    title: impl Into<String>,
    summary: impl Into<String>,
    mut affected_skill_ids: Vec<String>,
    mut affected_placement_ids: Vec<String>,
    mut evidence: Vec<String>,
    evidence_quality: EvidenceQuality,
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
        title,
        affected_skill_ids.join(","),
        affected_placement_ids.join(",")
    );
    findings.push(Finding {
        id: format!("finding_{}", fnv1a64(id_basis.as_bytes())),
        category,
        severity,
        title,
        summary: summary.into(),
        affected_skill_ids,
        affected_placement_ids,
        evidence,
        evidence_quality,
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
    find_matching(scan, task, limit, None)
}

pub(crate) fn find_matching(
    scan: &ScanResult,
    task: &str,
    limit: usize,
    candidate_ids: Option<&BTreeSet<String>>,
) -> Vec<FindMatch> {
    let query = task.trim().to_lowercase();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let query_tokens = tokens(&query);
    let placement_groups = placements_by_skill(scan);
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
            let triggers = skill.metadata.triggers.join(" ").to_lowercase();
            let all_text = skill_search_text(skill).to_lowercase();
            let name_overlap = query_tokens.intersection(&tokens(&name)).count();
            let trigger_overlap = query_tokens.intersection(&tokens(&triggers)).count();
            let description_overlap = query_tokens.intersection(&tokens(&description)).count();
            let overlap = query_tokens.intersection(&tokens(&all_text)).count();
            let mut score = name_overlap as f64 * 24.0
                + trigger_overlap as f64 * 18.0
                + description_overlap as f64 * 12.0
                + overlap as f64 * 3.0;
            let mut reasons = Vec::new();
            if name == query {
                score += 100.0;
                reasons.push("exact_name".into());
            } else if name.contains(&query) {
                score += 45.0;
                reasons.push("name_phrase".into());
            }
            if !triggers.is_empty() && triggers.contains(&query) {
                score += 35.0;
                reasons.push("declared_trigger".into());
            }
            if !description.is_empty() && description.contains(&query) {
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
            if overlap > 0 {
                reasons.push(format!("all_text_tokens:{overlap}"));
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
            Some(FindMatch {
                rank: 0,
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                score,
                paths,
                agents,
                roster_state: "unknown".into(),
                source: skill.metadata.source.clone(),
                match_reasons: reasons,
                evidence_quality: if observed_usage {
                    EvidenceQuality::Observed
                } else {
                    EvidenceQuality::Inferred
                },
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
    matches.truncate(limit);
    for (index, matched) in matches.iter_mut().enumerate() {
        matched.rank = index + 1;
    }
    matches
}

fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 2)
        .map(str::to_lowercase)
        .collect()
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
    use crate::scan::{ScanOptions, scan};
    use std::fs;
    use std::path::PathBuf;
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
        assert!(report.findings.iter().any(|finding| {
            finding.title == "Archive candidacy is unknown"
                && finding.evidence_quality == EvidenceQuality::Unknown
        }));
        assert!(!report.files_changed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn semantic_overlap_is_candidate_evidence_and_never_a_confirmed_duplicate() {
        let (root, mut scan) = fixture();
        let mut candidate = scan.skills[0].clone();
        candidate.id = "skill_semantic_candidate".into();
        candidate.name = "source-check".into();
        candidate.content_digest = "different_digest".into();
        candidate.metadata.description =
            Some("Search and verify primary sources using official evidence".into());
        candidate.metadata.triggers = vec!["research".into(), "verify".into()];
        candidate.summary = "Investigate facts with primary sources.".into();
        candidate.normalized_text =
            "Search and verify primary sources using official evidence Investigate facts".into();
        scan.skills.push(candidate);

        let report = build_report(&scan);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.title == "Semantic overlap candidate")
            .unwrap();
        assert_eq!(finding.evidence_quality, EvidenceQuality::Inferred);
        assert!(finding.summary.contains("review-only candidate evidence"));
        assert!(finding.summary.contains("not a confirmed duplicate"));
        fs::remove_dir_all(root).unwrap();
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
            files_observed: 2,
            files_skipped: 0,
            denominator_reliable: true,
            bytes_observed: 100,
            lines_observed: 10,
            truncated: false,
            first_seen_unix: Some(10),
            last_seen_unix: Some(20),
        });

        let report = build_report(&scan);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.title == "Stale archive candidates require review")
            .unwrap();
        assert_eq!(finding.evidence_quality, EvidenceQuality::Inferred);
        assert!(finding.summary.contains("candidate evidence only"));
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
                    content_digest: format!("digest_{index}"),
                    link_target: None,
                    link_status: crate::scan::LinkStatus::NotLink,
                    default_exposed: true,
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
    fn maintained_routing_set_has_at_least_ninety_five_percent_top_three_recall() {
        #[derive(serde::Deserialize)]
        struct Case {
            task: String,
            skill: String,
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
                find(&scan, &case.task, 3)
                    .iter()
                    .any(|item| item.name == case.skill)
            })
            .count();
        assert!(
            hits * 100 >= cases.len() * 95,
            "Top-3 recall was {hits}/{}",
            cases.len()
        );
    }
}
