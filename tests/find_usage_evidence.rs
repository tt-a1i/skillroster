use std::fs;
use std::process::Command;

use serde_json::{Value, json};
use tempfile::TempDir;

fn run(temp: &TempDir, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .arg("--home")
        .arg(temp.path().join("home"))
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn find_and_verified_load_keep_usage_out_of_relevance_evidence() {
    let temp = TempDir::new().unwrap();
    let package = temp.path().join("home/.codex/skills/research");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("SKILL.md"), "---\nname: research\ndescription: Verify primary sources\n---\nResearch original evidence.\n").unwrap();
    let snapshot = run(&temp, &["scan"]);
    let before = run(&temp, &["find", "research", "--load"]);
    let db = rusqlite::Connection::open(temp.path().join("state/skillroster.db")).unwrap();
    let id = snapshot["result"]["snapshot_id"].as_str().unwrap();
    let payload: String = db
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let mut payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = before["result"]["matches"][0]["skill_id"].as_str().unwrap();
    // A synthetic typed observation isolates presentation/ranking from log parsing.
    payload["usage"].as_array_mut().unwrap().push(json!({
        "agent":"pi", "skill_id":skill_id, "stage":"loaded", "quality":"observed",
        "event_count":99, "first_seen_unix":10, "last_seen_unix":20,
        "month_start_unix":null, "source_path_digest":"synthetic"
    }));
    db.execute(
        "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
        rusqlite::params![payload.to_string(), id],
    )
    .unwrap();
    drop(db);
    let after = run(&temp, &["find", "research", "--load"]);
    let candidate = &after["result"]["matches"][0];
    let selection = &after["result"]["loaded_skill"]["selection"];
    assert_eq!(candidate["score"], before["result"]["matches"][0]["score"]);
    assert_eq!(candidate["evidence_quality"], "inferred");
    assert_eq!(candidate["evidence_quality_scope"], "routing_relevance");
    assert_eq!(candidate["agents"], json!(["codex"]));
    let expected = json!({"skill_id":skill_id, "quality":"observed", "observed_stages":["loaded"], "evidence_agents":["pi"]});
    assert_eq!(candidate["local_usage_prior"], expected);
    assert_eq!(selection["local_usage_prior"], expected);
    assert!(
        !selection["ranking_evidence"]
            .as_array()
            .unwrap()
            .contains(&json!("observed_local_usage"))
    );
    assert_eq!(selection["evidence_quality"], "inferred");
    assert_eq!(after["result"]["loaded_skill"]["content"]["complete"], true);
}
