use std::fs;
use std::process::Command;

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
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {} / {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.success(), value)
}

#[test]
fn find_exposes_missing_selection_evidence_and_executable_read_only_recovery() {
    let temp = TempDir::new().unwrap();
    let package = temp.path().join("home/.codex/skills/deployment-helper");
    fs::create_dir_all(&package).unwrap();
    let contents = "---\nname: deployment-helper\ndescription: Inspect deployment progress and workloads in staging; diagnose service rollout and list pod states.\n---\nInspect status without changing deployments.\n";
    fs::write(package.join("SKILL.md"), contents).unwrap();
    assert!(run(&temp, &["scan"]).0);
    let task = "查看 staging 服务发布进度和 Pod 状态";
    for (hint, reason) in [
        (None, "cjk_hint_required_for_weak_match"),
        (
            Some("deployment status pod"),
            "hint_direct_selection_evidence_required",
        ),
    ] {
        let mut args = vec!["find", "--limit", "1"];
        if let Some(hint) = hint {
            args.extend(["--hint", hint]);
        }
        let (ok, discovered) = run(&temp, &[args.clone(), vec!["--", task]].concat());
        assert!(ok, "{discovered}");
        let candidate = &discovered["result"]["matches"][0];
        assert_eq!(candidate["name"], "deployment-helper");
        assert_eq!(candidate["load_readiness"]["state"], "blocked");
        assert_eq!(candidate["load_readiness"]["reason_codes"][0], reason);
        assert_eq!(candidate["load_readiness"]["verification_performed"], false);
        assert_eq!(
            candidate["load_readiness"]["direct_selection_evidence"],
            false
        );
        assert!(discovered["result"]["loaded_skill"].is_null());

        let (ok, blocked) = run(&temp, &[args, vec!["--load", "--", task]].concat());
        assert!(!ok);
        assert_eq!(blocked["error"]["details"]["reason"], reason);
        assert_eq!(blocked["error"]["details"]["files_changed"], false);
        assert!(blocked["result"].is_null());
        let action = &blocked["suggested_actions"][0];
        assert_eq!(action["action"], "inspect_ranked_skill_evidence");
        assert_eq!(action["mutates"], false);
        assert_eq!(action["requires_confirmation"], false);
        let argv = action["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(!argv.contains(&"--load"));
        assert!(!argv.contains(&"deployment-helper"));
        assert_eq!(argv.last(), Some(&task));
        assert!(argv.contains(&"--require-snapshot"));
        if let Some(hint) = hint {
            assert!(argv.windows(2).any(|pair| pair == ["--hint", hint]));
        }
        let replay = Command::new(argv[0]).args(&argv[1..]).output().unwrap();
        let inspected: Value = serde_json::from_slice(&replay.stdout).unwrap();
        assert!(replay.status.success(), "{inspected}");
        assert_eq!(inspected["result"]["task"], task);
        assert_eq!(
            inspected["result"]["snapshot_id"],
            discovered["result"]["snapshot_id"]
        );
        assert!(inspected["result"]["loaded_skill"].is_null());
    }
    let args = ["find", "--hint", "deployment-helper", "--limit", "1"];
    let (ok, selected) = run(&temp, &[args.as_slice(), &["--", task]].concat());
    assert!(ok);
    assert_eq!(
        selected["result"]["matches"][0]["load_readiness"]["state"],
        "verification_required"
    );
    assert_eq!(
        selected["result"]["matches"][0]["load_readiness"]["direct_selection_evidence"],
        true
    );
    let (ok, loaded) = run(&temp, &[args.as_slice(), &["--load", "--", task]].concat());
    assert!(ok, "{loaded}");
    assert_eq!(
        loaded["result"]["loaded_skill"]["selection"]["name"],
        "deployment-helper"
    );
    assert_eq!(
        fs::read_to_string(package.join("SKILL.md")).unwrap(),
        contents
    );
}

fn deployment_copies(declared: bool) -> TempDir {
    let temp = TempDir::new().unwrap();
    for (agent, body) in [
        ("codex", "Inspect status without changes."),
        ("hermes", "Report rollout without mutations."),
    ] {
        let package = temp
            .path()
            .join(format!("home/.{agent}/skills/deployment-helper"));
        fs::create_dir_all(&package).unwrap();
        let declaration = if declared {
            "source: github:example/deployment-helper\nversion: v1\n"
        } else {
            ""
        };
        fs::write(package.join("SKILL.md"), format!("---\nname: deployment-helper\n{declaration}description: Inspect deployment progress and workloads in staging; diagnose service rollout and list pod states.\n---\n{body}\n")).unwrap();
    }
    assert!(run(&temp, &["scan"]).0);
    temp
}

#[test]
fn discovery_blocks_known_declared_content_selection_before_load() {
    let temp = deployment_copies(true);
    let (ok, found) = run(&temp, &["find", "--limit", "1", "--", "deployment-helper"]);
    assert!(ok, "{found}");
    let candidate = &found["result"]["matches"][0];
    assert_eq!(candidate["variant_count"], 1);
    assert_eq!(
        candidate["content_choice"]["content_selection_required"],
        true
    );
    assert_eq!(candidate["load_readiness"]["state"], "blocked");
    assert!(
        candidate["load_readiness"]["reason_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "declared_identity_content_ambiguous")
    );
    let (ok, blocked) = run(
        &temp,
        &["find", "--load", "--limit", "1", "--", "deployment-helper"],
    );
    assert!(!ok);
    assert_eq!(
        blocked["error"]["code"],
        "declared_identity_content_ambiguous"
    );
    assert!(blocked["result"].is_null());
}

#[test]
fn discovery_exposes_known_entrypoint_transport_limit_without_loading_content() {
    let temp = TempDir::new().unwrap();
    let package = temp.path().join("home/.codex/skills/diagram");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        format!(
            "---\nname: diagram\ndescription: Render diagrams\n---\n{}",
            "x".repeat(140_000)
        ),
    )
    .unwrap();
    assert!(run(&temp, &["scan"]).0);
    let (ok, found) = run(&temp, &["find", "--limit", "1", "--", "diagram"]);
    assert!(ok, "{found}");
    let candidate = &found["result"]["matches"][0];
    assert!(!candidate["paths"].as_array().unwrap().is_empty());
    assert_eq!(candidate["load_readiness"]["state"], "blocked");
    assert!(
        candidate["load_readiness"]["reason_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "entrypoint_exceeds_content_limit")
    );
    assert_eq!(candidate["load_readiness"]["verification_performed"], false);
    assert!(found["result"]["loaded_skill"].is_null());
    let (ok, blocked) = run(&temp, &["find", "--load", "--limit", "1", "--", "diagram"]);
    assert!(!ok);
    assert_eq!(
        blocked["error"]["details"]["reason"],
        "entrypoint_exceeds_content_limit"
    );
}

#[test]
fn weak_selection_never_advertises_comparison_loads_that_its_evidence_gate_refuses() {
    let task = "查看 staging 服务发布进度和 Pod 状态";
    for declared in [false, true] {
        let temp = deployment_copies(declared);
        for hint in [None, Some("deployment status pod")] {
            let mut args = vec!["find", "--limit", "1"];
            if let Some(hint) = hint {
                args.extend(["--hint", hint]);
            }
            let (ok, found) = run(&temp, &[args, vec!["--", task]].concat());
            assert!(ok, "{found}");
            let actions = found["suggested_actions"].as_array().unwrap();
            assert!(
                !actions.is_empty(),
                "inspection must remain available: {found}"
            );
            assert!(
                !actions.iter().any(|a| a["argv"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|arg| arg == "--load")),
                "{found}"
            );
            assert_eq!(
                found["result"]["matches"][0]["load_readiness"]["state"],
                "blocked"
            );
        }
        let (ok, exact) = run(
            &temp,
            &[
                "find",
                "--hint",
                "deployment-helper",
                "--limit",
                "1",
                "--",
                task,
            ],
        );
        assert!(ok, "{exact}");
        let loads = exact["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["action"].as_str().unwrap().starts_with("load_exact"))
            .collect::<Vec<_>>();
        assert_eq!(loads.len(), 2, "{exact}");
        for action in loads {
            let argv = action["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>();
            let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
                .args(&argv[1..])
                .output()
                .unwrap();
            let loaded: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(output.status.success(), "{loaded}");
            assert_eq!(loaded["result"]["files_changed"], false);
            assert_eq!(loaded["result"]["task"], task);
            assert_eq!(
                loaded["result"]["loaded_skill"]["selection"]["name"],
                "deployment-helper"
            );
        }
    }
}

#[test]
fn same_name_comparison_actions_only_offer_loadable_selected_content() {
    for declared in [false, true] {
        let temp = deployment_copies(declared);
        if declared {
            // Two contents under one declared identity require placement selection,
            // while an unrelated same-name identity remains selectable by Skill ID.
            let package = temp.path().join("home/.claude/skills/deployment-helper");
            fs::create_dir_all(&package).unwrap();
            fs::write(package.join("SKILL.md"), "---\nname: deployment-helper\ndescription: Inspect deployments\n---\nOther implementation\n").unwrap();
        } else {
            // This variant has a complete inventory fingerprint and a current path,
            // but exceeds the stricter verified-load transport bound.
            let package = temp.path().join("home/.hermes/skills/deployment-helper");
            fs::write(
                package.join("SKILL.md"),
                format!(
                    "---\nname: deployment-helper\ndescription: Inspect deployments\n---\n{}",
                    "x".repeat(140_000)
                ),
            )
            .unwrap();
        }
        assert!(run(&temp, &["scan"]).0);
        let (ok, found) = run(&temp, &["find", "--limit", "1", "--", "deployment-helper"]);
        assert!(ok, "{found}");
        let actions = found["suggested_actions"].as_array().unwrap();
        let loads = actions
            .iter()
            .filter(|a| a["action"].as_str().unwrap().starts_with("load_exact"))
            .collect::<Vec<_>>();
        assert_eq!(loads.len(), if declared { 3 } else { 1 }, "{found}");
        assert_eq!(
            loads
                .iter()
                .filter(|a| a["action"] == "load_exact_variant_for_comparison")
                .count(),
            1,
            "{found}"
        );
        assert_eq!(
            loads
                .iter()
                .filter(|a| a["action"] == "load_exact_content_for_comparison")
                .count(),
            if declared { 2 } else { 0 },
            "{found}"
        );
        for action in loads {
            let argv = action["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>();
            let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
                .args(&argv[1..])
                .output()
                .unwrap();
            let loaded: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(output.status.success(), "{loaded}");
            assert_eq!(loaded["result"]["files_changed"], false);
            assert_eq!(
                loaded["result"]["loaded_skill"]["selection"]["name"],
                "deployment-helper"
            );
        }
    }
}
