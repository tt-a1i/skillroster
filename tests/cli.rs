use std::fs;
use std::fs::{FileTimes, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use fs2::FileExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn run(args: &[&str], stdin: Option<&str>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skillroster"));
    command.args(args);
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn json_output(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn report_help_names_the_safe_default_and_explicit_exhaustive_export() {
    let output = run(&["report", "--help"], None);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(
        help.contains("Defaults to the bounded Summary view"),
        "{help}"
    );
    assert!(help.contains("--full"), "{help}");
    assert!(help.contains("exhaustive report"), "{help}");
}

#[test]
fn healthy_status_does_not_suggest_an_unconditional_rescan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/skills")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let missing_snapshot = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(
        missing_snapshot["suggested_actions"][0]["argv"],
        json!(["skillroster", "scan", "--json"])
    );
    assert_eq!(
        missing_snapshot["suggested_actions"][0]["reason_code"],
        "snapshot_required"
    );

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let healthy = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert!(healthy["result"]["latest_snapshot_id"].is_string());
    assert_eq!(healthy["result"]["recovery_state"], "clear");
    assert_eq!(healthy["suggested_actions"], json!([]));
}

#[test]
fn public_cli_exits_quietly_when_the_output_consumer_closes() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    for index in 0..80 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\ndescription: fixture\n---\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"][0]["id"].as_str().unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .args([&common[..], &["report", "--finding", finding_id, "--full"]].concat())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_find_paths_are_readable(found: &Value) {
    let paths = found["result"]["matches"][0]["paths"]
        .as_array()
        .expect("find paths");
    assert!(!paths.is_empty());
    for path in paths {
        fs::File::open(path.as_str().unwrap()).expect("find must return a readable SKILL.md");
    }
}

#[test]
fn public_find_keeps_the_user_task_and_uses_agent_retrieval_hints() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/archify");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: archify\ndescription: Create interactive architecture workflow diagrams\n---\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let unhinted = json_output(&run(
        &[&common[..], &["find", "把架构流程画成可交互图"]].concat(),
        None,
    ));
    assert!(unhinted["result"]["matches"].as_array().unwrap().is_empty());
    assert_eq!(unhinted["result"]["warnings"].as_array().unwrap().len(), 1);
    assert!(
        unhinted["result"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("--hint"))
    );

    let found = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "把架构流程画成可交互图",
                "--hint",
                "interactive architecture workflow diagram",
                "--hint",
                "  interactive architecture workflow diagram  ",
            ],
        ]
        .concat(),
        None,
    ));

    assert_eq!(found["result"]["task"], "把架构流程画成可交互图");
    assert_eq!(
        found["result"]["retrieval_hints"],
        json!(["interactive architecture workflow diagram"])
    );
    assert_eq!(found["result"]["matches"][0]["name"], "archify");
    assert_eq!(
        found["result"]["ranking_strategy"],
        "task_hint_reciprocal_rank_fusion"
    );
}

#[test]
fn public_find_hints_do_not_erase_a_native_task_match() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    let native = skill_root.join("humanizer-zh");
    fs::create_dir_all(&native).unwrap();
    fs::write(
        native.join("SKILL.md"),
        "---\nname: humanizer-zh\ndescription: 把中文文章改得更自然，更像人类写的。\n---\n保留原意并去掉机器表达。\n",
    )
    .unwrap();
    for name in [
        "ai-tone-editor",
        "content-polisher",
        "english-humanizer",
        "writing-assistant",
        "x-post-writer",
        "zh-copy-editor",
    ] {
        let directory = skill_root.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: Humanize Chinese writing, remove AI tone, and polish English posts.\n---\n"
            ),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let task = "把这篇中文文章改得像人写的";
    let unhinted = json_output(&run(
        &[&common[..], &["find", task, "--limit", "3"]].concat(),
        None,
    ));
    assert!(
        unhinted["result"]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|matched| matched["name"] == "humanizer-zh")
    );

    let hinted = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                task,
                "--hint",
                "humanize Chinese writing remove AI tone",
                "--limit",
                "3",
            ],
        ]
        .concat(),
        None,
    ));
    let native = hinted["result"]["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|matched| matched["name"] == "humanizer-zh")
        .expect("the task-only native match must survive hint fusion");
    assert!(native["rank"].as_u64().unwrap() <= 3);
    assert!(native["task_channel_rank"].as_u64().unwrap() <= 3);
    assert!(native["augmented_channel_rank"].is_number());
    assert_eq!(
        hinted["result"]["ranking_strategy"],
        "task_hint_reciprocal_rank_fusion"
    );
}

#[test]
fn public_find_hinted_ranking_is_prefix_stable_across_limits() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    for (directory, contents) in [
        (
            "analyze-data-quality",
            "---\nname: analyze-data-quality\ndescription: Assess whether structured data and analytical evidence are trustworthy. Use when the task is to check data quality.\n---\n",
        ),
        (
            "spreadsheets",
            "---\nname: Spreadsheets\ndescription: Create, edit, analyze, and verify standalone spreadsheet files and workbooks, including Excel files.\n---\n",
        ),
        (
            "local-data-quality",
            "---\nname: local-data-quality\ndescription: 分析本地表格数据和数据质量。\n---\n",
        ),
        (
            "local-report-storage",
            "---\nname: local-report-storage\ndescription: 管理本地表格数据和数据质量报告文件。\n---\n",
        ),
    ] {
        let path = skill_root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), contents).unwrap();
    }
    for index in 0..24 {
        let path = skill_root.join(format!("spreadsheet-helper-{index:02}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!(
                "---\nname: spreadsheet-helper-{index:02}\ndescription: Generic spreadsheet helper number {index}.\n---\n"
            ),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let task = "分析一个本地表格的数据质量";
    let hint = "analyze standalone spreadsheet workbook data quality";
    let find_names = |limit: &str| {
        let found = json_output(&run(
            &[
                &common[..],
                &[
                    "find",
                    task,
                    "--hint",
                    hint,
                    "--hint",
                    "Spreadsheets",
                    "--limit",
                    limit,
                ],
            ]
            .concat(),
            None,
        ));
        found["result"]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|matched| matched["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    let complete_ranking = find_names("10");
    assert_eq!(complete_ranking[0], "Spreadsheets");
    assert!(
        complete_ranking[..3]
            .iter()
            .any(|name| name == "local-data-quality")
    );
    assert!(complete_ranking.iter().any(|name| name == "Spreadsheets"));
    assert!(
        complete_ranking
            .iter()
            .any(|name| name == "analyze-data-quality")
    );
    for limit in 1..=10 {
        let bounded = find_names(&limit.to_string());
        assert_eq!(bounded, complete_ranking[..bounded.len()]);
    }
}

#[test]
fn public_find_routes_natural_cjk_paraphrases_against_cjk_metadata() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    let humanizer = skill_root.join("humanizer-zh");
    let unrelated = skill_root.join("generic-writer");
    fs::create_dir_all(&humanizer).unwrap();
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(
        humanizer.join("SKILL.md"),
        "---\nname: humanizer-zh\ndescription: 去除文本中的 AI 生成痕迹，让中文表达更自然、更像人类书写。\n---\n编辑中文并保留原意。\n",
    )
    .unwrap();
    fs::write(
        unrelated.join("SKILL.md"),
        "---\nname: generic-writer\ndescription: Generic English writing helper\n---\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let found = json_output(&run(
        &[&common[..], &["find", "把中文改自然一点", "--limit", "3"]].concat(),
        None,
    ));

    assert_eq!(found["result"]["task"], "把中文改自然一点");
    assert_eq!(found["result"]["matches"][0]["name"], "humanizer-zh");
    assert!(
        found["result"]["matches"][0]["match_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason
                .as_str()
                .unwrap()
                .starts_with("cjk_description_bigrams:"))
    );
    assert_find_paths_are_readable(&found);
}

#[test]
fn public_find_expands_plural_candidates_and_bounds_incidental_single_token_matches() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    let blog = skill_root.join("blog");
    fs::create_dir_all(&blog).unwrap();
    fs::write(
        blog.join("SKILL.md"),
        "---\nname: blog\ndescription: Publish a technical article\n---\n",
    )
    .unwrap();
    for index in 0..8 {
        let skill = skill_root.join(format!("incidental-{index}"));
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!(
                "---\nname: incidental-{index}\ndescription: Generic helper\n---\nMentions archive incidentally.\n"
            ),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let plural = json_output(&run(
        &[&common[..], &["find", "blogs", "--limit", "10"]].concat(),
        None,
    ));
    assert_eq!(plural["result"]["matches"][0]["name"], "blog");

    let incidental = json_output(&run(
        &[&common[..], &["find", "archive", "--limit", "100"]].concat(),
        None,
    ));
    assert_eq!(incidental["result"]["matches"].as_array().unwrap().len(), 3);
}

#[test]
fn finding_drilldown_is_bounded_and_pageable() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    for index in 0..51 {
        let directory = skill_root.join(format!("skill-{index:02}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!(
                "---\nname: skill-{index:02}\ndescription: Dedicated task number {index}\n---\n"
            ),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let invalid_full = run(
        &[&common[..], &["report", "--full", "--summary"]].concat(),
        None,
    );
    assert!(!invalid_full.status.success());
    let invalid_full: Value = serde_json::from_slice(&invalid_full.stdout).unwrap();
    assert_eq!(invalid_full["error"]["code"], "invalid_cli_arguments");
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["category"] == "exposure")
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let first_output = run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    );
    assert!(first_output.stdout.len() < 20_000);
    let first = json_output(&first_output);
    assert_eq!(first["result"]["page"]["offset"], 0);
    assert_eq!(first["result"]["page"]["limit"], 20);
    assert_eq!(first["result"]["page"]["next_offset"], 20);
    assert_eq!(first["result"]["items"].as_array().unwrap().len(), 20);
    assert_eq!(first["result"]["detail"]["mode"], "compact");
    assert!(first["result"].get("placements").is_none());
    assert!(first["result"].get("affected_placement_ids").is_none());
    assert!(first["result"].get("evidence_ids").is_none());

    let second = json_output(&run(
        &[
            &common[..],
            &[
                "report",
                "--finding",
                finding_id,
                "--offset",
                "20",
                "--limit",
                "20",
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(second["result"]["page"]["offset"], 20);
    assert_ne!(
        first["result"]["items"][0]["evidence_id"],
        second["result"]["items"][0]["evidence_id"]
    );

    let full_output = run(
        &[&common[..], &["report", "--finding", finding_id, "--full"]].concat(),
        None,
    );
    let full = json_output(&full_output);
    assert_eq!(full["result"]["detail"]["mode"], "full");
    assert!(full["result"]["placements"].is_array());
    assert!(full["result"]["evidence"].is_array());
    assert!(full["result"]["affected_placement_ids"].is_array());
    assert!(full_output.stdout.len() > first_output.stdout.len());
}

#[test]
fn finding_list_is_paged_filterable_and_leads_to_evidence() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    for index in 0..30 {
        let content = format!(
            "---\nname: duplicate-{index:02}\ndescription: Dedicated duplicate task {index}\n---\n"
        );
        for root in [".codex/skills", ".claude/skills"] {
            let directory = home.join(root).join(format!("duplicate-{index:02}"));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("SKILL.md"), &content).unwrap();
        }
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let summary = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let default_report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    assert_eq!(
        default_report["result"]["findings"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        default_report["result"]["finding_count"],
        summary["result"]["finding_count"]
    );
    assert_eq!(
        default_report["result"]["finding_rollups"],
        summary["result"]["finding_rollups"]
    );
    assert_eq!(default_report["result"]["files_changed"], false);
    let exhaustive_report = json_output(&run(&[&common[..], &["report", "--full"]].concat(), None));
    assert_eq!(
        exhaustive_report["result"]["findings"]
            .as_array()
            .unwrap()
            .len(),
        60
    );
    assert_eq!(exhaustive_report["result"]["files_changed"], false);
    let duplicate_rollup = summary["result"]["finding_rollups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rollup| rollup["title"] == "Exact duplicate Skill placements")
        .unwrap();
    assert_eq!(duplicate_rollup["finding_count"], 30);
    assert_eq!(duplicate_rollup["affected_skill_count"], 30);
    assert_eq!(duplicate_rollup["affected_placement_count"], 60);
    assert_eq!(summary["suggested_actions"][0]["action"], "list_findings");
    assert_eq!(
        summary["suggested_actions"][0]["argv"],
        json!([
            "skillroster",
            "report",
            "--findings",
            "--limit",
            "20",
            "--offset",
            "0",
            "--json"
        ])
    );

    let first_output = run(
        &[
            &common[..],
            &[
                "report",
                "--findings",
                "--category",
                "overlap",
                "--severity",
                "medium",
                "--limit",
                "10",
            ],
        ]
        .concat(),
        None,
    );
    assert!(first_output.stdout.len() < 20_000);
    let first = json_output(&first_output);
    assert_eq!(first["result"]["view"], "findings");
    assert_eq!(first["result"]["matched_finding_count"], 30);
    assert_eq!(first["result"]["page"]["returned"], 10);
    assert_eq!(first["result"]["page"]["next_offset"], 10);
    assert_eq!(
        first["result"]["finding_rollups"],
        summary["result"]["finding_rollups"]
    );
    assert_eq!(first["result"]["items"].as_array().unwrap().len(), 10);
    assert!(
        first["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["category"] == "overlap"
                && finding["severity"] == "medium"
                && finding.get("evidence_ids").is_none())
    );
    assert_eq!(
        first["suggested_actions"][0]["argv"],
        json!([
            "skillroster",
            "report",
            "--findings",
            "--category",
            "overlap",
            "--severity",
            "medium",
            "--limit",
            "10",
            "--offset",
            "10",
            "--json"
        ])
    );

    let second = json_output(&run(
        &[
            &common[..],
            &[
                "report",
                "--findings",
                "--category",
                "overlap",
                "--severity",
                "medium",
                "--limit",
                "10",
                "--offset",
                "10",
            ],
        ]
        .concat(),
        None,
    ));
    assert_ne!(
        first["result"]["items"][0]["id"],
        second["result"]["items"][0]["id"]
    );

    let finding_id = first["result"]["items"][0]["id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert!(!detail["result"]["items"].as_array().unwrap().is_empty());

    for invalid in [
        vec!["report", "--category", "overlap"],
        vec!["report", "--summary", "--findings"],
    ] {
        let output = run(&[&common[..], &invalid].concat(), None);
        assert!(!output.status.success());
        let output: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output["error"]["code"], "invalid_cli_arguments");
    }
}

#[test]
fn same_name_divergent_finding_keeps_variant_paths_and_requires_a_choice() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex = home.join(".codex/skills/shared-capability");
    let claude = home.join(".claude/skills/shared-capability");
    fs::create_dir_all(&codex).unwrap();
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        codex.join("SKILL.md"),
        "---\nname: shared-capability\ndescription: First implementation\n---\nalpha\n",
    )
    .unwrap();
    fs::write(
        claude.join("SKILL.md"),
        "---\nname: shared-capability\ndescription: Second implementation\n---\nbeta\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["title"] == "Same-name Skills have different content"
                && finding["summary"]
                    .as_str()
                    .unwrap()
                    .contains("shared-capability")
        })
        .unwrap();
    assert_eq!(finding["affected_skill_count"], 2);
    assert_eq!(finding["affected_placement_count"], 2);
    let finding_id = finding["id"].as_str().unwrap();

    let compact = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(
        compact["result"]["resolution"]["decision"],
        "choose_same_name_variant"
    );
    assert_eq!(
        compact["result"]["resolution"]["automatic_change_supported"],
        false
    );
    let variants = compact["result"]["resolution"]["variants"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 2);
    for variant in variants {
        assert!(variant["skill_id"].as_str().unwrap().starts_with("skill_"));
        assert_eq!(variant["content_digests"].as_array().unwrap().len(), 1);
        assert_eq!(variant["paths"].as_array().unwrap().len(), 1);
        assert!(Path::new(variant["paths"][0].as_str().unwrap()).is_file());
        assert_eq!(variant["agents"].as_array().unwrap().len(), 1);
    }
    assert!(
        compact["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );

    let full = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id, "--full"]].concat(),
        None,
    ));
    assert_eq!(full["result"]["placements"].as_array().unwrap().len(), 2);
}

