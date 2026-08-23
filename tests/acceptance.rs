use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;
use skillroster::harness::{AgentKind, known_agent_roots};
use skillroster::query::{FindingCategory, build_report};
use skillroster::scan::{EvidenceQuality, LinkStatus, ScanOptions, UsageStage, scan};
use tempfile::TempDir;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

#[derive(Debug, Deserialize)]
struct RouteCase {
    task: String,
    skill: String,
    #[serde(default)]
    hints: Vec<String>,
    #[serde(default)]
    rank_first: bool,
    #[serde(default)]
    max_matches: Option<usize>,
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

fn cli_json(home: &Path, state: &Path, args: &[&str], stdin: Option<&str>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skillroster"));
    command.args([
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ]);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "command {args:?} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], true);
    envelope
}

fn plan_evidence_id(home: &Path, state: &Path) -> String {
    let report = cli_json(home, state, &["report"], None);
    let finding = report["result"]["findings"]
        .as_array()
        .and_then(|findings| findings.first())
        .and_then(|finding| finding["id"].as_str())
        .expect("fixture report must expose a traceable Finding");
    let detail = cli_json(home, state, &["report", "--finding", finding], None);
    detail["result"]["items"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["evidence_id"].as_str())
        .expect("Finding must expose Evidence through the public CLI")
        .to_string()
}

fn public_skill_ids(home: &Path, state: &Path, names: &[String]) -> BTreeMap<String, String> {
    names
        .iter()
        .map(|name| {
            let found = cli_json(home, state, &["find", name, "--limit", "1"], None);
            let matched = &found["result"]["matches"][0];
            assert_eq!(matched["name"], name.as_str());
            (
                name.clone(),
                matched["skill_id"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn evaluate_capabilities(home: &Path, state: &Path, routes: &[RouteCase]) -> (usize, usize) {
    let mut routed = 0;
    let mut succeeded = 0;
    for case in routes {
        let mut arguments = vec!["find", case.task.as_str()];
        for hint in &case.hints {
            arguments.extend(["--hint", hint.as_str()]);
        }
        arguments.extend([
            "--limit",
            if case.max_matches.is_some() {
                "10"
            } else {
                "3"
            },
        ]);
        let found = cli_json(home, state, &arguments, None);
        let matches = found["result"]["matches"].as_array().unwrap();
        if case.rank_first {
            assert_eq!(
                matches.first().and_then(|matched| matched["name"].as_str()),
                Some(case.skill.as_str()),
                "dedicated capability must rank first for {:?}",
                case.task
            );
        }
        if let Some(max_matches) = case.max_matches {
            assert!(
                matches.len() <= max_matches,
                "low-confidence tail for {:?} had {} matches",
                case.task,
                matches.len()
            );
        }
        let Some(matched) = matches.iter().find(|matched| matched["name"] == case.skill) else {
            continue;
        };
        routed += 1;
        let contract = format!("CAPABILITY: {}", case.skill);
        let executable = matched["paths"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(Path::new)
            .find(|path| path.is_file())
            .and_then(|path| fs::read_to_string(path).ok())
            .is_some_and(|contents| contents.lines().any(|line| line.trim() == contract));
        succeeded += usize::from(executable);
    }
    (routed, succeeded)
}

fn populate_value_inventory(home: &Path) -> Vec<String> {
    let codex = home.join(".codex/skills");
    let claude = home.join(".claude/skills");
    let names = (0..120)
        .map(|index| format!("value-skill-{index:03}"))
        .collect::<Vec<_>>();
    for (index, name) in names.iter().enumerate() {
        let contents = format!(
            "---\nname: {name}\ndescription: deterministic value fixture {index}\n---\nCAPABILITY: {name}\n"
        );
        let primary = codex.join(name);
        fs::create_dir_all(&primary).unwrap();
        fs::write(primary.join("SKILL.md"), &contents).unwrap();
        if index < 80 {
            let duplicate = claude.join(name);
            fs::create_dir_all(&duplicate).unwrap();
            fs::write(duplicate.join("SKILL.md"), contents).unwrap();
        }
    }
    names
}

fn exact_duplicate_placements(home: &Path) -> usize {
    let result = scan(&ScanOptions::for_home(home.to_path_buf())).unwrap();
    let mut copies = BTreeMap::<&str, usize>::new();
    for placement in &result.placements {
        if placement.agent.is_some() && placement.link_status == LinkStatus::NotLink {
            *copies.entry(&placement.skill_id).or_default() += 1;
        }
    }
    copies.values().map(|count| count.saturating_sub(1)).sum()
}

fn agent_tree_signature(home: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, current: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        if !current.exists() {
            return;
        }
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root).unwrap().display().to_string(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }
    let mut output = Vec::new();
    for relative in [".codex/skills", ".claude/skills"] {
        walk(home, &home.join(relative), &mut output);
    }
    output.sort();
    output
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
fn reports_reuse_one_snapshot_but_scope_finding_ids_to_each_new_report() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = home.join(".skillroster");
    copy_tree(&Path::new(FIXTURES).join("agents/home"), &home);

    cli_json(&home, &state, &["scan"], None);
    let first = cli_json(&home, &state, &["report"], None);
    let repeated = cli_json(&home, &state, &["report"], None);
    assert_eq!(
        first["result"]["report_id"],
        repeated["result"]["report_id"]
    );
    assert_eq!(first["result"]["findings"], repeated["result"]["findings"]);

    cli_json(&home, &state, &["scan"], None);
    let next = cli_json(&home, &state, &["report"], None);
    assert_ne!(first["result"]["report_id"], next["result"]["report_id"]);
    let first_ids = first["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let next_ids = next["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert!(first_ids.is_disjoint(&next_ids));
}

#[test]
fn usage_finding_names_skills_and_uses_public_agent_ids() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = home.join(".skillroster");
    copy_tree(&Path::new(FIXTURES).join("agents/home"), &home);

    cli_json(&home, &state, &["scan"], None);
    let report = cli_json(
        &home,
        &state,
        &["report", "--findings", "--category", "usage"],
        None,
    );
    let finding_id = report["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Five-stage usage evidence")
        .and_then(|finding| finding["id"].as_str())
        .expect("usage Finding")
        .to_owned();

    let compact = cli_json(
        &home,
        &state,
        &["report", "--finding", &finding_id, "--limit", "100"],
        None,
    );
    let compact_claude = compact["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["kind"] == "usage"
                && item["facts"]["agent"] == "claude-code"
                && item["facts"]["stage"] == "loaded"
        })
        .expect("compact Claude Code Loaded evidence");
    assert_eq!(compact_claude["facts"]["skill_name"], "claude-code-fixture");
    let overview = &compact["result"]["usage_overview"];
    assert_eq!(overview["stages"].as_array().unwrap().len(), 5);
    assert_eq!(overview["coverage"]["supported_agent_count"], 8);
    assert_eq!(overview["coverage"]["roots_present_agent_count"], 8);
    assert_eq!(overview["coverage"]["sampled_agent_count"], 8);
    let observed_skills = overview["observed_skills"].as_array().unwrap();
    assert!(!observed_skills.is_empty());
    assert!(observed_skills.iter().all(|signal| {
        signal["skill_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("skill_"))
            && signal["skill_name"]
                .as_str()
                .is_some_and(|name| !name.is_empty())
            && signal["stage"] != "exposed"
    }));
    assert!(
        observed_skills
            .iter()
            .any(|signal| signal["stage"] == "loaded")
    );

    let full = cli_json(
        &home,
        &state,
        &[
            "report",
            "--finding",
            &finding_id,
            "--full",
            "--limit",
            "100",
        ],
        None,
    );
    let full_claude = full["result"]["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .find(|evidence| {
            evidence["kind"] == "usage"
                && evidence["details"]["agent"] == "claude-code"
                && evidence["details"]["stage"] == "loaded"
        })
        .expect("full Claude Code Loaded evidence");
    assert_eq!(full_claude["details"]["skill_name"], "claude-code-fixture");
    assert_eq!(
        compact["result"]["usage_overview"],
        full["result"]["usage_overview"]
    );
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

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = home.join(".skillroster");
    copy_tree(
        &Path::new(FIXTURES).join("routing-skills"),
        &home.join(".codex/skills"),
    );
    let before_signature = agent_tree_signature(&home);
    let scan_result = cli_json(&home, &state, &["scan"], None);
    let scan_id = scan_result["result"]["snapshot_id"].as_str().unwrap();
    let (before_routed, before_succeeded) = evaluate_capabilities(&home, &state, &routes);
    assert_eq!(
        before_routed,
        routes.len(),
        "Top-3 recall was {before_routed}/{}",
        routes.len()
    );
    assert_eq!(
        before_succeeded, before_routed,
        "every routed fixture must pass its independent capability contract"
    );

    let names = fs::read_dir(Path::new(FIXTURES).join("routing-skills"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let ids = public_skill_ids(&home, &state, &names);
    let evidence_id = plan_evidence_id(&home, &state);
    let core = BTreeSet::from(["research", "diagnose", "code-review"]);
    let roster_changes = names
        .iter()
        .map(|name| {
            serde_json::json!({
                "agent": "codex",
                "skill_id": ids[name],
                "state": if core.contains(name.as_str()) { "core" } else { "on_demand" }
            })
        })
        .collect::<Vec<_>>();
    let request = serde_json::json!({
        "schema_version": 1,
        "scan_id": scan_id,
        "evidence_ids": [evidence_id],
        "roster_changes": roster_changes
    });
    let plan = cli_json(
        &home,
        &state,
        &["plan", "--stdin"],
        Some(&request.to_string()),
    );
    let applied = cli_json(
        &home,
        &state,
        &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(applied["result"]["verification"], "passed");

    let (after_routed, after_succeeded) = evaluate_capabilities(&home, &state, &routes);
    assert_eq!(
        after_routed, before_routed,
        "Apply must not regress Top-3 routing"
    );
    assert_eq!(
        after_succeeded, before_succeeded,
        "Apply must preserve readable, executable capability contracts"
    );

    let undone = cli_json(
        &home,
        &state,
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(agent_tree_signature(&home), before_signature);
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
fn three_arm_value_comparison_runs_real_filesystem_governance_and_restore() {
    #[derive(Debug)]
    struct Measured {
        exposure: u64,
        duplicates: usize,
    }
    let measure = |home: &Path, state: &Path| {
        cli_json(home, state, &["scan"], None);
        let report = cli_json(home, state, &["report"], None);
        Measured {
            exposure: report["result"]["default_exposure"].as_u64().unwrap(),
            duplicates: exact_duplicate_placements(home),
        }
    };

    // Arm 1: leave the duplicated Agent roots unmanaged and measure them.
    let unmanaged_temp = TempDir::new().unwrap();
    let unmanaged_home = unmanaged_temp.path().join("home");
    populate_value_inventory(&unmanaged_home);
    let unmanaged = measure(&unmanaged_home, &unmanaged_home.join(".skillroster"));

    // Arm 2: perform a declared manual procedure using only filesystem moves,
    // hard links, and a manifest. The resulting metrics are scanned, not filled in.
    let manual_temp = TempDir::new().unwrap();
    let manual_home = manual_temp.path().join("home");
    let names = populate_value_inventory(&manual_home);
    let library = manual_home.join(".agents_skills");
    fs::create_dir_all(&library).unwrap();
    let mut manifest = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let source = manual_home.join(".codex/skills").join(name);
        let canonical = library.join(name);
        fs::rename(&source, &canonical).unwrap();
        let duplicate = manual_home.join(".claude/skills").join(name);
        // The careful-manual arm has an explicit scope budget: it resolves 70
        // of the 80 cross-Agent copies and leaves 10 documented in the manifest.
        if index >= 10 && duplicate.exists() {
            fs::remove_dir_all(&duplicate).unwrap();
        }
        let state = if index < 54 { "core" } else { "on_demand" };
        if state == "core" {
            let exposed = manual_home.join(".codex/skills").join(name);
            fs::create_dir_all(&exposed).unwrap();
            fs::hard_link(canonical.join("SKILL.md"), exposed.join("SKILL.md")).unwrap();
        }
        manifest.push(serde_json::json!({
            "skill": name,
            "state": state,
            "unresolved_cross_agent_copy": index < 10
        }));
    }
    fs::write(
        library.join("manual-roster.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let manual = measure(&manual_home, &manual_home.join(".skillroster"));
    assert_eq!(
        manual.duplicates, 10,
        "manual scope must be measured, not assumed"
    );

    // Arm 3: exercise the public SkillRoster loop and prove Receipt-bounded Undo.
    let roster_temp = TempDir::new().unwrap();
    let roster_home = roster_temp.path().join("home");
    let roster_state = roster_home.join(".skillroster");
    let names = populate_value_inventory(&roster_home);
    let before_signature = agent_tree_signature(&roster_home);
    let scan_result = cli_json(&roster_home, &roster_state, &["scan"], None);
    let scan_id = scan_result["result"]["snapshot_id"].as_str().unwrap();
    let ids = public_skill_ids(&roster_home, &roster_state, &names);
    let evidence_id = plan_evidence_id(&roster_home, &roster_state);
    let mut roster_changes = Vec::new();
    for (index, name) in names.iter().enumerate() {
        roster_changes.push(serde_json::json!({
            "agent": "codex",
            "skill_id": ids[name],
            "state": if index < 36 { "core" } else { "on_demand" }
        }));
        if index < 80 {
            roster_changes.push(serde_json::json!({
                "agent": "claude-code",
                "skill_id": ids[name],
                "state": "on_demand"
            }));
        }
    }
    let request = serde_json::json!({
        "schema_version": 1,
        "scan_id": scan_id,
        "evidence_ids": [evidence_id],
        "roster_changes": roster_changes
    });
    let plan = cli_json(
        &roster_home,
        &roster_state,
        &["plan", "--stdin"],
        Some(&request.to_string()),
    );
    let applied = cli_json(
        &roster_home,
        &roster_state,
        &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(applied["result"]["verification"], "passed");
    let roster = measure(&roster_home, &roster_state);

    assert!(manual.exposure < unmanaged.exposure);
    assert!(roster.exposure * 2 <= unmanaged.exposure);
    assert!(manual.duplicates < unmanaged.duplicates);
    assert!(roster.duplicates < manual.duplicates);

    let undone = cli_json(
        &roster_home,
        &roster_state,
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(agent_tree_signature(&roster_home), before_signature);
    let restored = measure(&roster_home, &roster_state);
    assert_eq!(restored.exposure, unmanaged.exposure);
    assert_eq!(restored.duplicates, unmanaged.duplicates);
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
