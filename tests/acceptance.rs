use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde_json::Value;
use skillroster::harness::{AgentKind, known_agent_roots};
use skillroster::query::{FindingCategory, build_report, find};
use skillroster::scan::{EvidenceQuality, ScanOptions, UsageStage, scan};
use tempfile::TempDir;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

#[derive(Debug, Deserialize)]
struct RouteCase {
    task: String,
    skill: String,
}

#[derive(Debug, Deserialize)]
struct PresentationScenario {
    name: String,
    independent_skills: usize,
    copies_per_skill: usize,
    cross_agent: bool,
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn all_eight_agent_fixtures_discover_one_skill_and_all_five_usage_stages() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    copy_tree(&Path::new(FIXTURES).join("agents/home"), &home);

    let result = scan(&ScanOptions::for_home(home.clone())).unwrap();
    let expected_agents = AgentKind::ALL.into_iter().collect::<BTreeSet<_>>();
    let discovered_agents = result
        .placements
        .iter()
        .filter_map(|placement| placement.agent)
        .collect::<BTreeSet<_>>();
    assert_eq!(discovered_agents, expected_agents);
    assert_eq!(
        result.skills.len(),
        8,
        "every direct adapter has an independent fixture"
    );

    for roots in known_agent_roots(&home) {
        let placement = result
            .placements
            .iter()
            .find(|placement| placement.agent == Some(roots.agent))
            .unwrap_or_else(|| panic!("{} fixture was not discovered", roots.agent.id()));
        let stages = result
            .usage
            .iter()
            .filter(|usage| usage.agent == roots.agent && usage.skill_id == placement.skill_id)
            .map(|usage| (usage.stage, usage.quality))
            .collect::<Vec<_>>();
        for (stage, quality) in [
            (UsageStage::Exposed, EvidenceQuality::Inferred),
            (UsageStage::Matched, EvidenceQuality::Observed),
            (UsageStage::Loaded, EvidenceQuality::Observed),
            (UsageStage::Applied, EvidenceQuality::Observed),
            (UsageStage::Outcome, EvidenceQuality::Observed),
        ] {
            assert!(
                stages.contains(&(stage, quality)),
                "{} is missing {stage:?} with {quality:?}",
                roots.agent.id()
            );
        }
        assert_eq!(
            stages.len(),
            5,
            "prose-only records must add no observed stage"
        );
        let coverage = result
            .coverage
            .iter()
            .find(|coverage| coverage.agent == roots.agent)
            .unwrap();
        assert!(coverage.denominator_reliable);
        assert_eq!(coverage.files_observed, 1);
        assert_eq!(coverage.files_skipped, 0);
    }
}

#[test]
fn maintained_routing_set_meets_top_three_and_governance_does_not_regress_success() {
    let routes: Vec<RouteCase> =
        serde_json::from_slice(&fs::read(Path::new(FIXTURES).join("routing-eval.json")).unwrap())
            .unwrap();
    assert!(
        routes.len() >= 40,
        "the baseline must remain representative"
    );

    let mut options = ScanOptions::for_home(PathBuf::from("/nonexistent-fixture-home"));
    options
        .explicit_skill_roots
        .push(Path::new(FIXTURES).join("routing-skills"));
    options.include_session_evidence = false;
    let unmanaged = scan(&options).unwrap();

    let successes = |inventory: &skillroster::scan::ScanResult| {
        routes
            .iter()
            .filter(|case| {
                find(inventory, &case.task, 3)
                    .iter()
                    .any(|matched| matched.name == case.skill)
            })
            .count()
    };
    let unmanaged_successes = successes(&unmanaged);
    let recall = unmanaged_successes as f64 / routes.len() as f64;
    assert!(recall >= 0.95, "Top-3 recall was {:.1}%", recall * 100.0);

    // Roster governance changes exposure, not Library searchability. Model the
    // post-governance inventory by taking every non-core placement off default
    // exposure while retaining every Skill in the searchable Scan.
    let mut governed = unmanaged.clone();
    let core = BTreeSet::from(["research", "diagnose", "code-review"]);
    let names = governed
        .skills
        .iter()
        .map(|skill| (skill.id.clone(), skill.name.clone()))
        .collect::<BTreeMap<_, _>>();
    for placement in &mut governed.placements {
        placement.default_exposed = names
            .get(&placement.skill_id)
            .is_some_and(|name| core.contains(name.as_str()));
    }
    let governed_successes = successes(&governed);
    assert_eq!(
        governed_successes, unmanaged_successes,
        "moving Skills to On-demand must not regress task-success"
    );
}