#[test]
fn public_cli_scans_reports_plans_applies_and_undoes() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    let skill = skill_root.join("example");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: example\ndescription: Example task helper\ntriggers: [example]\n---\n",
    )
    .unwrap();
    let second_skill = home.join(".claude/skills/example");
    fs::create_dir_all(&second_skill).unwrap();
    fs::write(
        second_skill.join("SKILL.md"),
        "---\nname: example\ndescription: Example task helper\ntriggers: [example]\n---\n",
    )
    .unwrap();
    let session_root = home.join(".codex/sessions");
    fs::create_dir_all(&session_root).unwrap();
    fs::write(
        session_root.join("session.jsonl"),
        "{\"type\":\"invoke_skill\",\"invoked_skill\":\"example\"}\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(scan["ok"], true);
    assert_eq!(scan["result"]["skill_count"], 1);
    assert_eq!(scan["result"]["files_changed"], false);
    assert_eq!(
        scan["suggested_actions"][0]["argv"],
        serde_json::json!(["skillroster", "report", "--json"])
    );
    let codex_coverage = scan["result"]["coverage"]
        .as_array()
        .unwrap()
        .iter()
        .find(|coverage| coverage["agent"] == "codex")
        .unwrap();
    assert_eq!(codex_coverage["files_discovered"], 1);
    assert_eq!(codex_coverage["files_observed"], 1);
    assert_eq!(codex_coverage["files_partially_observed"], 0);
    assert_eq!(codex_coverage["denominator_reliable"], true);
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    for (table, minimum) in [
        ("roots", 16_i64),
        ("placements", 1),
        ("evidence", 1),
        ("usage_events", 1),
    ] {
        let count: i64 = database
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(count >= minimum, "expected normalized rows in {table}");
    }

    let initial_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(initial_status["result"]["retention"]["raw_usage_days"], 180);
    assert_eq!(initial_status["result"]["pending_plan_count"], 0);

    let report_output = run(&[&common[..], &["report"]].concat(), None);
    let report_output_len = report_output.stdout.len();
    let report = json_output(&report_output);
    assert_eq!(report["ok"], true);
    assert!(report["result"]["findings"].is_array());
    assert!(report["result"]["primary_metrics"].is_object());
    assert_eq!(report["result"]["observed_use_agent_count"], 1);
    assert_eq!(report["result"]["coverage_root_agent_count"], 1);
    assert_eq!(report["result"]["coverage_sampled_agent_count"], 1);
    assert_eq!(report["result"]["coverage_reliable_agent_count"], 1);
    assert_eq!(report["result"]["coverage_limited_agent_count"], 0);
    assert_eq!(report["result"]["coverage_missing_agent_count"], 7);
    assert_eq!(report["result"]["coverage_inaccessible_agent_count"], 0);
    assert_eq!(
        report["result"]["session_coverage"],
        json!({
            "supported_agents": 8,
            "roots_present_agents": 1,
            "sampled_agents": 1,
            "complete_agents": 1,
            "limited_agents": 0,
            "missing_root_agents": 7,
            "inaccessible_agents": 0
        })
    );
    let repeated_report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    assert_eq!(
        repeated_report["result"]["report_id"],
        report["result"]["report_id"]
    );
    let summary_report_output = run(&[&common[..], &["report", "--summary"]].concat(), None);
    let summary_report = json_output(&summary_report_output);
    assert_eq!(
        summary_report["result"]["finding_count"],
        report["result"]["finding_count"]
    );
    assert!(
        summary_report["result"]["findings"]
            .as_array()
            .unwrap()
            .len()
            <= 3
    );
    assert_eq!(
        summary_report["result"]["findings"],
        report["result"]["findings"]
    );
    let full_report_output = run(&[&common[..], &["report", "--full"]].concat(), None);
    let full_report = json_output(&full_report_output);
    assert_eq!(
        full_report["result"]["findings"].as_array().unwrap().len(),
        report["result"]["finding_count"].as_u64().unwrap() as usize
    );
    assert!(full_report_output.stdout.len() > report_output_len);
    assert_eq!(
        summary_report["result"]["session_coverage"],
        report["result"]["session_coverage"]
    );
    for finding in summary_report["result"]["findings"].as_array().unwrap() {
        assert!(finding.get("affected_skill_ids").is_none());
        assert!(finding.get("affected_placement_ids").is_none());
        assert!(finding.get("evidence_ids").is_none());
        assert!(finding.get("primary_evidence_id").is_some());
        assert!(finding["affected_skill_count"].is_number());
        assert!(finding["affected_placement_count"].is_number());
    }
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["affected_placement_count"]
                .as_u64()
                .unwrap_or_default()
                > 0
        })
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let finding = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(finding["result"]["id"], finding_id);
    assert!(finding["result"].get("affected_skill_ids").is_none());
    assert!(finding["result"].get("affected_placement_ids").is_none());
    assert!(finding["result"]["impact"].is_object());
    assert!(finding["result"]["coverage"].is_object());
    assert!(
        finding["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["path"].as_str() == Some(skill.join("SKILL.md").to_str().unwrap()))
    );
    assert!(!finding["result"]["items"].as_array().unwrap().is_empty());
    for item in finding["result"]["items"].as_array().unwrap() {
        let exists: i64 = database
            .query_row(
                "SELECT COUNT(*) FROM evidence WHERE id = ?1",
                [item["evidence_id"].as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "Finding Evidence must be traceable");
    }

    let find = json_output(&run(
        &[&common[..], &["find", "example task"]].concat(),
        None,
    ));
    assert_eq!(find["result"]["matches"][0]["name"], "example");
    let skill_id = find["result"]["matches"][0]["skill_id"].as_str().unwrap();
    let evidence_id: String = database
        .query_row(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let setup_plan = setup["result"]["plan_id"].as_str().unwrap();
    assert!(setup["result"]["operation_count"].as_u64().unwrap() > 0);
    assert!(setup["result"].get("operations").is_none());
    assert!(!setup["result"].to_string().contains("## Safety boundaries"));
    let setup_applied = json_output(&run(&[&common[..], &["apply", setup_plan]].concat(), None));
    assert!(skill_root.join("skillroster/SKILL.md").is_file());
    let setup_receipt = setup_applied["result"]["receipt_id"].as_str().unwrap();
    let setup_undone = json_output(&run(
        &[&common[..], &["undo", setup_receipt]].concat(),
        None,
    ));
    assert_eq!(setup_undone["result"]["verification"], "passed");
    assert!(!skill_root.join("skillroster").exists());

    let plan_input = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": [{
            "agent": "codex",
            "skill_id": skill_id,
            "state": "on_demand"
        }]
    })
    .to_string();
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&plan_input),
    ));
    assert_eq!(plan["result"]["files_changed"], false);
    assert_eq!(plan["result"]["evidence"]["ids"][0], evidence_id);
    assert_eq!(plan["result"]["change_summary"]["roster_change_count"], 1);
    assert_eq!(
        plan["result"]["impact"]["roster"]["after_state_counts"]["on_demand"],
        1
    );
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();

    let agent_id: String = database
        .query_row("SELECT id FROM agents WHERE kind = 'codex'", [], |row| {
            row.get(0)
        })
        .unwrap();
    database
        .execute(
            "INSERT INTO roster_entries (agent_id, skill_id, state, updated_at)
             VALUES (?1, ?2, 'core', 0)",
            rusqlite::params![agent_id, skill_id],
        )
        .unwrap();
    let drifted = run(&[&common[..], &["apply", plan_id]].concat(), None);
    assert!(!drifted.status.success(), "Apply must reject Roster drift");
    database.execute("DELETE FROM roster_entries", []).unwrap();

    let pending_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(pending_status["result"]["pending_plan_count"], 1);

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(applied["result"]["verification"], "passed");
    let roster_state: String = database
        .query_row("SELECT state FROM roster_entries", [], |row| row.get(0))
        .unwrap();
    assert_eq!(roster_state, "on_demand");
    let receipt = applied["result"]["receipt_id"].as_str().unwrap();
    let receipt_verification: String = database
        .query_row(
            "SELECT verification_json FROM receipts WHERE id = ?1",
            [receipt],
            |row| row.get(0),
        )
        .unwrap();
    let receipt_verification: Value = serde_json::from_str(&receipt_verification).unwrap();
    assert_eq!(receipt_verification["evidence_ids"][0], evidence_id);
    let applied_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(
        applied_status["result"]["last_receipt"]["receipt_id"],
        receipt
    );

    let export_path = temp.path().join("lifecycle.json");
    let exported = json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                export_path.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(exported["result"]["operation"], "export");
    let export: Value = serde_json::from_slice(&fs::read(&export_path).unwrap()).unwrap();
    assert_eq!(export["retention"]["raw_usage_days"], 180);
    let overwrite = run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                export_path.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    );
    assert!(!overwrite.status.success());

    let purged = json_output(&run(
        &[&common[..], &["lifecycle", "purge", "--raw-days", "180"]].concat(),
        None,
    ));
    assert_eq!(purged["result"]["plans_or_receipts_changed"], false);
    assert_eq!(purged["result"]["agent_files_changed"], false);

    let shared_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("write.lock"))
        .unwrap();
    FileExt::try_lock_shared(&shared_lock).unwrap();
    let concurrent_status = run(&[&common[..], &["status"]].concat(), None);
    assert!(concurrent_status.status.success());
    let blocked_purge = run(
        &[&common[..], &["lifecycle", "purge", "--raw-days", "180"]].concat(),
        None,
    );
    assert!(!blocked_purge.status.success());
    FileExt::unlock(&shared_lock).unwrap();
    drop(shared_lock);

    let write_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("write.lock"))
        .unwrap();
    write_lock.try_lock_exclusive().unwrap();
    let locked_purge = run(
        &[&common[..], &["lifecycle", "purge", "--raw-days", "180"]].concat(),
        None,
    );
    assert!(!locked_purge.status.success());
    let locked_purge: Value = serde_json::from_slice(&locked_purge.stdout).unwrap();
    assert_eq!(locked_purge["error"]["code"], "write_locked");
    let locked_status = run(&[&common[..], &["status"]].concat(), None);
    assert!(!locked_status.status.success());
    let locked_status: Value = serde_json::from_slice(&locked_status.stdout).unwrap();
    assert_eq!(locked_status["error"]["code"], "write_locked");
    let locked_delete = run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    );
    assert!(!locked_delete.status.success());
    let locked_delete: Value = serde_json::from_slice(&locked_delete.stdout).unwrap();
    assert_eq!(locked_delete["error"]["code"], "write_locked");
    FileExt::unlock(&write_lock).unwrap();
    drop(write_lock);

    let undone = json_output(&run(&[&common[..], &["undo", receipt]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    let original_status: String = database
        .query_row(
            "SELECT status FROM receipts WHERE id = ?1",
            [receipt],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original_status, "undone");
    let roster_count: i64 = database
        .query_row("SELECT COUNT(*) FROM roster_entries", [], |row| row.get(0))
        .unwrap();
    assert_eq!(roster_count, 0);

    let repeated = run(&[&common[..], &["undo", receipt]].concat(), None);
    assert!(!repeated.status.success());
    let repeated: Value = serde_json::from_slice(&repeated.stdout).unwrap();
    assert_eq!(repeated["ok"], false);
    let recovery_count: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM receipts WHERE status = 'recovery_required'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recovery_count, 0);

    let unconfirmed_history_purge = run(
        &[&common[..], &["lifecycle", "purge", "--plans-receipts"]].concat(),
        None,
    );
    assert!(!unconfirmed_history_purge.status.success());
    let history = json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "purge",
                "--plans-receipts",
                "--confirm",
                "PURGE-PLANS-RECEIPTS",
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(history["result"]["plans_or_receipts_changed"], true);
    assert_eq!(history["result"]["files_changed"], true);
    assert_eq!(history["result"]["agent_files_changed"], false);
    assert_eq!(history["result"]["library_files_changed"], false);
    assert!(
        history["result"]["plan_receipt_result"]["recovery_directories"]
            .as_u64()
            .unwrap()
            > 0,
        "purging Receipt history must also remove its recovery content"
    );
    for table in ["plans", "plan_operations", "receipts", "receipt_operations"] {
        let count: i64 = database
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "explicit history purge must clear {table}");
    }
    assert!(skill.join("SKILL.md").is_file());
}

