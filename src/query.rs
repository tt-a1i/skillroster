use crate::harness::AgentKind;
use crate::scan::{
    EvidenceQuality, LinkStatus, ScanResult, ScannedSkill, UsageStage, agents_with_usage,
    placements_by_skill, skill_search_text,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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
    pub agents_with_session_roots: usize,
    pub agents_with_sampled_session_data: usize,
    pub agents_with_limited_session_data: usize,
    pub agents_missing_session_roots: usize,
    pub agents_with_inaccessible_session_roots: usize,
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
    /// Provider identities for externally managed plugin placements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    /// True when at least one placement may participate in a governance Plan.
    pub governable: bool,
    pub match_reasons: Vec<String>,
    pub evidence_quality: EvidenceQuality,
    /// Same declared name with distinct Skill identities is one ambiguous capability result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variant_skill_ids: Vec<String>,
    /// Same-name identities with provider and path facts kept correctly associated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<FindVariant>,
    pub variant_count: usize,
    pub variants_truncated: bool,
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
            .filter(|coverage| coverage.roots_present > 0 && !coverage.denominator_reliable)
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
    let roots_present_count = scan
        .coverage
        .iter()
        .filter(|coverage| coverage.roots_present > 0)
        .count();
    let sampled_count = scan
        .coverage
        .iter()
        .filter(|coverage| coverage.files_observed > 0)
        .count();
    let missing_root_count = scan
        .coverage
        .iter()
        .filter(|coverage| {
            coverage.roots_present == 0
                && coverage.roots_missing > 0
                && coverage.roots_inaccessible == 0
        })
        .count();
    let inaccessible_root_count = scan
        .coverage
        .iter()
        .filter(|coverage| coverage.roots_inaccessible > 0)
        .count();
    let limited_root_count = scan
        .coverage
        .iter()
        .filter(|coverage| coverage.roots_present > 0 && !coverage.denominator_reliable)
        .count();
    let coverage_discovered = scan
        .coverage
        .iter()
        .map(|coverage| {
            coverage.files_discovered.max(
                coverage
                    .files_observed
                    .saturating_add(coverage.files_skipped),
            )
        })
        .sum::<usize>();
    let coverage_observed = scan
        .coverage
        .iter()
        .map(|coverage| coverage.files_observed)
        .sum::<usize>();
    let coverage_partial = scan
        .coverage
        .iter()
        .map(|coverage| coverage.files_partially_observed)
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
    let discovery_truncated = scan
        .coverage
        .iter()
        .any(|coverage| coverage.discovery_truncated);
    push_finding(
        findings,
        FindingCategory::Usage,
        Severity::Info,
        "Five-stage usage evidence",
        format!(
            "{}. Coverage: roots {roots_present_count}/{supported}, sampled {sampled_count}/{supported}, complete {reliable_count}/{supported}, missing {missing_root_count}/{supported}, inaccessible {inaccessible_root_count}/{supported}; files discovered={coverage_discovered}, observed={coverage_observed}, partial={coverage_partial}, skipped={coverage_skipped}; bytes={coverage_bytes}, lines={coverage_lines}, truncated={coverage_truncated}, discovery_truncated={discovery_truncated}.",
            stage_summaries.join("; "),
            supported = AgentKind::ALL.len(),
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
    let incomplete_count = AgentKind::ALL.len().saturating_sub(reliable_count);
    if incomplete_count != 0 {
        push_finding(
            findings,
            FindingCategory::Usage,
            Severity::Info,
            "Usage coverage is incomplete",
            format!(
                "A complete observable-session denominator is unavailable for {incomplete_count}/{} supported Agents: {missing_root_count} session roots are missing, {inaccessible_root_count} are inaccessible, and {limited_root_count} present roots have bounded or incomplete samples. Recent observed events remain usable; absence of evidence is not evidence of non-use.",
                AgentKind::ALL.len(),
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
            let physical_sources = exact_placements
                .iter()
                .map(|placement| physical_source_identity(placement, &scan.placements))
                .collect::<BTreeSet<_>>();
            if physical_sources.len() < 2 {
                continue;
            }
            push_finding(
                findings,
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
    let mut candidates = Vec::new();
    for (index, left) in scan.skills.iter().enumerate() {
        let left_tokens = &vocabularies[index];
        for (right_index, right) in scan.skills.iter().enumerate().skip(index + 1) {
            if left.content_digest == right.content_digest {
                continue;
            }
            let right_tokens = &vocabularies[right_index];
            let intersection = left_tokens.intersection(right_tokens).count();
            let union = left_tokens.union(right_tokens).count();
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

fn physical_source_identity(
    start: &crate::scan::SkillPlacement,
    placements: &[crate::scan::SkillPlacement],
) -> PathBuf {
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
    find_matching(scan, task, limit, None, None)
}

pub(crate) fn find_matching(
    scan: &ScanResult,
    task: &str,
    limit: usize,
    candidate_ids: Option<&BTreeSet<String>>,
    variant_eligible_ids: Option<&BTreeSet<String>>,
) -> Vec<FindMatch> {
    let query = task.trim().to_lowercase();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let query_tokens = tokens(&query);
    let placement_groups = placements_by_skill(scan);
    let mut variants_by_name = BTreeMap::<String, Vec<String>>::new();
    for skill in scan
        .skills
        .iter()
        .filter(|skill| variant_eligible_ids.is_none_or(|ids| ids.contains(&skill.id)))
    {
        variants_by_name
            .entry(skill.name.trim().to_lowercase())
            .or_default()
            .push(skill.id.clone());
    }
    for variants in variants_by_name.values_mut() {
        variants.sort();
        variants.dedup();
    }
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
            let name_overlap = query_tokens.intersection(&tokens(&name)).count();
            let trigger_overlap = query_tokens.intersection(&tokens(&triggers)).count();
            let description_overlap = query_tokens
                .intersection(&tokens(&positive_description))
                .count();
            let excluded_description_overlap = query_tokens
                .intersection(&tokens(&excluded_description))
                .count();
            let exclusion_penalty_tokens = if excluded_description_overlap >= 2 {
                excluded_description_overlap
            } else {
                0
            };
            let overlap = query_tokens.intersection(&tokens(&all_text)).count();
            let mut score = name_overlap as f64 * 24.0
                + trigger_overlap as f64 * 18.0
                + description_overlap as f64 * 12.0
                + overlap as f64 * 3.0
                - exclusion_penalty_tokens as f64 * 18.0;
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
            if !positive_description.is_empty() && positive_description.contains(&query) {
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
            if exclusion_penalty_tokens > 0 {
                reasons.push(format!(
                    "excluded_description_tokens:{exclusion_penalty_tokens}"
                ));
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
            let all_variant_skill_ids = variants_by_name.get(&name.trim().to_lowercase());
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
            let governable = placements.iter().any(|placement| placement.governable);
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
                        Some(FindVariant {
                            skill_id: variant_id.clone(),
                            paths,
                            agents,
                            roster_state: "unknown".into(),
                            source: variant_skill.metadata.source.clone(),
                            providers,
                            governable: variant_placements
                                .iter()
                                .any(|placement| placement.governable),
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
                match_reasons: reasons,
                evidence_quality: if observed_usage {
                    EvidenceQuality::Observed
                } else {
                    EvidenceQuality::Inferred
                },
                variant_skill_ids,
                variants,
                variant_count,
                variants_truncated,
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
    capabilities
}

fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 2)
        .map(normalize_token)
        .collect()
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
    ];
    let description = description.to_lowercase();
    let mut positive = Vec::new();
    let mut excluded = Vec::new();
    for section in description.split(['.', '!', '?', ';', '\n']) {
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
    fn semantic_overlap_analysis_stays_bounded_for_a_realistic_inventory() {
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

        let started = std::time::Instant::now();
        let report = build_report(&scan);

        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "semantic overlap analysis took {:?} for 193 Skills",
            started.elapsed()
        );
        assert_eq!(
            report
                .findings
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
        assert_eq!(candidate_search_text("publish blogs"), "publish blogs blog");
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
        let scan = scan(&options).unwrap();

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
        let filtered = find_matching(&scan, "diagrams", 10, Some(&eligible), Some(&eligible));
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
            find_matching(&scan, "diagrams", 10, Some(&eligible), Some(&all_routable)).remove(0);
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
