use serde_json::Value;
use std::{fs, process::Command};
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
    (
        output.status.success(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}
fn fixture() -> TempDir {
    let temp = TempDir::new().unwrap();
    for (agent, body) in [("codex", "PRIVATE_FIRST"), ("hermes", "PRIVATE_SECOND")] {
        let path = temp
            .path()
            .join(format!("home/.{agent}/skills/deployment-helper"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), format!("---\nname: deployment-helper\ndescription: Inspect deployment progress and workloads in staging; diagnose service rollout and list pod states.\n---\n{body}\n")).unwrap();
    }
    assert!(run(&temp, &["scan"]).0);
    temp
}
#[test]
fn weak_ambiguous_load_first_recovers_selection_evidence() {
    let temp = fixture();
    let task = "查看 staging 服务发布进度和 Pod 状态";
    let (ok, blocked) = run(
        &temp,
        &[
            "find",
            "--hint",
            "deployment status pod",
            "--load",
            "--",
            task,
        ],
    );
    assert!(!ok);
    assert_eq!(
        blocked["error"]["details"]["reason"], "hint_direct_selection_evidence_required",
        "{blocked}"
    );
    let action = &blocked["suggested_actions"][0];
    assert_eq!(action["action"], "inspect_ranked_skill_evidence");
    let argv = action["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(argv.contains(&"--require-snapshot"));
    assert!(!argv.contains(&"--load"));
    let output = Command::new(argv[0]).args(&argv[1..]).output().unwrap();
    assert!(output.status.success());
    let inspected: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inspected["result"]["matches"][0]["variant_count"], 2);
    assert!(!inspected.to_string().contains("PRIVATE_"));
    assert!(
        !inspected["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["action"].as_str().unwrap().starts_with("load_exact"))
    );

    let (ok, strong_blocked) = run(
        &temp,
        &[
            "find",
            "--hint",
            "inspect deployment progress",
            "--load",
            "--",
            task,
        ],
    );
    assert!(!ok);
    assert_eq!(
        strong_blocked["error"]["details"]["reason"], "same_name_variants_ambiguous",
        "{strong_blocked}"
    );
    let (ok, strong) = run(
        &temp,
        &["find", "--hint", "inspect deployment progress", "--", task],
    );
    assert!(ok, "{strong}");
    let loads = strong["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["action"] == "load_exact_variant_for_comparison")
        .collect::<Vec<_>>();
    assert_eq!(loads.len(), 2, "{strong}");
    for action in loads {
        let argv = action["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(argv.contains(&"--load"));
        assert!(argv.contains(&"--require-snapshot"));
        let output = Command::new(argv[0]).args(&argv[1..]).output().unwrap();
        let loaded: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success(), "{loaded}");
        assert_eq!(
            loaded["result"]["loaded_skill"]["selection"]["variant_selection"]["mode"],
            "explicit_skill_id"
        );
    }
}

#[test]
fn cjk_and_wrong_domain_weak_ambiguity_block_but_english_no_hint_keeps_ambiguity() {
    let temp = fixture();
    let (ok, cjk) = run(
        &temp,
        &[
            "find",
            "--load",
            "--",
            "查看 staging 服务发布进度和 Pod 状态",
        ],
    );
    assert!(!ok);
    assert_eq!(
        cjk["error"]["details"]["reason"], "cjk_hint_required_for_weak_match",
        "{cjk}"
    );
    let (ok, english) = run(&temp, &["find", "--load", "--", "deployment status pod"]);
    assert!(!ok);
    assert_eq!(
        english["error"]["details"]["reason"], "same_name_variants_ambiguous",
        "{english}"
    );
    let (ok, wrong) = run(
        &temp,
        &[
            "find",
            "--hint",
            "deployment specification decisions",
            "--load",
            "--",
            "整理产品决定成为规格",
        ],
    );
    assert!(!ok);
    assert_eq!(
        wrong["error"]["details"]["reason"], "hint_direct_selection_evidence_required",
        "{wrong}"
    );
    assert!(!wrong.to_string().contains("PRIVATE_"));
}