#[test]
fn source_confirmation_details_follow_public_lifecycle() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let details = state.join("source-confirmation");
    fs::create_dir(&details).unwrap();
    let artifact = details.join(format!("{}.json", ulid::Ulid::new()));
    let reviewed = temp.path().join("reviewed");
    let artifact_payload = json!({
        "schema_version": 1,
        "reason": "trusted_canonical_sources_required",
        "decision": "confirm_trusted_source_roots",
        "requested_core_budget": 10,
        "blocked_change_count": 1,
        "blocked_changes": [{
            "agent": "codex",
            "skill_id": "skill_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "name": "fixture",
            "reason": "no_owned_exact_content_to_preserve",
            "state": "unchanged",
            "observed_source_target": reviewed
        }],
        "skill_ids": ["skill_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        "source_root_count": 1,
        "source_roots": [reviewed],
        "after_confirmation": {
            "repeatable_option": "--source-root",
            "source_roots": [reviewed],
            "argv": ["skillroster", "--source-root", reviewed, "scan", "--json"]
        }
    });
    fs::write(&artifact, serde_json::to_vec(&artifact_payload).unwrap()).unwrap();

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(
        status["result"]["retention"]["source_confirmation_details"]["count"],
        1
    );
    let inspect = json_output(&run(
        &[&common[..], &["lifecycle", "inspect"]].concat(),
        None,
    ));
    assert_eq!(
        inspect["result"]["counts"]["source_confirmation_details"],
        1
    );
    assert_eq!(
        inspect["result"]["source_confirmation_details"]["retention"],
        "until_explicit_purge_or_delete"
    );

    let export_path = temp.path().join("lifecycle-details.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                export_path.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let exported: Value = serde_json::from_slice(&fs::read(export_path).unwrap()).unwrap();
    assert_eq!(
        exported["source_confirmation_details"][0]["schema_version"],
        1
    );

    let purged = json_output(&run(
        &[
            &common[..],
            &["lifecycle", "purge", "--source-confirmation"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(purged["result"]["removed_source_confirmation_details"], 1);
    assert!(!details.exists());

    fs::create_dir(&details).unwrap();
    let user_json = details.join(format!("{}.json", ulid::Ulid::new()));
    let mut lookalike_payload = artifact_payload.clone();
    lookalike_payload["after_confirmation"]["argv"] = json!(["skillroster", "lifecycle", "delete"]);
    fs::write(&user_json, serde_json::to_vec(&lookalike_payload).unwrap()).unwrap();
    let refused = run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    );
    assert!(!refused.status.success());
    assert!(user_json.is_file());
    assert!(state.join("skillroster.db").is_file());
    fs::remove_file(&user_json).unwrap();
    let non_minimal_json = details.join(format!("{}.json", ulid::Ulid::new()));
    let mut non_minimal_payload = artifact_payload.clone();
    let child = reviewed.join("child");
    non_minimal_payload["source_root_count"] = json!(2);
    non_minimal_payload["source_roots"] = json!([child, reviewed]);
    non_minimal_payload["after_confirmation"]["source_roots"] = json!([child, reviewed]);
    non_minimal_payload["after_confirmation"]["argv"] = json!([
        "skillroster",
        "--source-root",
        child,
        "--source-root",
        reviewed,
        "scan",
        "--json"
    ]);
    fs::write(
        &non_minimal_json,
        serde_json::to_vec(&non_minimal_payload).unwrap(),
    )
    .unwrap();
    let refused = run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    );
    assert!(!refused.status.success());
    assert!(non_minimal_json.is_file());
    assert!(state.join("skillroster.db").is_file());
    fs::remove_file(&non_minimal_json).unwrap();
    fs::write(&artifact, serde_json::to_vec(&artifact_payload).unwrap()).unwrap();
    let deleted = json_output(&run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(deleted["result"]["removed_source_confirmation_details"], 1);
    assert!(!details.exists());
}

#[test]
fn setup_requires_a_choice_before_replacing_a_modified_bootstrap_skill() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let ordinary = root.join("ordinary");
    let bootstrap = root.join("skillroster/SKILL.md");
    fs::create_dir_all(&ordinary).unwrap();
    fs::write(
        ordinary.join("SKILL.md"),
        "---\nname: ordinary\ndescription: fixture\n---\n",
    )
    .unwrap();
    fs::create_dir_all(bootstrap.parent().unwrap()).unwrap();
    let modified =
        "---\nname: skillroster\ndescription: locally customized\n---\ncustom instructions\n";
    fs::write(&bootstrap, modified).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let blocked = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(blocked["result"]["state"], "modified_choice_required");
    assert_eq!(blocked["result"]["modified_count"], 1);
    assert!(blocked["result"]["plan_id"].is_null());
    assert_eq!(blocked["result"]["targets"][0]["status"], "modified");
    assert!(blocked["result"]["targets"][0]["installed_version"].is_null());
    assert_eq!(blocked["suggested_actions"].as_array().unwrap().len(), 2);
    assert_eq!(
        blocked["suggested_actions"][0]["argv"],
        serde_json::json!([
            "skillroster",
            "setup",
            "--modified-choice",
            "retain-local",
            "--json"
        ])
    );
    assert_eq!(blocked["suggested_actions"][0]["mutates"], false);
    assert_eq!(
        blocked["suggested_actions"][0]["requires_confirmation"],
        false
    );
    assert_eq!(
        blocked["suggested_actions"][1]["argv"],
        serde_json::json!([
            "skillroster",
            "setup",
            "--modified-choice",
            "adopt-current",
            "--json"
        ])
    );
    assert_eq!(fs::read_to_string(&bootstrap).unwrap(), modified);

    let retained = json_output(&run(
        &[&common[..], &["setup", "--modified-choice", "retain-local"]].concat(),
        None,
    ));
    assert_eq!(retained["result"]["state"], "local_modifications_retained");
    assert!(retained["result"]["plan_id"].is_null());
    assert_eq!(fs::read_to_string(&bootstrap).unwrap(), modified);

    let upgrade = json_output(&run(
        &[
            &common[..],
            &["setup", "--modified-choice", "adopt-current"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(upgrade["result"]["state"], "preview_ready");
    assert_eq!(upgrade["result"]["replace_count"], 1);
    assert_eq!(upgrade["result"]["modified_count"], 1);
    assert_eq!(upgrade["suggested_actions"].as_array().unwrap().len(), 1);
    assert_eq!(upgrade["suggested_actions"][0]["action"], "apply");
    assert_eq!(upgrade["suggested_actions"][0]["mutates"], true);
    assert_eq!(
        upgrade["suggested_actions"][0]["requires_confirmation"],
        true
    );
    assert_eq!(fs::read_to_string(&bootstrap).unwrap(), modified);
    let plan_id = upgrade["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    assert_eq!(detail["result"]["operations"][0]["kind"], "replace_file");

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(
        fs::read_to_string(&bootstrap).unwrap(),
        include_str!("../skill/skillroster/SKILL.md").replace("\r\n", "\n")
    );
    let current = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(current["result"]["state"], "up_to_date");
    assert!(current["result"]["plan_id"].is_null());
    assert_eq!(
        current["result"]["targets"][0]["installed_version"],
        "1.8.18"
    );

    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(fs::read_to_string(&bootstrap).unwrap(), modified);
}

#[test]
fn setup_upgrades_an_exact_official_legacy_bootstrap_and_undo_restores_it() {
    for (legacy_version, legacy) in [
        ("1.4.0", include_str!("fixtures/bootstrap-v1.4.0.md")),
        ("1.5.0", include_str!("fixtures/bootstrap-v1.5.0.md")),
        ("1.5.1", include_str!("fixtures/bootstrap-v1.5.1.md")),
        ("1.6.0", include_str!("fixtures/bootstrap-v1.6.0.md")),
        ("1.7.0", include_str!("fixtures/bootstrap-v1.7.0.md")),
        ("1.7.1", include_str!("fixtures/bootstrap-v1.7.1.md")),
        ("1.8.0", include_str!("fixtures/bootstrap-v1.8.0.md")),
        ("1.8.1", include_str!("fixtures/bootstrap-v1.8.1.md")),
        ("1.8.2", include_str!("fixtures/bootstrap-v1.8.2.md")),
        ("1.8.3", include_str!("fixtures/bootstrap-v1.8.3.md")),
        ("1.8.4", include_str!("fixtures/bootstrap-v1.8.4.md")),
    ] {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        let root = home.join(".codex/skills");
        let ordinary = root.join("ordinary");
        let bootstrap = root.join("skillroster/SKILL.md");
        fs::create_dir_all(&ordinary).unwrap();
        fs::write(
            ordinary.join("SKILL.md"),
            "---\nname: ordinary\ndescription: fixture\n---\n",
        )
        .unwrap();
        fs::create_dir_all(bootstrap.parent().unwrap()).unwrap();
        fs::write(&bootstrap, legacy).unwrap();
        let common = [
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
        ];
        json_output(&run(&[&common[..], &["scan"]].concat(), None));

        let upgrade = json_output(&run(&[&common[..], &["setup"]].concat(), None));
        assert_eq!(upgrade["result"]["state"], "preview_ready");
        assert_eq!(upgrade["result"]["outdated_count"], 1);
        assert_eq!(upgrade["result"]["modified_count"], 0);
        assert_eq!(upgrade["result"]["replace_count"], 1);
        assert_eq!(
            upgrade["result"]["targets"][0]["status"],
            "official_outdated"
        );
        assert_eq!(
            upgrade["result"]["targets"][0]["installed_version"],
            legacy_version
        );
        assert_eq!(fs::read_to_string(&bootstrap).unwrap(), legacy);

        let plan_id = upgrade["result"]["plan_id"].as_str().unwrap();
        let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
        assert_eq!(applied["result"]["verification"], "passed");
        assert_eq!(
            fs::read_to_string(&bootstrap).unwrap(),
            include_str!("../skill/skillroster/SKILL.md").replace("\r\n", "\n")
        );
        let current = json_output(&run(&[&common[..], &["setup"]].concat(), None));
        assert_eq!(current["result"]["state"], "up_to_date");

        let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
        let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
        assert_eq!(undone["result"]["verification"], "passed");
        assert_eq!(fs::read_to_string(&bootstrap).unwrap(), legacy);
    }
}

#[cfg(unix)]
#[test]
fn setup_deduplicates_shared_agent_roots_and_undo_restores_each_physical_root() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let shared = home.join(".agents_skills");
    let opencode = home.join(".config/opencode/skills");
    let hermes = home.join(".hermes/skills");
    for root in [&shared, &opencode, &hermes] {
        fs::create_dir_all(root).unwrap();
    }
    for (parent, link) in [
        (home.join(".codex"), home.join(".codex/skills")),
        (home.join(".claude"), home.join(".claude/skills")),
        (home.join(".pi/agent"), home.join(".pi/agent/skills")),
    ] {
        fs::create_dir_all(parent).unwrap();
        symlink(&shared, link).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(setup["result"]["state"], "preview_ready");
    assert_eq!(
        setup["result"]["detected_agents"].as_array().unwrap().len(),
        5
    );
    assert_eq!(setup["result"]["targets"].as_array().unwrap().len(), 5);
    assert_eq!(setup["result"]["missing_count"], 5);
    assert_eq!(setup["result"]["physical_target_count"], 3);
    assert_eq!(setup["result"]["operation_count"], 6);

    let plan_id = setup["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    let operations = detail["result"]["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 6);
    let unique_targets = operations
        .iter()
        .map(|operation| operation["target"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique_targets.len(), 6);

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    for root in [&shared, &opencode, &hermes] {
        assert!(root.join("skillroster/SKILL.md").is_file());
    }
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    for root in [&shared, &opencode, &hermes] {
        assert!(!root.join("skillroster").exists());
    }
}

#[test]
fn repeated_setup_reuses_the_same_ready_plan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/skills")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let first = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let retry = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(first["result"]["state"], "preview_ready");
    assert_eq!(retry["result"]["state"], "preview_ready");
    assert_eq!(retry["result"]["plan_id"], first["result"]["plan_id"]);
    assert_eq!(first["result"]["files_changed"], false);
    assert_eq!(retry["result"]["files_changed"], false);

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 1);
    assert_eq!(
        status["result"]["pending_plans"][0]["plan_id"],
        first["result"]["plan_id"]
    );

    let first_plan_id = first["result"]["plan_id"].as_str().unwrap();
    let applied = json_output(&run(
        &[&common[..], &["apply", first_plan_id]].concat(),
        None,
    ));
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");

    let after_terminal_state = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(after_terminal_state["result"]["state"], "preview_ready");
    assert_ne!(
        after_terminal_state["result"]["plan_id"],
        first["result"]["plan_id"]
    );
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 1);
    assert_eq!(
        status["result"]["pending_plans"][0]["plan_id"],
        after_terminal_state["result"]["plan_id"]
    );

    let explicit_choice = json_output(&run(
        &[&common[..], &["setup", "--modified-choice", "retain-local"]].concat(),
        None,
    ));
    assert_ne!(
        explicit_choice["result"]["plan_id"],
        after_terminal_state["result"]["plan_id"]
    );
    let explicit_retry = json_output(&run(
        &[&common[..], &["setup", "--modified-choice", "retain-local"]].concat(),
        None,
    ));
    assert_eq!(
        explicit_retry["result"]["plan_id"],
        explicit_choice["result"]["plan_id"]
    );
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 2);

    let default_after_explicit = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(
        default_after_explicit["result"]["plan_id"],
        after_terminal_state["result"]["plan_id"]
    );
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 2);

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let after_new_snapshot = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_ne!(
        after_new_snapshot["result"]["plan_id"],
        default_after_explicit["result"]["plan_id"]
    );
}

#[test]
fn setup_without_a_snapshot_returns_a_typed_scan_action() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let output = json_output(&run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "setup",
        ],
        None,
    ));

    assert_eq!(output["result"]["state"], "scan_required");
    assert_eq!(output["result"]["bootstrap_version"], "1.8.18");
    assert_eq!(output["suggested_actions"].as_array().unwrap().len(), 1);
    assert_eq!(
        output["suggested_actions"][0]["argv"],
        serde_json::json!(["skillroster", "scan", "--json"])
    );
}

#[test]
fn setup_preserves_unsupported_bootstrap_targets_without_creating_a_plan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let ordinary = root.join("ordinary");
    fs::create_dir_all(&ordinary).unwrap();
    fs::write(
        ordinary.join("SKILL.md"),
        "---\nname: ordinary\ndescription: fixture\n---\n",
    )
    .unwrap();
    let blocked_parent = root.join("skillroster");
    fs::write(&blocked_parent, "user-owned non-directory target\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    for setup_args in [
        vec!["setup"],
        vec!["setup", "--modified-choice", "adopt-current"],
    ] {
        let output = json_output(&run(&[&common[..], &setup_args].concat(), None));
        assert_eq!(output["result"]["state"], "unsupported_targets");
        assert_eq!(output["result"]["unsupported_count"], 1);
        assert!(output["result"]["plan_id"].is_null());
        assert_eq!(output["result"]["targets"][0]["status"], "unsupported");
        assert_eq!(
            fs::read_to_string(&blocked_parent).unwrap(),
            "user-owned non-directory target\n"
        );
    }
}

