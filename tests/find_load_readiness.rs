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
