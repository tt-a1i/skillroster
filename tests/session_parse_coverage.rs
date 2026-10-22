use std::fs;
use std::process::Command;
use std::time::{Duration, SystemTime};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;

fn fixture() -> TempDir {
    let temp = TempDir::new().unwrap();
    for name in ["example", "fallback-only"] {
        let root = temp.path().join("home/.codex/skills").join(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} fixture\n---\n"),
        )
        .unwrap();
    }
    fs::create_dir_all(temp.path().join("home/.codex/sessions")).unwrap();
    temp
}

fn cli(temp: &TempDir, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .args(["--home", temp.path().join("home").to_str().unwrap()])
        .args([
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "--json",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["ok"], true, "{output}");
    output["result"].clone()
}

fn codex_coverage(scan: &Value) -> &Value {
    scan["coverage"]
        .as_array()
        .unwrap()
        .iter()
        .find(|coverage| coverage["agent"] == "codex")
        .unwrap()
}

fn usage_finding(report: &Value) -> &str {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Five-stage usage evidence")
        .unwrap()["id"]
        .as_str()
        .unwrap()
}

fn codex_evidence(finding: &Value) -> &Value {
    finding["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .find(|evidence| evidence["kind"] == "coverage" && evidence["details"]["agent"] == "codex")
        .unwrap()
}

#[test]
fn malformed_records_preserve_valid_neighbors_without_name_fallback_or_raw_storage() {
    let temp = fixture();
    let sessions = temp.path().join("home/.codex/sessions");
    let valid = "{\"type\":\"invoke_skill\",\"invoked_skill\":\"example\"}\n";
    let malformed = format!("{valid}{{broken fallback-only PRIVATE-SESSION-SECRET\n{valid}");
    let mut invalid_utf8 = b"{\"skill_name\":\"fallback-only\",\"text\":\"".to_vec();
    invalid_utf8.extend_from_slice(&[0xff, 0xfe]);
    invalid_utf8.extend_from_slice(b"\"}\n");
    invalid_utf8.extend_from_slice(valid.as_bytes());
    let broken_document = format!("{{\"events\":[{valid}], broken fallback-only}}");
    let originals = [
        ("mixed.jsonl", malformed.into_bytes()),
        ("invalid-utf8.log", invalid_utf8),
        ("broken.json", broken_document.into_bytes()),
        (
            "unknown.jsonl",
            b"{\"type\":\"message\",\"content\":\"ordinary conversation\"}\n{\"unknown_schema\":true}\n".to_vec(),
        ),
    ];
    for (name, bytes) in &originals {
        fs::write(sessions.join(name), bytes).unwrap();
    }
    let scan = cli(&temp, &["scan"]);
    let coverage = codex_coverage(&scan);
    assert_eq!(coverage["parse_contract_version"], 1);
    assert_eq!(coverage["denominator_reliable"], false);
    let failures: Vec<_> = coverage["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| fact["code"] == "file_parse_failure")
        .collect();
    assert_eq!(failures.len(), 1, "{coverage}");
    for (field, expected) in [
        ("scope", json!("file")),
        ("count_kind", json!("exact")),
        ("unit", json!("files")),
        ("observed", json!(3)),
    ] {
        assert_eq!(failures[0][field], expected, "{coverage}");
    }
    let report = cli(&temp, &["report", "--full"]);
    assert_eq!(report["coverage_reliable_agent_count"], 0);
    let finding = cli(
        &temp,
        &["report", "--finding", usage_finding(&report), "--full"],
    );
    let usage = finding["usage_overview"]["observed_skills"]
        .as_array()
        .unwrap();
    assert_eq!(usage.len(), 1, "{usage:?}");
    assert_eq!(usage[0]["skill_name"], "example");
    assert_eq!(usage[0]["quality"], "observed");
    assert_eq!(usage[0]["stage"], "applied");
    assert_eq!(usage[0]["event_count"], 3);
    let details = &codex_evidence(&finding)["details"];
    assert_eq!(details["limitation_state"], "limited");
    assert!(
        details["actionability"]["supported_next_steps"]
            .as_array()
            .unwrap()
            .contains(&json!("verify_session_record_format_before_rescan"))
    );
    for (name, bytes) in &originals {
        assert_eq!(fs::read(sessions.join(name)).unwrap(), *bytes);
    }
    for entry in fs::read_dir(temp.path().join("state")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let stored = fs::read(path).unwrap();
            assert!(
                !stored
                    .windows(b"PRIVATE-SESSION-SECRET".len())
                    .any(|part| part == b"PRIVATE-SESSION-SECRET")
            );
        }
    }
    // Legal records without Skill signals are a complete negative observation.
    for (name, _) in originals.iter().take(3) {
        fs::remove_file(sessions.join(name)).unwrap();
    }
    fs::write(sessions.join("unknown.log"), "{\"future_schema\":42}\n").unwrap();
    let clean = cli(&temp, &["scan"]);
    assert_eq!(codex_coverage(&clean)["denominator_reliable"], true);
    assert_eq!(codex_coverage(&clean)["limitations"], json!([]));
    let clean_report = cli(&temp, &["report", "--full"]);
    let clean_finding = cli(
        &temp,
        &[
            "report",
            "--finding",
            usage_finding(&clean_report),
            "--full",
        ],
    );
    assert_eq!(
        clean_finding["usage_overview"]["observed_skills"],
        json!([])
    );
}

fn remove_parse_versions(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("parse_contract_version");
            object.remove("session_coverage_presentation_version");
            for child in object.values_mut() {
                remove_parse_versions(child);
            }
        }
        Value::Array(array) => array.iter_mut().for_each(remove_parse_versions),
        _ => {}
    }
}