#[test]
fn agent_plan_refuses_arbitrary_skill_root_write_file() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let skill = root.join("example");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: example\n---\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let input = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "operations": [{
            "kind": "write_file",
            "target": root.join("arbitrary.txt"),
            "content": "not governance\n",
            "expected_fingerprint": "missing"
        }]
    })
    .to_string();
    let rejected = run(&[&common[..], &["plan", "--stdin"]].concat(), Some(&input));
    assert!(!rejected.status.success());
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected["ok"], false);
    assert!(!root.join("arbitrary.txt").exists());

    let missing_evidence = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "roster_changes": [{
            "agent": "codex",
            "skill_id": "skill_missing",
            "state": "core"
        }]
    })
    .to_string();
    let missing_evidence = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&missing_evidence),
    );
    assert!(!missing_evidence.status.success());
    let missing_evidence: Value = serde_json::from_slice(&missing_evidence.stdout).unwrap();
    assert!(
        missing_evidence["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing_evidence")
    );

    let missing_skill = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "roster_changes": [{
            "agent": "codex",
            "skill_id": "skill_missing",
            "state": "core"
        }]
    })
    .to_string();
    let rejected_skill = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&missing_skill),
    );
    assert!(!rejected_skill.status.success());

    let stale_snapshot = json!({
        "schema_version": 1,
        "scan_id": "scan_missing",
        "roster_changes": [{
            "agent": "codex",
            "skill_id": scan["result"]["snapshot_id"],
            "state": "core"
        }]
    })
    .to_string();
    let rejected_snapshot = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&stale_snapshot),
    );
    assert!(!rejected_snapshot.status.success());

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let plan_count: i64 = database
        .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get(0))
        .unwrap();
    assert_eq!(plan_count, 0, "an invalid Plan must never become Ready");
}

#[test]
fn json_failure_is_one_parseable_document() {
    let temp = TempDir::new().unwrap();
    let output = run(
        &[
            "--home",
            temp.path().to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "--json",
            "report",
        ],
        None,
    );
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "report");
    assert_eq!(value["error"]["code"], "snapshot_required");
    assert_eq!(value["error"]["retryable"], false);
    assert!(output.stderr.is_empty());

    let invalid = run(&["--json", "not-a-command"], None);
    assert_eq!(invalid.status.code(), Some(2));
    let invalid_value: Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert_eq!(invalid_value["command"], "cli");
    assert_eq!(invalid_value["error"]["code"], "invalid_cli_arguments");
    assert!(invalid.stderr.is_empty());
}