#[test]
fn small_large_and_cross_agent_duplicate_scenarios_keep_presentation_facts() {
    let scenarios: Vec<PresentationScenario> = serde_json::from_slice(
        &fs::read(Path::new(FIXTURES).join("presentation-scenarios.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(scenarios.len(), 3);

    for scenario in scenarios {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let roots = known_agent_roots(&home);
        let first = roots[0].skill_roots[0].clone();
        let second = if scenario.cross_agent {
            roots[1].skill_roots[0].clone()
        } else {
            home.join(".agents_skills")
        };
        for index in 0..scenario.independent_skills {
            let contents = format!(
                "---\nname: {0}-skill-{1:03}\ndescription: synthetic {0} acceptance fixture {1}\n---\nStable body {1}.\n",
                scenario.name, index
            );
            for (copy, root) in [first.clone(), second.clone()]
                .into_iter()
                .take(scenario.copies_per_skill)
                .enumerate()
            {
                let directory = root.join(format!("skill-{index:03}-copy-{copy}"));
                fs::create_dir_all(&directory).unwrap();
                fs::write(directory.join("SKILL.md"), &contents).unwrap();
            }
        }

        let result = scan(&ScanOptions::for_home(home.clone())).unwrap();
        assert_eq!(result.skills.len(), scenario.independent_skills);
        assert_eq!(
            result.placements.len(),
            scenario.independent_skills * scenario.copies_per_skill
        );
        let report = build_report(&result);
        assert_eq!(
            report.metrics.independent_skills,
            scenario.independent_skills
        );
        assert_eq!(report.metrics.placements, result.placements.len());
        assert!(report.metrics.default_exposure > 0);
        assert!(report.findings.iter().any(|finding| {
            finding.category == FindingCategory::Overlap
                && finding.evidence_quality == EvidenceQuality::Observed
                && !finding.affected_placement_ids.is_empty()
                && !finding.evidence.is_empty()
        }));

        if scenario.name == "large" {
            assert!(report.metrics.independent_skills > 100);
        }
        if scenario.cross_agent {
            let exposed_agents = result
                .placements
                .iter()
                .filter_map(|placement| placement.agent)
                .collect::<BTreeSet<_>>();
            assert_eq!(exposed_agents.len(), 2);
        }
    }
}

#[test]
fn synthetic_value_comparison_reduces_exposure_without_task_success_loss() {
    let fixture: Value = serde_json::from_slice(
        &fs::read(Path::new(FIXTURES).join("value-comparison.json")).unwrap(),
    )
    .unwrap();
    let approaches = fixture["approaches"].as_array().unwrap();
    let by_name = |name: &str| {
        approaches
            .iter()
            .find(|approach| approach["name"] == name)
            .unwrap()
    };
    let unmanaged = by_name("unmanaged");
    let manual = by_name("careful_manual");
    let roster = by_name("skillroster");

    assert!(manual["default_exposure"].as_u64() < unmanaged["default_exposure"].as_u64());
    assert!(
        roster["default_exposure"].as_u64().unwrap() * 2
            <= unmanaged["default_exposure"].as_u64().unwrap()
    );
    assert!(roster["remaining_duplicates"].as_u64() < manual["remaining_duplicates"].as_u64());
    assert_eq!(roster["task_successes"], unmanaged["task_successes"]);
    assert_eq!(roster["reversible_receipts"], true);
    assert!(
        fixture["notes"]
            .as_str()
            .unwrap()
            .contains("Synthetic fixed")
    );
}

#[test]
fn plain_cli_preserves_report_contract_at_sixty_eighty_and_one_twenty_columns() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    copy_tree(&Path::new(FIXTURES).join("agents/home"), &home);

    let binary = env!("CARGO_BIN_EXE_skillroster");
    let scan_output = Command::new(binary)
        .args([
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "scan",
        ])
        .output()
        .unwrap();
    assert!(scan_output.status.success());
    assert!(scan_output.stderr.is_empty());
    let scan_json: Value = serde_json::from_slice(&scan_output.stdout).unwrap();
    assert_eq!(scan_json["ok"], true);

    for width in [60, 80, 120] {
        let output = Command::new(binary)
            .args([
                "--home",
                home.to_str().unwrap(),
                "--state-dir",
                state.to_str().unwrap(),
                "report",
            ])
            .env("COLUMNS", width.to_string())
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).unwrap();
        for field in [
            "Independent Skills",
            "Placements",
            "Default exposure",
            "Observed-use Agents",
            "Top Findings",
            "Category totals",
            "Read-only · no Agent files changed",
        ] {
            assert!(text.contains(field), "{field} missing at {width} columns");
        }
        assert!(!text.contains("\u{1b}["));
    }
}
