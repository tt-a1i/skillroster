use std::{fs, path::Path, process::Command};

use serde_json::Value;
use tempfile::TempDir;

fn run(temp: &TempDir, args: &[&str]) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .arg("--home")
        .arg(temp.path().join("home"))
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap();
    (output.status.success(), value)
}

fn write_skill(temp: &TempDir, agent: &str, content: &str) {
    let path = temp.path().join(format!("home/.{agent}/skills/example"));
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("SKILL.md"), content).unwrap();
}

#[test]
fn blocked_load_recovery_inspects_bounded_metadata_without_body() {
    let temp = TempDir::new().unwrap();
    write_skill(
        &temp,
        "codex",
        "---\nname: deployment-helper\ndescription: Inspect deployment progress and workloads in staging; diagnose service rollout and list pod states.\ntriggers: [rollout, workloads]\n---\nBODY_PRIVATE_SENTINEL\n",
    );
    assert!(run(&temp, &["scan"]).0);
    let args = [
        "find",
        "--hint",
        "deployment status pod",
        "--limit",
        "1",
        "--load",
        "--",
        "查看 staging 服务发布进度和 Pod 状态",
    ];
    let (ok, blocked) = run(&temp, &args);
    assert!(!ok, "{blocked}");
    assert_eq!(
        blocked["error"]["details"]["reason"],
        "hint_direct_selection_evidence_required"
    );
    let action = &blocked["suggested_actions"][0];
    assert_eq!(action["action"], "inspect_ranked_skill_evidence");
    let argv = action["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    let output = Command::new(argv[0]).args(&argv[1..]).output().unwrap();
    assert!(output.status.success());
    let inspected: Value = serde_json::from_slice(&output.stdout).unwrap();
    let matched = &inspected["result"]["matches"][0];
    assert_eq!(
        matched["routing_metadata"]["description"],
        "Inspect deployment progress and workloads in staging; diagnose service rollout and list pod states."
    );
    assert_eq!(
        matched["routing_metadata"]["declared_triggers"],
        serde_json::json!(["rollout", "workloads"])
    );
    assert!(matched["ranking_content_identity_digest"].is_string());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("BODY_PRIVATE_SENTINEL"));
}

#[test]
fn ranked_metadata_comes_from_one_declared_identity_observation() {
    let temp = TempDir::new().unwrap();
    for (agent, description, trigger) in [
        ("codex", "alpha observatory analysis", "alpha"),
        ("hermes", "beta unrelated reporting", "beta"),
    ] {
        write_skill(
            &temp,
            agent,
            &format!(
                "---\nname: example\nsource: github:example/example\nversion: v1\ndescription: {description}\ntriggers: [{trigger}]\n---\nBODY_PRIVATE_SENTINEL\n"
            ),
        );
    }
    assert!(run(&temp, &["scan"]).0);
    let mut observed_ids = Vec::new();
    let mut observed_digests = Vec::new();
    for (query, trigger, agent) in [
        ("alpha observatory analysis", "alpha", "codex"),
        ("beta unrelated reporting", "beta", "hermes"),
    ] {
        let (ok, found) = run(&temp, &["find", "--limit", "1", "--", query]);
        assert!(ok, "{found}");
        let matched = &found["result"]["matches"][0];
        assert_eq!(matched["routing_metadata"]["description"], query);
        assert_eq!(
            matched["routing_metadata"]["declared_triggers"],
            serde_json::json!([trigger])
        );
        let groups = matched["content_choice"]["groups"].as_array().unwrap();
        let group = groups
            .iter()
            .find(|group| {
                group["placements"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|placement| {
                        Path::new(placement["path"].as_str().unwrap())
                            .components()
                            .map(|part| part.as_os_str())
                            .collect::<Vec<_>>()
                            .windows(2)
                            .any(|parts| {
                                parts
                                    == [
                                        std::ffi::OsStr::new(&format!(".{agent}")),
                                        std::ffi::OsStr::new("skills"),
                                    ]
                            })
                    })
            })
            .unwrap();
        assert!(group["content_identity_digest"].is_string());
        assert_eq!(
            matched["ranking_content_identity_digest"],
            group["content_identity_digest"]
        );
        observed_ids.push(matched["skill_id"].clone());
        observed_digests.push(matched["ranking_content_identity_digest"].clone());
        assert!(!found.to_string().contains("BODY_PRIVATE_SENTINEL"));
    }
    assert_eq!(observed_ids[0], observed_ids[1]);
    assert_ne!(observed_digests[0], observed_digests[1]);
}

#[test]
fn routing_metadata_bounds_unicode_and_marks_count_and_text_truncation() {
    let temp = TempDir::new().unwrap();
    let description = format!("needle {}", "界".repeat(520));
    let first_trigger = "触".repeat(129);
    write_skill(
        &temp,
        "codex",
        &format!(
            "---\nname: example\ndescription: {description}\ntriggers: [{first_trigger}, two, three, four, five, six]\n---\nBODY_PRIVATE_SENTINEL\n"
        ),
    );
    assert!(run(&temp, &["scan"]).0);
    let (ok, found) = run(&temp, &["find", "--limit", "1", "--", "needle"]);
    assert!(ok, "{found}");
    let metadata = &found["result"]["matches"][0]["routing_metadata"];
    assert_eq!(
        metadata["description"].as_str().unwrap().chars().count(),
        512
    );
    assert_eq!(metadata["description_truncated"], true);
    assert_eq!(metadata["trigger_count"], 6);
    assert_eq!(metadata["declared_triggers"].as_array().unwrap().len(), 5);
    assert_eq!(
        metadata["declared_triggers"][0]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        128
    );
    assert_eq!(metadata["triggers_truncated"], true);
    assert!(!found.to_string().contains("BODY_PRIVATE_SENTINEL"));
}