#[test]
fn explicit_roots_preserve_all_eight_agent_identities() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("empty-home");
    let state = temp.path().join("state");
    let agents = [
        "codex",
        "claude-code",
        "pi",
        "opencode",
        "hermes",
        "cursor",
        "gemini-cli",
        "github-copilot",
    ];
    let shared_root = temp.path().join("shared-skills");
    let shared_skill = shared_root.join("shared-fixture");
    fs::create_dir_all(&shared_skill).unwrap();
    fs::write(
        shared_skill.join("SKILL.md"),
        "---\nname: shared-fixture\n---\nshared fixture\n",
    )
    .unwrap();
    let mut args = vec![
        "--home".to_owned(),
        home.display().to_string(),
        "--state-dir".to_owned(),
        state.display().to_string(),
        "--json".to_owned(),
    ];
    for agent in agents {
        args.push("--root".to_owned());
        args.push(format!("{agent}={}", shared_root.display()));
    }
    args.push("scan".to_owned());
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let scan = json_output(&run(&refs, None));
    let explicit = scan["result"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|root| root["explicit"] == true)
        .collect::<Vec<_>>();
    assert_eq!(explicit.len(), 8);
    for agent in agents {
        assert!(explicit.iter().any(|root| root["agent"] == agent));
    }
    assert!(explicit.iter().all(|root| root["status"] == "included"));
    let coverage_agents = scan["result"]["coverage"]
        .as_array()
        .unwrap()
        .iter()
        .map(|coverage| coverage["agent"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(coverage_agents, agents);
    assert_eq!(scan["result"]["placement_count"], 8);
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let assigned_agents: i64 = database
        .query_row(
            "SELECT COUNT(DISTINCT p.agent_id)
             FROM placements p JOIN roots r ON r.id = p.root_id
             WHERE r.explicit = 1 AND p.exposed = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(assigned_agents, 8);
}

#[test]
fn lifecycle_exclusion_and_database_delete_preserve_user_files() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/example");
    let session = home.join(".codex/sessions/session.jsonl");
    let library = state.join("library/example");
    fs::create_dir_all(&skill).unwrap();
    fs::create_dir_all(session.parent().unwrap()).unwrap();
    fs::create_dir_all(&library).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: example\n---\nbody\n").unwrap();
    fs::write(
        &session,
        "{\"type\":\"invoke_skill\",\"invoked_skill\":\"example\",\"prompt\":\"PRIVATE-CONVERSATION-TEXT\"}\n",
    )
    .unwrap();
    fs::write(library.join("SKILL.md"), "library sentinel\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let excluded = json_output(&run(
        &[&common[..], &["lifecycle", "exclude", "codex"]].concat(),
        None,
    ));
    assert_eq!(excluded["result"]["raw_conversations_copied"], false);
    let rescanned = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert!(
        rescanned["result"]["roots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|root| root["agent"] == "codex"
                && root["kind"] == "sessions"
                && root["status"] == "excluded")
    );
    let inspect = json_output(&run(
        &[&common[..], &["lifecycle", "inspect"]].concat(),
        None,
    ));
    assert_eq!(inspect["result"]["evidence_exclusions"], json!(["codex"]));
    let export_path = temp.path().join("lifecycle-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                export_path.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    assert!(
        !fs::read_to_string(&export_path)
            .unwrap()
            .contains("PRIVATE-CONVERSATION-TEXT")
    );

    let wrong_confirmation = run(
        &[&common[..], &["lifecycle", "delete", "--confirm", "wrong"]].concat(),
        None,
    );
    assert!(!wrong_confirmation.status.success());
    assert!(state.join("skillroster.db").is_file());
    let deleted = json_output(&run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(deleted["result"]["agent_files_changed"], false);
    assert_eq!(deleted["result"]["library_files_changed"], false);
    assert!(!state.join("skillroster.db").exists());
    assert!(skill.join("SKILL.md").is_file());
    assert!(library.join("SKILL.md").is_file());

    let rebuilt = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(rebuilt["result"]["skill_count"], 1);
    assert!(state.join("skillroster.db").is_file());
}

#[test]
fn unchanged_rescans_do_not_multiply_exported_usage_observations() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/example");
    let session = home.join(".codex/sessions/session.jsonl");
    fs::create_dir_all(&skill).unwrap();
    fs::create_dir_all(session.parent().unwrap()).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: example\n---\nbody\n").unwrap();
    fs::write(
        &session,
        "{\"type\":\"invoke_skill\",\"invoked_skill\":\"example\"}\n",
    )
    .unwrap();
    fs::File::options()
        .write(true)
        .open(&session)
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(10)))
        .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let first_export = temp.path().join("first-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                first_export.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let first: Value = serde_json::from_slice(&fs::read(&first_export).unwrap()).unwrap();
    assert_eq!(first["data"]["usage_events"].as_array().unwrap().len(), 1);

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let second_export = temp.path().join("second-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                second_export.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let second: Value = serde_json::from_slice(&fs::read(&second_export).unwrap()).unwrap();
    assert_eq!(second["data"]["usage_events"].as_array().unwrap().len(), 1);

    let mut session_file = OpenOptions::new().append(true).open(&session).unwrap();
    writeln!(
        session_file,
        "{{\"type\":\"invoke_skill\",\"invoked_skill\":\"example\"}}"
    )
    .unwrap();
    session_file
        .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(20)))
        .unwrap();
    drop(session_file);
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let third_export = temp.path().join("third-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                third_export.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let third: Value = serde_json::from_slice(&fs::read(&third_export).unwrap()).unwrap();
    assert_eq!(third["data"]["usage_events"].as_array().unwrap().len(), 2);

    json_output(&run(
        &[&common[..], &["lifecycle", "purge", "--raw-days", "1"]].concat(),
        None,
    ));
    let retained_export = temp.path().join("retained-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                retained_export.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let retained: Value = serde_json::from_slice(&fs::read(&retained_export).unwrap()).unwrap();
    assert_eq!(retained["usage_history"]["raw_value_field"], "event_delta");
    assert_eq!(
        retained["usage_history"]["snapshot_evidence_additive"],
        false
    );
    assert_eq!(retained["data"]["usage_events"], json!([]));
    assert!(
        retained["data"]["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .all(|evidence| evidence["kind"] != "usage")
    );
    assert_eq!(
        retained["data"]["usage_monthly"].as_array().unwrap().len(),
        1
    );
    assert_eq!(retained["data"]["usage_monthly"][0]["event_count"], 2);
    assert_eq!(
        retained["data"]["usage_monthly"][0]["derivation"],
        "source_delta"
    );
}

#[test]
fn source_update_requires_conflict_choice_and_round_trips_adoption() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_dir = home.join(".codex/skills/source-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    let entrypoint = skill_dir.join("SKILL.md");
    let original =
        "---\nname: source-skill\nsource: github:example/source-skill\nversion: v1\n---\nold\n";
    fs::write(&entrypoint, original).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = payload["skills"][0]["id"].as_str().unwrap();
    let placement_id = payload["placements"][0]["id"].as_str().unwrap();
    let current_fingerprint = payload["placements"][0]["content_digest"].as_str().unwrap();
    let evidence_id: String = database
        .query_row(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let upstream =
        "---\nname: source-skill\nsource: github:example/source-skill\nversion: v2\n---\nnew\n";
    let digest = |content: &str| format!("{:x}", Sha256::digest(content.as_bytes()));
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "source_updates": [{
            "skill_id": skill_id,
            "placement_id": placement_id,
            "source": "github:example/source-skill",
            "current_revision": "v1",
            "current_fingerprint": current_fingerprint,
            "base_digest": digest(original),
            "upstream_revision": "v2",
            "upstream_content": upstream,
            "upstream_digest": digest(upstream)
        }]
    });
    let untrusted_default = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!untrusted_default.status.success());
    let untrusted_error: Value = serde_json::from_slice(&untrusted_default.stdout).unwrap();
    assert!(
        untrusted_error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("first-observed and untrusted")
    );
    let mut explicit_adopt = request.clone();
    explicit_adopt["source_updates"][0]["choice"] = json!("adopt_upstream");
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&explicit_adopt.to_string()),
    ));
    assert_eq!(plan["result"]["risk"], "source_update");
    assert_eq!(
        plan["result"]["diff_summary"]["items"][0]["choice_reason"],
        "first_observed_baseline_untrusted"
    );
    assert_eq!(
        plan["result"]["diff_summary"]["items"][0]["choice"],
        "adopt_upstream"
    );
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(fs::read_to_string(&entrypoint).unwrap(), upstream);
    let receipt = applied["result"]["receipt_id"].as_str().unwrap();

    let trusted_digest: String = database
        .query_row(
            "SELECT trusted_digest FROM source_baselines
             WHERE source = 'github:example/source-skill' AND revision = 'v2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(trusted_digest, digest(upstream));
    let trusted_scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let trusted_snapshot = trusted_scan["result"]["snapshot_id"].as_str().unwrap();
    let trusted_payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [trusted_snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let trusted_payload: Value = serde_json::from_str(&trusted_payload).unwrap();
    let upstream_v3 =
        "---\nname: source-skill\nsource: github:example/source-skill\nversion: v3\n---\nnewer\n";
    let trusted_request = json!({
        "schema_version": 1,
        "scan_id": trusted_snapshot,
        "evidence_ids": [database.query_row::<String, _, _>(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [trusted_snapshot],
            |row| row.get(0),
        ).unwrap()],
        "source_updates": [{
            "skill_id": trusted_payload["skills"][0]["id"],
            "placement_id": trusted_payload["placements"][0]["id"],
            "source": "github:example/source-skill",
            "current_revision": "v2",
            "current_fingerprint": trusted_payload["placements"][0]["content_digest"],
            "upstream_revision": "v3",
            "upstream_content": upstream_v3,
            "upstream_digest": digest(upstream_v3)
        }]
    });
    let trusted_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&trusted_request.to_string()),
    ));
    assert_eq!(
        trusted_plan["result"]["diff_summary"]["items"][0]["choice_reason"],
        "trusted_baseline_clean"
    );
    let undone = json_output(&run(&[&common[..], &["undo", receipt]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(fs::read_to_string(&entrypoint).unwrap(), original);

    fs::write(&entrypoint, original.replace("old", "local edit")).unwrap();
    let rescanned = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let new_snapshot = rescanned["result"]["snapshot_id"].as_str().unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [new_snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let modified_request = json!({
        "schema_version": 1,
        "scan_id": new_snapshot,
        "evidence_ids": [database.query_row::<String, _, _>(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [new_snapshot],
            |row| row.get(0),
        ).unwrap()],
        "source_updates": [{
            "skill_id": payload["skills"][0]["id"],
            "placement_id": payload["placements"][0]["id"],
            "source": "github:example/source-skill",
            "current_revision": "v1",
            "current_fingerprint": payload["placements"][0]["content_digest"],
            "base_digest": digest(original),
            "upstream_revision": "v2",
            "upstream_content": upstream,
            "upstream_digest": digest(upstream)
        }]
    });
    let conflict = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&modified_request.to_string()),
    );
    assert!(!conflict.status.success());

    let mut forged_baseline_request = modified_request.clone();
    forged_baseline_request["source_updates"][0]["base_digest"] =
        json!(digest(&original.replace("old", "local edit")));
    let forged_baseline = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&forged_baseline_request.to_string()),
    );
    assert!(
        !forged_baseline.status.success(),
        "a caller-supplied hash of locally edited content must not replace the stored baseline"
    );
    let forged_error: Value = serde_json::from_slice(&forged_baseline.stdout).unwrap();
    assert!(
        forged_error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("immutable source baseline"),
        "unexpected forged-baseline rejection: {forged_error}"
    );

    let mut stale_evidence_request = modified_request.clone();
    stale_evidence_request["evidence_ids"] = json!([evidence_id]);
    stale_evidence_request["source_updates"][0]["choice"] = json!("retain_local");
    let stale_evidence = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&stale_evidence_request.to_string()),
    );
    assert!(!stale_evidence.status.success());

    let mut preserve_request = modified_request.clone();
    preserve_request["source_updates"][0]["choice"] = json!("preserve_both");
    let preserve_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&preserve_request.to_string()),
    ));
    let preserved = json_output(&run(
        &[
            &common[..],
            &[
                "apply",
                preserve_plan["result"]["plan_id"].as_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let sibling = skill_dir.join("SKILL.upstream-v2.md");
    assert_eq!(fs::read_to_string(&sibling).unwrap(), upstream);
    assert!(
        fs::read_to_string(&entrypoint)
            .unwrap()
            .contains("local edit")
    );
    let preserve_undo = json_output(&run(
        &[
            &common[..],
            &["undo", preserved["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(preserve_undo["result"]["verification"], "passed");
    assert!(!sibling.exists());

    let mut retain_request = modified_request;
    retain_request["source_updates"][0]["choice"] = json!("retain_local");
    let retain_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&retain_request.to_string()),
    ));
    let retained = json_output(&run(
        &[
            &common[..],
            &["apply", retain_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(retained["result"]["changed_path_count"], 0);
    assert_eq!(retained["result"]["files_changed"], false);
    let retained_undo = json_output(&run(
        &[
            &common[..],
            &["undo", retained["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(retained_undo["result"]["verification"], "passed");

    database
        .execute("DELETE FROM source_baselines", [])
        .unwrap();
    let mut missing_baseline_request = retain_request.clone();
    missing_baseline_request["source_updates"][0]
        .as_object_mut()
        .unwrap()
        .remove("base_digest");
    missing_baseline_request["source_updates"][0]
        .as_object_mut()
        .unwrap()
        .remove("choice");
    let missing_baseline = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&missing_baseline_request.to_string()),
    );
    assert!(
        !missing_baseline.status.success(),
        "a missing baseline must never silently adopt upstream content"
    );
    let missing_error: Value = serde_json::from_slice(&missing_baseline.stdout).unwrap();
    assert!(
        missing_error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("explicit choice is required")
    );
    missing_baseline_request["source_updates"][0]["choice"] = json!("retain_local");
    let explicit_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&missing_baseline_request.to_string()),
    ));
    assert_eq!(explicit_plan["result"]["risk"], "source_update");
}

#[test]
fn library_governance_managed_and_hosted_round_trip_and_refuse_drift() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_skill = home.join(".codex/skills/shared");
    let claude_skill = home.join(".claude/skills/shared");
    let content = "---\nname: shared\ndescription: shared fixture\n---\nbody\n";
    for directory in [&codex_skill, &claude_skill] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = payload["skills"][0]["id"].as_str().unwrap();
    let placements = payload["placements"].as_array().unwrap();
    assert_eq!(placements.len(), 2);
    let canonical = placements
        .iter()
        .find(|placement| placement["agent"] == "codex")
        .unwrap();
    let placement_ids = placements
        .iter()
        .map(|placement| placement["id"].clone())
        .collect::<Vec<_>>();
    let evidence_id: String = database
        .query_row(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let request = |state_name: &str| {
        json!({
            "schema_version": 1,
            "scan_id": snapshot,
            "evidence_ids": [evidence_id],
            "library_changes": [{
                "skill_id": skill_id,
                "canonical_placement_id": canonical["id"],
                "placement_ids": placement_ids,
                "requested_state": state_name
            }]
        })
    };

    let managed_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request("managed").to_string()),
    ));
    assert_eq!(managed_plan["result"]["risk"], "library_governance");
    let managed = json_output(&run(
        &[
            &common[..],
            &["apply", managed_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert!(
        fs::symlink_metadata(&claude_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&claude_skill).unwrap(),
        fs::canonicalize(&codex_skill).unwrap()
    );
    let governance_state: String = database
        .query_row(
            "SELECT governance_state FROM skills WHERE id = ?1",
            [skill_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(governance_state, "managed");
    let managed_undo = json_output(&run(
        &[
            &common[..],
            &["undo", managed["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(
        managed_undo["result"]["verification"], "passed",
        "{managed_undo}"
    );
    assert!(claude_skill.is_dir());
    assert!(
        !fs::symlink_metadata(&claude_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let governance_state: String = database
        .query_row(
            "SELECT governance_state FROM skills WHERE id = ?1",
            [skill_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(governance_state, "observed");

    let hosted_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request("hosted").to_string()),
    ));
    let hosted = json_output(&run(
        &[
            &common[..],
            &["apply", hosted_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    let library_skill = state.join("library/shared");
    assert!(library_skill.join("SKILL.md").is_file());
    assert_eq!(
        fs::canonicalize(&codex_skill).unwrap(),
        fs::canonicalize(&library_skill).unwrap()
    );
    assert_eq!(
        fs::canonicalize(&claude_skill).unwrap(),
        fs::canonicalize(&library_skill).unwrap()
    );
    let governance_state: String = database
        .query_row(
            "SELECT governance_state FROM skills WHERE id = ?1",
            [skill_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(governance_state, "hosted");
    let hosted_find = json_output(&run(&[&common[..], &["find", "shared"]].concat(), None));
    assert_eq!(
        hosted_find["result"]["matches"][0]["roster_state"],
        "unassigned"
    );
    assert_find_paths_are_readable(&hosted_find);
    let expected_library_entry = fs::canonicalize(library_skill.join("SKILL.md")).unwrap();
    assert!(
        hosted_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| fs::canonicalize(path.as_str().unwrap())
                .is_ok_and(|actual| actual == expected_library_entry))
    );
    json_output(&run(
        &[
            &common[..],
            &["undo", hosted["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert!(!state.join("library").exists());
    assert!(!state.join("plan-backups").exists());
    assert!(codex_skill.join("SKILL.md").is_file());
    assert!(claude_skill.join("SKILL.md").is_file());
    let restored_find = json_output(&run(&[&common[..], &["find", "shared"]].concat(), None));
    assert_find_paths_are_readable(&restored_find);
    assert!(
        restored_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| !path.as_str().unwrap().contains("/library/"))
    );

    let drift_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request("managed").to_string()),
    ));
    fs::write(claude_skill.join("SKILL.md"), "user drift\n").unwrap();
    let drifted = run(
        &[
            &common[..],
            &["apply", drift_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    );
    assert!(!drifted.status.success());
    assert!(
        !fs::symlink_metadata(&claude_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
#[cfg(unix)]
fn exact_duplicate_plan_deduplicates_shared_agent_roots_and_undo_restores_sources() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let shared_root = home.join(".agents_skills");
    let shared_skill = shared_root.join("shared");
    let opencode_skill = home.join(".config/opencode/skills/shared");
    let hermes_skill = home.join(".hermes/skills/shared");
    let content = "---\nname: shared\ndescription: shared fixture\n---\nbody\n";
    for directory in [&shared_skill, &opencode_skill, &hermes_skill] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    for (parent, logical_root) in [
        (home.join(".codex"), home.join(".codex/skills")),
        (home.join(".claude"), home.join(".claude/skills")),
        (home.join(".pi/agent"), home.join(".pi/agent/skills")),
    ] {
        fs::create_dir_all(parent).unwrap();
        symlink(&shared_root, logical_root).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Exact duplicate Skill placements")
        .unwrap();
    assert!(
        finding["summary"]
            .as_str()
            .unwrap()
            .contains("6 placements across 3 distinct physical sources")
    );
    let finding_id = finding["id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["canonical_candidate_count"], 3);
    assert_eq!(planning["canonical_candidates_truncated"], false);
    assert_eq!(
        planning["canonical_candidates"][0]["path"],
        shared_skill.join("SKILL.md").to_str().unwrap()
    );
    let canonical_placement_id = planning["canonical_candidates"][0]["placement_id"]
        .as_str()
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": finding_id,
            "canonical_placement_id": canonical_placement_id,
            "requested_state": "managed"
        }]
    });

    let codex_root = home.join(".codex/skills");
    let drifted_root = home.join("drifted-shared-root");
    fs::create_dir_all(drifted_root.join("shared")).unwrap();
    fs::write(drifted_root.join("shared/SKILL.md"), content).unwrap();
    fs::remove_file(&codex_root).unwrap();
    symlink(&drifted_root, &codex_root).unwrap();
    let drifted_output = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!drifted_output.status.success());
    let drifted: Value = serde_json::from_slice(&drifted_output.stdout).unwrap();
    assert_eq!(drifted["ok"], false);
    assert_eq!(drifted["error"]["code"], "state_drift");
    fs::remove_file(&codex_root).unwrap();
    symlink(&shared_root, &codex_root).unwrap();

    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert_eq!(plan["result"]["change_summary"]["operation_count"], 5);
    assert_eq!(plan["result"]["affected"]["placement_count"], 6);
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let full = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    let targets = full["result"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| operation["target"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(targets.len(), 5);

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    for directory in [&opencode_skill, &hermes_skill] {
        assert!(
            fs::symlink_metadata(directory)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::canonicalize(directory).unwrap(),
            fs::canonicalize(&shared_skill).unwrap()
        );
    }
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    for directory in [&opencode_skill, &hermes_skill] {
        assert!(directory.join("SKILL.md").is_file());
        assert!(
            !fs::symlink_metadata(directory)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(directory.join("SKILL.md")).unwrap(),
            content
        );
    }
    assert_eq!(
        fs::read_to_string(shared_skill.join("SKILL.md")).unwrap(),
        content
    );
}

#[test]
#[cfg(unix)]
fn exact_duplicate_plan_moves_the_real_source_instead_of_its_alias() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_skill = home.join(".codex/skills/shared");
    let opencode_real = home.join(".config/opencode/skills/z-real");
    let opencode_alias = home.join(".config/opencode/skills/a-alias");
    let hermes_skill = home.join(".hermes/skills/shared");
    let content = "---\nname: shared\ndescription: shared fixture\n---\nbody\n";
    for directory in [&codex_skill, &opencode_real, &hermes_skill] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    symlink(&opencode_real, &opencode_alias).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Exact duplicate Skill placements")
        .unwrap();
    let finding_id = finding["id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["canonical_candidate_count"], 3);
    assert!(
        planning["canonical_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| candidate["path"] != opencode_alias.join("SKILL.md").to_str().unwrap())
    );
    let canonical_placement_id = planning["canonical_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["path"] == codex_skill.join("SKILL.md").to_str().unwrap())
        .unwrap()["placement_id"]
        .as_str()
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": finding_id,
            "canonical_placement_id": canonical_placement_id,
            "requested_state": "managed"
        }]
    });

    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert_eq!(plan["result"]["change_summary"]["operation_count"], 5);
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let full = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    let moved_sources = full["result"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|operation| operation["kind"] == "move_recoverable")
        .map(|operation| operation["source"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        moved_sources
            .iter()
            .any(|source| Path::new(source).file_name().unwrap() == "z-real")
    );
    assert!(
        moved_sources
            .iter()
            .all(|source| Path::new(source).file_name().unwrap() != "a-alias")
    );

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert!(
        fs::symlink_metadata(&opencode_real)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&opencode_alias).unwrap(),
        fs::canonicalize(&codex_skill).unwrap()
    );
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    assert!(opencode_real.join("SKILL.md").is_file());
    assert_eq!(
        fs::read_to_string(opencode_alias.join("SKILL.md")).unwrap(),
        content
    );
}

#[test]
fn exact_duplicate_finding_prepares_library_plan_from_semantic_choices() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let source_root = temp.path().join("sources");
    let source_skill = source_root.join("shared");
    let codex_skill = home.join(".codex/skills/shared");
    let claude_skill = home.join(".claude/skills/shared");
    let content = "---\nname: shared\ndescription: shared fixture\n---\nbody\n";
    for directory in [&source_skill, &codex_skill, &claude_skill] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    for index in 0..48 {
        let directory = home.join(format!(".codex/skills/shared-copy-{index:02}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--source-root",
        source_root.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Exact duplicate Skill placements")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], true);
    assert_eq!(planning["snapshot_id"], report["result"]["snapshot_id"]);
    assert_eq!(planning["request_field"], "finding_library_changes");
    let candidates = planning["canonical_candidates"].as_array().unwrap();
    assert_eq!(planning["canonical_candidate_count"], 51);
    assert_eq!(planning["canonical_candidates_truncated"], true);
    assert_eq!(
        candidates[0]["path"],
        fs::canonicalize(source_skill.join("SKILL.md"))
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(candidates[0]["reason"], "non_exposed_source");
    let canonical_placement_id = candidates[0]["placement_id"].as_str().unwrap();

    let request = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": finding_id,
            "canonical_placement_id": canonical_placement_id,
            "requested_state": "managed"
        }]
    });
    let plan_output = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(
        plan_output.stdout.len() < 8_000,
        "summary response was {} bytes",
        plan_output.stdout.len()
    );
    let plan = json_output(&plan_output);
    assert_eq!(plan["result"]["risk"], "library_governance");
    assert_eq!(plan["result"]["findings"]["ids"], json!([finding_id]));
    assert_eq!(plan["result"]["detail_level"], "summary");
    assert!(plan["result"].get("operations").is_none());
    assert!(plan["result"].get("library_changes").is_none());
    assert_eq!(plan["result"]["change_summary"]["operation_count"], 101);
    assert_eq!(plan["result"]["affected"]["placement_count"], 51);
    assert_eq!(plan["result"]["affected"]["agent_count"], 2);
    assert_eq!(
        plan["result"]["affected"]["agents"],
        json!(["claude-code", "codex"])
    );
    assert_eq!(plan["result"]["evidence"]["count"], 1);
    assert_eq!(plan["result"]["files_changed"], false);
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    assert_eq!(
        plan["result"]["detail"]["command"],
        json!(["plan", "--show", plan_id, "--json"])
    );
    assert!(
        !plan["result"]["detail"]["contains"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_core_selections"))
    );
    let full_output = run(&[&common[..], &["plan", "--show", plan_id]].concat(), None);
    assert!(full_output.stdout.len() > plan_output.stdout.len());
    let full = json_output(&full_output);
    assert_eq!(full["result"]["detail_level"], "full");
    assert_eq!(full["result"]["operations"].as_array().unwrap().len(), 101);
    assert_eq!(
        full["result"]["library_changes"][0]["placement_ids"]
            .as_array()
            .unwrap()
            .len(),
        51
    );
    assert_eq!(full["result"]["finding_ids"], json!([finding_id]));
    assert_eq!(full["result"]["files_changed"], false);
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let stored_report_id: String = database
        .query_row(
            "SELECT report_id FROM plans WHERE id = ?1",
            [plan_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_report_id, report["result"]["report_id"]);
    assert!(
        !fs::symlink_metadata(&codex_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        !fs::symlink_metadata(&claude_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    let invalid_canonical = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": finding_id,
            "canonical_placement_id": "placement_not_in_finding",
            "requested_state": "managed"
        }]
    });
    let rejected = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&invalid_canonical.to_string()),
    );
    assert!(!rejected.status.success());
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("canonical placement is not part of Finding")
    );

    let non_overlap_finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["category"] != "overlap")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let wrong_finding_type = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": non_overlap_finding,
            "canonical_placement_id": canonical_placement_id,
            "requested_state": "managed"
        }]
    });
    let rejected = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&wrong_finding_type.to_string()),
    );
    assert!(!rejected.status.success());
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is not an exact-duplicate Finding")
    );

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let stale = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!stale.status.success());
    let stale: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert!(
        stale["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not belong to the latest Snapshot")
    );
    let stale_detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(stale_detail["result"]["planning"]["supported"], false);
    assert_eq!(
        stale_detail["result"]["planning"]["reason"],
        "stale_finding"
    );
    assert!(
        stale_detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );
}

#[test]
#[cfg(unix)]
fn large_roster_finding_blocks_partial_plan_until_source_is_confirmed() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let outside = temp.path().join("unconfirmed-source");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(
        outside.join("SKILL.md"),
        "---\nname: external\n---\nfixture\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, root.join("zzz-external")).unwrap();
    for index in 0..51 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(detail["result"]["kind"], "large_default_roster");
    assert_eq!(detail["result"]["files_changed"], false);
    assert_eq!(detail["result"]["planning"]["supported"], false);
    assert_eq!(
        detail["result"]["planning"]["reason"],
        "trusted_canonical_sources_required"
    );
    assert_eq!(detail["result"]["planning"]["blocked_change_count"], 1);
    assert_eq!(
        detail["result"]["planning"]["observed_link_targets"],
        json!([outside])
    );
    assert!(
        detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );

    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let blocked = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert!(
        blocked["error"]["message"]
            .as_str()
            .unwrap()
            .contains("confirm the reported source roots")
    );
}

#[test]
#[cfg(unix)]
fn custom_budget_plan_reports_actionable_source_blockers() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let reviewed = temp.path().join("reviewed");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&reviewed).unwrap();
    for index in 0..10 {
        let directory = root.join(format!("aaa-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: aaa-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    for name in ["mid-a", "mid-b"] {
        let canonical = reviewed.join(name);
        fs::create_dir(&canonical).unwrap();
        fs::write(
            canonical.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture\n"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&canonical, root.join(name)).unwrap();
    }
    for index in 0..49 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", &finding_id]].concat(),
        None,
    ));
    assert_eq!(detail["result"]["planning"]["supported"], true);

    let default_request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let default_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&default_request.to_string()),
    ));
    assert_eq!(default_plan["ok"], true);
    assert_eq!(default_plan["result"]["files_changed"], false);

    let custom_request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 10,
            "protected_skill_ids": []
        }]
    });
    let blocked = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&custom_request.to_string()),
    );
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(blocked["ok"], false);
    assert_eq!(
        blocked["error"]["code"],
        "trusted_canonical_sources_required"
    );
    assert_eq!(blocked["error"]["details"]["requested_core_budget"], 10);
    assert_eq!(blocked["error"]["details"]["blocked_change_count"], 2);
    assert_eq!(
        blocked["error"]["details"]["blocked_changes_truncated"],
        false
    );
    let exact_sources = [reviewed.join("mid-a"), reviewed.join("mid-b")];
    assert_eq!(
        blocked["error"]["details"]["source_roots"],
        json!(exact_sources)
    );
    assert_eq!(blocked["error"]["paths"], json!(exact_sources));
    let names = blocked["error"]["details"]["blocked_changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["mid-a", "mid-b"]);
    assert!(
        blocked["error"]["details"]["blocked_changes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| {
                item["agent"] == "codex"
                    && item["reason"] == "no_owned_exact_content_to_preserve"
                    && item["observed_source_target"]
                        .as_str()
                        .is_some_and(|path| path.starts_with(reviewed.to_str().unwrap()))
            })
    );
    assert_eq!(
        blocked["error"]["details"]["after_confirmation"]["repeatable_option"],
        "--source-root"
    );
    assert_eq!(blocked["error"]["details"]["files_changed"], false);
    assert_eq!(blocked["error"]["details"]["state_files_changed"], false);
    assert_eq!(
        blocked["error"]["details"]["detail_artifact_created"],
        false
    );
    assert_eq!(
        blocked["suggested_actions"][0]["action"],
        "scan_with_confirmed_source_roots"
    );
    assert_eq!(
        blocked["suggested_actions"][0]["requires_confirmation"],
        true
    );
    assert_eq!(blocked["suggested_actions"][0]["mutates"], false);
    let suggested_argv = blocked["suggested_actions"][0]["argv"].as_array().unwrap();
    for exact_source in &exact_sources {
        assert!(suggested_argv.windows(2).any(|pair| {
            pair[0] == "--source-root" && pair[1] == exact_source.to_str().unwrap()
        }));
    }
    assert!(
        !suggested_argv
            .windows(2)
            .any(|pair| { pair[0] == "--source-root" && pair[1] == reviewed.to_str().unwrap() })
    );
    let suggested_roots = suggested_argv
        .windows(2)
        .filter(|pair| pair[0] == "--source-root")
        .map(|pair| {
            pair[1]
                .as_str()
                .expect("source-root argument must be a string")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        suggested_roots,
        exact_sources
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect::<Vec<_>>()
    );
    let retry_source_args = suggested_roots
        .iter()
        .flat_map(|root| ["--source-root", *root])
        .collect::<Vec<_>>();
    assert!(root.join("mid-a").exists());
    assert!(root.join("aaa-000").exists());

    for width in ["60", "80", "120"] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_skillroster"));
        command
            .args([
                "--home",
                home.to_str().unwrap(),
                "--state-dir",
                state.to_str().unwrap(),
                "plan",
                "--stdin",
            ])
            .env("COLUMNS", width)
            .env("NO_COLOR", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(custom_request.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let human = String::from_utf8(output.stderr).unwrap();
        assert!(human.contains("mid-a"), "{human}");
        assert!(human.contains("--source-root"), "{human}");
        assert!(
            human.contains("no automatic change is supported"),
            "{human}"
        );
        let max_width: usize = width.parse().unwrap();
        assert!(
            human.lines().all(|line| line.chars().count() <= max_width),
            "line exceeded {width} columns:\n{human}"
        );
    }

    json_output(&run(
        &[&common[..], &retry_source_args, &["scan"]].concat(),
        None,
    ));
    let report = json_output(&run(
        &[&common[..], &retry_source_args, &["report"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let retry = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 10,
            "protected_skill_ids": []
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &retry_source_args, &["plan", "--stdin"]].concat(),
        Some(&retry.to_string()),
    ));
    assert_eq!(plan["ok"], true);
    assert_eq!(plan["result"]["files_changed"], false);
    assert!(plan["result"]["plan_id"].as_str().is_some());
    assert_eq!(plan["result"]["impact"]["after_default_exposure"], 10);
    assert!(root.join("mid-a").exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 61);
}

#[test]
#[cfg(unix)]
fn large_roster_finding_reports_a_dependent_source_link_before_planning() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let canonical = root.join("zzz-canonical");
    let source_root = temp.path().join("sources");
    fs::create_dir_all(&canonical).unwrap();
    fs::create_dir_all(&source_root).unwrap();
    fs::write(
        canonical.join("SKILL.md"),
        "---\nname: zzz-canonical\n---\nfixture\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&canonical, source_root.join("dependent-source")).unwrap();
    for index in 0..51 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--source-root",
        source_root.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], false);
    assert_eq!(planning["reason"], "source_dependency_blocks_roster_change");
    assert_eq!(planning["decision"], "resolve_source_dependency");
    assert_eq!(planning["blocked_change_count"], 1);
    assert_eq!(
        planning["blocked_changes"][0]["reason"],
        "non_agent_source_link_depends_on_removal"
    );
    assert_eq!(planning["dependent_link_targets"], json!([canonical]));
    assert!(
        detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );

    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let blocked = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert!(
        blocked["error"]["message"]
            .as_str()
            .unwrap()
            .contains("source link depends on a placement scheduled for removal")
    );
}

#[test]
fn large_roster_finding_prepares_and_reverses_a_semantic_layering_plan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    for index in 0..61 {
        let name = if index == 0 {
            "skillroster".to_owned()
        } else {
            format!("skill-{index:03}")
        };
        let directory = root.join(&name);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], true);
    assert_eq!(planning["request_field"], "finding_roster_changes");
    assert_eq!(
        planning["absence_of_usage_evidence"],
        "not_negative_evidence"
    );
    assert_eq!(planning["explicit_only_or_archive_decision_implied"], false);
    assert_eq!(planning["agents"][0]["before_default_exposure"], 61);
    assert_eq!(planning["agents"][0]["proposed_core_count"], 50);
    assert_eq!(planning["agents"][0]["proposed_on_demand_count"], 11);
    assert_eq!(
        planning["selection_evidence"]["stable_fallback_core_count"],
        49
    );
    assert_eq!(
        planning["uncertainty"]["code"],
        "fallback_dominated_core_selection"
    );
    assert_eq!(planning["uncertainty"]["review_required"], true);
    assert!(
        planning["agents"][0]["core_preview"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "skillroster" && item["reason"] == "skillroster_bootstrap")
    );

    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 10,
            "protected_skill_ids": []
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert_eq!(plan["result"]["findings"]["ids"], json!([finding_id]));
    assert_eq!(plan["result"]["change_summary"]["roster_change_count"], 61);
    assert_eq!(plan["result"]["affected"]["placement_count"], 61);
    assert_eq!(plan["result"]["impact"]["before_default_exposure"], 61);
    assert_eq!(plan["result"]["impact"]["after_default_exposure"], 10);
    assert_eq!(plan["result"]["canonical_deletion_count"], 0);
    assert_eq!(
        plan["result"]["selection_evidence"]["core_selection_count"],
        10
    );
    assert_eq!(plan["result"]["selection_evidence"]["forced_core_count"], 1);
    assert_eq!(
        plan["result"]["selection_evidence"]["positive_signal_core_count"],
        0
    );
    assert_eq!(
        plan["result"]["selection_evidence"]["stable_fallback_core_count"],
        9
    );
    assert_eq!(
        plan["result"]["selection_evidence"]["fallback_dominated"],
        true
    );
    assert_eq!(
        plan["result"]["selection_evidence"]["detail_level"],
        "summary"
    );
    let core_preview = plan["result"]["selection_evidence"]["agents"][0]["core_preview"]
        .as_array()
        .unwrap();
    assert_eq!(core_preview.len(), 5);
    assert!(core_preview.iter().all(|selection| {
        selection["skill_id"].is_string()
            && selection["name"].is_string()
            && selection["reason"].is_string()
    }));
    assert_eq!(
        plan["result"]["selection_evidence"]["agents"][0]["core_preview_truncated"],
        true
    );
    assert!(
        plan["result"]["selection_evidence"]["agents"][0]
            .get("core_selections")
            .is_none()
    );
    assert_eq!(
        plan["result"]["uncertainty"]["code"],
        "fallback_dominated_core_selection"
    );
    assert_eq!(plan["result"]["uncertainty"]["review_required"], true);

    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    assert_eq!(
        detail["result"]["selection_evidence"]["detail_level"],
        "full"
    );
    let core_selections = detail["result"]["selection_evidence"]["agents"][0]["core_selections"]
        .as_array()
        .unwrap();
    assert_eq!(core_selections.len(), 10);
    assert_eq!(&core_selections[..5], core_preview);
    assert!(core_selections.iter().all(|selection| {
        selection["skill_id"].is_string()
            && selection["name"].is_string()
            && selection["reason"].is_string()
    }));
    for field in [
        "core_selection_count",
        "forced_core_count",
        "positive_signal_core_count",
        "stable_fallback_core_count",
        "fallback_dominated",
        "fallback_dominated_agent_count",
        "reason_counts",
    ] {
        assert_eq!(
            detail["result"]["selection_evidence"][field],
            plan["result"]["selection_evidence"][field]
        );
    }
    assert_eq!(
        detail["result"]["uncertainty"],
        plan["result"]["uncertainty"]
    );
    assert_eq!(plan["result"]["files_changed"], false);
    assert_eq!(detail["result"]["files_changed"], false);
    assert!(
        plan["result"]["detail"]["contains"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_core_selections"))
    );
    let human = run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "plan",
            "--show",
            plan_id,
        ],
        None,
    );
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("0 signals · 1 forced · 9 fallback"));
    assert!(human.contains("fallback-dominated Core selection"));

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 10);
    assert_eq!(fs::read_dir(state.join("library")).unwrap().count(), 51);

    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 61);
    assert!(!state.join("library").exists());
}