#[test]
fn legacy_cached_coverage_is_unknown_across_public_views_and_rescan_restores_completeness() {
    let temp = fixture();
    fs::write(
        temp.path().join("home/.codex/sessions/valid.jsonl"),
        "{\"type\":\"invoke_skill\",\"invoked_skill\":\"example\"}\n",
    )
    .unwrap();
    let scan = cli(&temp, &["scan"]);
    assert_eq!(codex_coverage(&scan)["denominator_reliable"], true);
    let report = cli(&temp, &["report", "--full"]);
    assert_eq!(report["coverage_reliable_agent_count"], 1);
    let historical_finding = usage_finding(&report).to_owned();
    let before = cli(
        &temp,
        &["report", "--finding", &historical_finding, "--full"],
    );
    assert_eq!(
        before["usage_overview"]["coverage"]["complete_agent_count"],
        1
    );
    assert!(before["summary"].as_str().unwrap().contains("complete 1/8"));
    let positive = before["usage_overview"]["observed_skills"].clone();
    assert!(!positive.as_array().unwrap().is_empty());

    // Recreate pre-contract storage, retaining the old true flag and empty limitations.
    let database = Connection::open(temp.path().join("state/skillroster.db")).unwrap();
    for (table, column) in [
        ("scan_payloads", "payload_json"),
        ("reports", "summary_json"),
        ("evidence", "details_json"),
    ] {
        let mut statement = database
            .prepare(&format!("SELECT rowid, {column} FROM {table}"))
            .unwrap();
        let rows: Vec<(i64, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(statement);
        for (rowid, encoded) in rows {
            let mut value: Value = serde_json::from_str(&encoded).unwrap();
            remove_parse_versions(&mut value);
            database
                .execute(
                    &format!("UPDATE {table} SET {column} = ?1 WHERE rowid = ?2"),
                    params![value.to_string(), rowid],
                )
                .unwrap();
        }
    }
    drop(database);
    let historical = cli(
        &temp,
        &["report", "--finding", &historical_finding, "--full"],
    );
    assert_eq!(
        historical["usage_overview"]["coverage"]["complete_agent_count"],
        0
    );
    assert!(
        historical["summary"]
            .as_str()
            .unwrap()
            .contains("complete 0/8")
    );
    assert!(
        !historical["summary"]
            .as_str()
            .unwrap()
            .contains("complete 1/8")
    );
    assert_eq!(historical["usage_overview"]["observed_skills"], positive);
    let details = &codex_evidence(&historical)["details"];
    assert_eq!(details["limitation_state"], "legacy_unknown");
    assert_eq!(details["denominator_reliable"], false);
    assert_eq!(
        details["actionability"]["inference_boundary"]["usage_denominator_supported"],
        false
    );
    for args in [
        vec!["report"],
        vec!["report", "--summary"],
        vec!["report", "--full"],
        vec!["report", "--findings"],
    ] {
        let view = cli(&temp, &args);
        assert_eq!(view["coverage_reliable_agent_count"], 0, "{args:?}: {view}");
        assert_eq!(view["session_coverage"]["complete_agents"], 0);
    }
    let refreshed = cli(&temp, &["report", "--full"]);
    assert_ne!(refreshed["report_id"], report["report_id"]);
    let current = cli(
        &temp,
        &["report", "--finding", usage_finding(&refreshed), "--full"],
    );
    assert_eq!(current["usage_overview"]["observed_skills"], positive);
    assert_eq!(
        codex_evidence(&current)["details"]["limitation_state"],
        "legacy_unknown"
    );
    let export_path = temp.path().join("legacy-export.json");
    cli(
        &temp,
        &[
            "lifecycle",
            "export",
            "--output",
            export_path.to_str().unwrap(),
        ],
    );
    let exported: Value = serde_json::from_slice(&fs::read(export_path).unwrap()).unwrap();
    let coverage = exported["data"]["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|record| record["kind"] == "coverage")
        .collect::<Vec<_>>();
    assert!(!coverage.is_empty());
    assert!(
        coverage
            .iter()
            .all(|record| record["details"]["denominator_reliable"] == false
                && record["details"]["limitation_state"] == "legacy_unknown")
    );
    let rescanned = cli(&temp, &["scan"]);
    assert_eq!(codex_coverage(&rescanned)["parse_contract_version"], 1);
    assert_eq!(codex_coverage(&rescanned)["denominator_reliable"], true);
    assert_eq!(cli(&temp, &["report"])["coverage_reliable_agent_count"], 1);
}

#[test]
fn historical_archive_candidate_loses_actionability_when_parse_coverage_is_unknown() {
    let temp = fixture();
    let old = SystemTime::now() - Duration::from_secs(181 * 24 * 60 * 60);
    for name in ["example", "fallback-only"] {
        fs::OpenOptions::new()
            .write(true)
            .open(
                temp.path()
                    .join("home/.codex/skills")
                    .join(name)
                    .join("SKILL.md"),
            )
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();
    }
    let scan = cli(&temp, &["scan"]);
    assert_eq!(codex_coverage(&scan)["denominator_reliable"], true);
    let report = cli(&temp, &["report", "--full"]);
    let id = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["kind"] == "stale_archive_candidates")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let before = cli(&temp, &["report", "--finding", id, "--full"]);
    assert_eq!(before["actionability"], "review_required");
    assert!(
        before["summary"]
            .as_str()
            .unwrap()
            .contains("reliable covered Agent windows")
    );
    let database = Connection::open(temp.path().join("state/skillroster.db")).unwrap();
    let encoded: String = database
        .query_row("SELECT payload_json FROM scan_payloads", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut legacy: Value = serde_json::from_str(&encoded).unwrap();
    remove_parse_versions(&mut legacy);
    database
        .execute(
            "UPDATE scan_payloads SET payload_json = ?1",
            [legacy.to_string()],
        )
        .unwrap();
    let historical = cli(&temp, &["report", "--finding", id, "--full"]);
    assert_eq!(historical["id"], before["id"]);
    assert_eq!(historical["kind"], "stale_archive_candidates");
    assert_eq!(historical["evidence_quality"], "unknown");
    assert_eq!(historical["coverage"]["evidence_quality"], "unknown");
    assert_eq!(historical["actionability"], "read_only");
    assert_eq!(historical["reversibility"], "not_governable");
    let summary = historical["summary"].as_str().unwrap();
    assert!(summary.contains("Coverage is insufficient"), "{summary}");
    assert!(summary.to_ascii_lowercase().contains("rescan"), "{summary}");
    assert!(
        !summary.contains("reliable covered Agent windows"),
        "{summary}"
    );
    let unchanged: String = database
        .query_row("SELECT payload_json FROM scan_payloads", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&unchanged).unwrap(), legacy);
}
