use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

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
    let invalid_full = run(&[&common[..], &["report", "--full"]].concat(), None);
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
    let summary_report_output_len = summary_report_output.stdout.len();
    let summary_report = json_output(&summary_report_output);
    assert_eq!(
        summary_report["result"]["finding_count"],
        report["result"]["findings"].as_array().unwrap().len()
    );
    assert!(
        summary_report["result"]["findings"]
            .as_array()
            .unwrap()
            .len()
            <= 3
    );
    assert!(summary_report_output_len < report_output_len);
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
        "1.8.9"
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
    assert_eq!(output["result"]["bootstrap_version"], "1.8.9");
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
        ("codex", "codex"),
        ("claude-code", "claude_code"),
        ("pi", "pi"),
        ("opencode", "open_code"),
        ("hermes", "hermes"),
        ("cursor", "cursor"),
        ("gemini-cli", "gemini_cli"),
        ("github-copilot", "git_hub_copilot"),
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
    for (agent, _) in agents {
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
    for (_, json_agent) in agents {
        assert!(explicit.iter().any(|root| root["agent"] == json_agent));
    }
    assert!(explicit.iter().all(|root| root["status"] == "included"));
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

    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
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
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
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