#[test]
fn large_roster_plan_uses_exact_cross_agent_usage_before_fallback() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_root = home.join(".codex/skills");
    fs::create_dir_all(&codex_root).unwrap();
    for index in 0..51 {
        let name = if index == 0 {
            "alpha".to_owned()
        } else if index == 50 {
            "zeta".to_owned()
        } else {
            format!("skill-{index:03}")
        };
        let directory = codex_root.join(&name);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture {index}\n"),
        )
        .unwrap();
    }

    let cursor_skill = home.join(".cursor/skills/zeta");
    fs::create_dir_all(&cursor_skill).unwrap();
    fs::copy(
        codex_root.join("zeta/SKILL.md"),
        cursor_skill.join("SKILL.md"),
    )
    .unwrap();
    let cursor_sessions = home.join(".cursor/projects");
    fs::create_dir_all(&cursor_sessions).unwrap();
    fs::write(
        cursor_sessions.join("session.jsonl"),
        json!({
            "role": "assistant",
            "message": {"role": "assistant", "content": [{
                "type": "tool_use",
                "name": "Read",
                "input": {"path": cursor_skill.join("SKILL.md")}
            }]}
        })
        .to_string(),
    )
    .unwrap();

    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 1,
            "protected_skill_ids": []
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));

    let evidence = &plan["result"]["selection_evidence"];
    assert_eq!(evidence["positive_signal_core_count"], 1);
    assert_eq!(evidence["direct_signal_core_count"], 0);
    assert_eq!(evidence["cross_agent_signal_core_count"], 1);
    assert_eq!(evidence["stable_fallback_core_count"], 0);
    assert_eq!(evidence["reason_counts"]["cross_agent_observed_loaded"], 1);
    let selected = &evidence["agents"][0]["core_preview"][0];
    assert_eq!(selected["name"], "zeta");
    assert_eq!(selected["evidence_scope"], "cross_agent");
    assert_eq!(selected["evidence_agents"], json!(["cursor"]));
    assert_eq!(
        plan["result"]["uncertainty"]["code"],
        "cross_agent_dominated_core_selection"
    );
    assert_eq!(plan["result"]["uncertainty"]["review_required"], true);
    assert_eq!(plan["result"]["files_changed"], false);
}

