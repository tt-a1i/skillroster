use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

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

    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    assert_eq!(report["ok"], true);
    assert!(report["result"]["findings"].is_array());
    assert!(report["result"]["primary_metrics"].is_object());
    let repeated_report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    assert_eq!(
        repeated_report["result"]["report_id"],
        report["result"]["report_id"]
    );
    let finding_id = report["result"]["findings"][0]["id"].as_str().unwrap();
    let finding = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(finding["result"]["id"], finding_id);
    assert!(finding["result"]["affected_skill_ids"].is_array());
    assert!(finding["result"]["affected_placement_ids"].is_array());
    assert!(finding["result"]["impact"].is_object());
    assert!(finding["result"]["coverage"].is_object());
    for evidence_id in finding["result"]["evidence_ids"].as_array().unwrap() {
        let exists: i64 = database
            .query_row(
                "SELECT COUNT(*) FROM evidence WHERE id = ?1",
                [evidence_id.as_str().unwrap()],
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
    assert_eq!(plan["result"]["evidence_ids"][0], evidence_id);
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
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert_eq!(plan["result"]["risk"], "source_update");
    assert_eq!(
        plan["result"]["diff_summary"][0]["choice"],
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
            .contains("immutable source baseline")
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
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": roster_changes
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&proposal.to_string()),
    ));
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