#[test]
#[cfg(unix)]
fn roster_plan_keeps_summary_detail_and_confirmation_scope_consistent() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_root = home.join(".codex/skills");
    let opencode_root = home.join(".config/opencode/skills");
    fs::create_dir_all(&codex_root).unwrap();
    fs::create_dir_all(&opencode_root).unwrap();

    for index in 0..51 {
        let name = format!("skill-{index:03}");
        let directory = codex_root.join(&name);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let canonical = codex_root.join("zzz-canonical");
    fs::create_dir(&canonical).unwrap();
    fs::write(
        canonical.join("SKILL.md"),
        "---\nname: zzz-canonical\n---\nfixture\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&canonical, opencode_root.join("zzz-canonical")).unwrap();

    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));

    assert_eq!(plan["result"]["risk"], "roster_change");
    assert_eq!(detail["result"]["risk"], plan["result"]["risk"]);
    assert_eq!(
        plan["result"]["affected"]["agents"],
        json!(["codex", "opencode"])
    );
    assert_eq!(detail["result"]["affected"], plan["result"]["affected"]);
    assert_eq!(
        detail["result"]["change_summary"],
        plan["result"]["change_summary"]
    );
    assert_eq!(detail["result"]["impact"], plan["result"]["impact"]);
    assert_eq!(
        detail["result"]["diff_summary"],
        plan["result"]["diff_summary"]
    );
    assert_eq!(plan["result"]["diff_summary"]["item_count"], 3);
    assert_eq!(plan["result"]["diff_summary"]["items"][0]["kind"], "roster");
    assert_eq!(
        plan["result"]["diff_summary"]["items"][1]["kind"],
        "library"
    );
    assert_eq!(
        plan["result"]["diff_summary"]["items"][2]["kind"],
        "filesystem"
    );

    let human = run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "plan",
            "--show",
            plan_id,
        ],
        None,
    );
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("default exposure 53 → 51"));
    assert!(human.contains("2 Agents · 52 Skills"), "{human}");
    assert!(human.contains("Risk                   roster_change"));
}

#[test]
fn large_roster_apply_reduces_exposure_and_undo_restores_every_skill() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    for index in 0..101 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let evidence_id: String = database
        .query_row(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let roster_changes = payload["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| {
            json!({
                "agent": "codex",
                "skill_id": skill["id"],
                "state": "on_demand"
            })
        })
        .collect::<Vec<_>>();
    let proposal = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": roster_changes
    });
    let plan_output = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&proposal.to_string()),
    );
    assert!(
        plan_output.stdout.len() < 12_000,
        "large Roster summary was {} bytes",
        plan_output.stdout.len()
    );
    let plan = json_output(&plan_output);
    assert_eq!(plan["result"]["detail_level"], "summary");
    assert!(plan["result"].get("operations").is_none());
    assert!(plan["result"].get("roster_changes").is_none());
    assert_eq!(plan["result"]["affected"]["skill_count"], 101);
    assert!(
        plan["result"]["affected"]["skill_ids"]
            .as_array()
            .unwrap()
            .len()
            <= 10
    );
    assert_eq!(plan["result"]["affected"]["skill_ids_truncated"], true);
    assert_eq!(plan["result"]["impact"]["before_default_exposure"], 101);
    assert!(
        plan["result"]["impact"]["after_default_exposure"]
            .as_u64()
            .unwrap()
            <= 50
    );
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    assert_eq!(fs::read_dir(state.join("library")).unwrap().count(), 101);
    let governed_find = json_output(&run(&[&common[..], &["find", "skill-000"]].concat(), None));
    assert_eq!(
        governed_find["result"]["matches"][0]["roster_state"],
        "on_demand"
    );
    assert_find_paths_are_readable(&governed_find);
    assert!(
        governed_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| Path::new(path.as_str().unwrap())
                .components()
                .any(|component| component.as_os_str() == "library"))
    );
    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 101);
    assert!(!state.join("library").exists());
    for index in 0..101 {
        assert!(root.join(format!("skill-{index:03}/SKILL.md")).is_file());
    }
    let restored_find = json_output(&run(&[&common[..], &["find", "skill-000"]].concat(), None));
    assert_eq!(
        restored_find["result"]["matches"][0]["roster_state"],
        "unassigned"
    );
    assert_find_paths_are_readable(&restored_find);
}

#[test]
fn core_roster_adds_verified_link_and_undo_restores_absence() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_root = home.join(".codex/skills");
    let canonical = home.join(".claude/skills/shared");
    fs::create_dir_all(&codex_root).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::write(canonical.join("SKILL.md"), "---\nname: shared\n---\nbody\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = payload["skills"][0]["id"].as_str().unwrap();
    let evidence_id: String = database
        .query_row(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": [{"agent": "codex", "skill_id": skill_id, "state": "core"}]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert_eq!(
        plan["result"]["impact"]["operation_groups"]["add_core_exposure"],
        1
    );
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    let linked = codex_root.join("shared");
    assert!(
        fs::symlink_metadata(&linked)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&linked).unwrap(),
        fs::canonicalize(&canonical).unwrap()
    );
    let core_find = json_output(&run(&[&common[..], &["find", "shared"]].concat(), None));
    assert_eq!(core_find["result"]["matches"][0]["roster_state"], "core");
    assert_find_paths_are_readable(&core_find);
    assert!(
        core_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == linked.join("SKILL.md").to_str().unwrap())
    );
    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert!(!linked.exists());
    assert!(canonical.join("SKILL.md").is_file());
    let restored_find = json_output(&run(&[&common[..], &["find", "shared"]].concat(), None));
    assert_eq!(
        restored_find["result"]["matches"][0]["roster_state"],
        "unassigned"
    );
    assert_find_paths_are_readable(&restored_find);
    assert!(
        restored_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| path != linked.join("SKILL.md").to_str().unwrap())
    );

    let drift_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    fs::write(
        canonical.join("SKILL.md"),
        "---\nname: shared\n---\nchanged after plan\n",
    )
    .unwrap();
    let drifted = run(
        &[
            &common[..],
            &["apply", drift_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    );
    assert!(!drifted.status.success());
    assert!(!linked.exists());
}

#[test]
fn public_find_uses_full_fts_body_and_archive_undo_restores_routing() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/deep-search");
    fs::create_dir_all(&skill).unwrap();
    let filler = "ordinary ".repeat(80);
    fs::write(
        skill.join("SKILL.md"),
        format!(
            "---\nname: deep-search\ndescription: generic helper\n---\n{filler} phosphorescent-reconciliation\n"
        ),
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let found = json_output(&run(
        &[&common[..], &["find", "phosphorescent reconciliation"]].concat(),
        None,
    ));
    assert_eq!(found["result"]["matches"][0]["name"], "deep-search");

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = payload["skills"][0]["id"].as_str().unwrap();
    let evidence_id: String = database
        .query_row(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": [{"agent": "codex", "skill_id": skill_id, "state": "archived"}]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    let archived = json_output(&run(
        &[&common[..], &["find", "phosphorescent reconciliation"]].concat(),
        None,
    ));
    assert_eq!(archived["result"]["matches"], json!([]));

    json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    let restored = json_output(&run(
        &[&common[..], &["find", "phosphorescent reconciliation"]].concat(),
        None,
    ));
    assert_eq!(restored["result"]["matches"][0]["name"], "deep-search");
}

#[test]
fn archived_same_name_identity_cannot_return_through_active_variant() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex = home.join(".codex/skills/shared-route");
    let claude = home.join(".claude/skills/shared-route");
    fs::create_dir_all(&codex).unwrap();
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        codex.join("SKILL.md"),
        "---\nname: shared-route\ndescription: archived identity\n---\narchived-only-marker\n",
    )
    .unwrap();
    fs::write(
        claude.join("SKILL.md"),
        "---\nname: shared-route\ndescription: active identity\n---\nactive-search-marker\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let before = json_output(&run(
        &[&common[..], &["find", "active search marker"]].concat(),
        None,
    ));
    assert_eq!(before["result"]["matches"][0]["variant_count"], 2);

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let codex_skill_id = payload["placements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|placement| placement["agent"] == "codex")
        .and_then(|placement| placement["skill_id"].as_str())
        .unwrap();
    let evidence_id: String = database
        .query_row(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": [{
            "agent": "codex",
            "skill_id": codex_skill_id,
            "state": "archived"
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));

    let after = json_output(&run(
        &[&common[..], &["find", "active search marker"]].concat(),
        None,
    ));
    assert_eq!(after["result"]["matches"][0]["variant_count"], 1);
    assert!(after["result"]["matches"][0]["variants"].is_null());

    json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
}

#[test]
fn find_rejects_content_drift_and_requests_rescan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/drift-check");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: drift-check\n---\nneedle-before-drift\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: drift-check\n---\nlocally changed\n",
    )
    .unwrap();

    let found = json_output(&run(
        &[&common[..], &["find", "needle before drift"]].concat(),
        None,
    ));
    assert_eq!(found["result"]["matches"][0]["paths"], json!([]));
    assert_eq!(found["result"]["rescan_required"], true);
    assert!(
        found["result"]["warnings"][0]
            .as_str()
            .unwrap()
            .contains("skillroster scan")
    );
}

#[test]
fn public_report_covers_issue_nine_finding_families_without_runtime_claims() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    for (directory, version, body) in [
        ("mismatch-one", "v1", "first body"),
        ("mismatch-copy", "v1", "changed local body"),
        ("mismatch-two", "v2", "second version body"),
    ] {
        let package = root.join(directory);
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("SKILL.md"),
            format!(
                "---\nname: declared-name\nsource: github:owner/report-fixture\nversion: {version}\n---\n{body}\n"
            ),
        )
        .unwrap();
        fs::write(package.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report", "--full"]].concat(), None));
    let findings = report["result"]["findings"].as_array().unwrap();
    for title in [
        "Declared identity has divergent local content",
        "Skill packages contain executable scripts",
        "Declared Skill names differ from placement directories",
        "Declared source has version divergence",
        "Upstream update drift is not verified",
    ] {
        let finding = findings
            .iter()
            .find(|finding| finding["title"] == title)
            .unwrap_or_else(|| panic!("missing public Finding: {title}"));
        assert!(finding["affected_skill_count"].as_u64().unwrap() > 0);
        assert!(!finding["evidence_ids"].as_array().unwrap().is_empty());
    }
    let encoded = report.to_string().to_lowercase();
    assert!(!encoded.contains("is malicious"));
    assert!(!encoded.contains("is safe at runtime"));
}

#[cfg(unix)]
#[test]
fn find_rejects_a_skill_symlink_that_now_escapes_approved_roots() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/escape-check");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: escape-check\n---\nunique escape needle\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let outside = temp.path().join("outside-skill");
    fs::rename(&skill, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &skill).unwrap();

    let found = json_output(&run(
        &[&common[..], &["find", "unique escape needle"]].concat(),
        None,
    ));
    assert_eq!(found["result"]["matches"][0]["paths"], json!([]));
    assert_eq!(found["result"]["rescan_required"], true);
}

#[cfg(unix)]
#[test]
fn escaping_link_finding_requests_trust_confirmation_instead_of_a_plan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let source = temp.path().join("trusted-source");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: external\ndescription: fixture\n---\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&source, root.join("external")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(
        detail["result"]["resolution"]["decision"],
        "confirm_trusted_source_roots"
    );
    assert_eq!(
        detail["result"]["resolution"]["observed_link_targets"],
        json!([source])
    );
    assert_eq!(
        detail["result"]["resolution"]["automatic_change_supported"],
        false
    );
    assert_eq!(detail["suggested_actions"].as_array().unwrap().len(), 1);
    assert_eq!(
        detail["suggested_actions"][0]["action"],
        "show_full_finding"
    );
    assert!(
        detail["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["facts"]["link_target"] == json!(source))
    );
}

#[test]
fn report_distinguishes_inaccessible_and_missing_session_roots() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/sessions"), "not a directory").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
    ];
    let mut json_common = common.to_vec();
    json_common.push("--json");
    json_output(&run(&[&json_common[..], &["scan"]].concat(), None));

    let report = json_output(&run(
        &[&json_common[..], &["report", "--summary"]].concat(),
        None,
    ));
    assert_eq!(report["result"]["coverage_missing_agent_count"], 7);
    assert_eq!(report["result"]["coverage_inaccessible_agent_count"], 1);
    assert_eq!(
        report["result"]["session_coverage"]["missing_root_agents"],
        7
    );
    assert_eq!(
        report["result"]["session_coverage"]["inaccessible_agents"],
        1
    );
    let usage_findings = json_output(&run(
        &[
            &json_common[..],
            &["report", "--findings", "--category", "usage"],
        ]
        .concat(),
        None,
    ));
    let coverage_summary = usage_findings["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Usage coverage is incomplete")
        .unwrap()["summary"]
        .as_str()
        .unwrap();
    assert!(coverage_summary.contains("7 session roots are missing"));
    assert!(coverage_summary.contains("1 are inaccessible"));

    let human = run(&[&common[..], &["report", "--summary"]].concat(), None);
    assert!(human.status.success());
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("limited 0/8 · missing 7/8"), "{stdout}");
    assert!(stdout.contains("Inaccessible           1/8"), "{stdout}");
}
