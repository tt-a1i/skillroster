use serde_json::Value;
use std::{fs, process::Command};
use tempfile::TempDir;

fn command(temp: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .arg("--home")
        .arg(temp.path().join("home"))
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .args(args)
        .output()
        .unwrap()
}

fn run(temp: &TempDir, args: &[&str]) -> Value {
    let output = command(temp, &[&["--json"][..], args].concat());
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn mixed_variants_explain_incomplete_packages_and_keep_healthy_explicit_loading() {
    let temp = TempDir::new().unwrap();
    let healthy = temp.path().join("home/.hermes/skills/diagram");
    let incomplete = temp.path().join("home/.codex/skills/diagram");
    for path in [&healthy, &incomplete] {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("SKILL.md"), "---\nname: diagram\ndescription: Render architecture diagrams.\n---\nDraw the architecture.\n").unwrap();
    }
    fs::create_dir(incomplete.join("assets")).unwrap();
    for index in 0..17 {
        fs::write(
            incomplete.join(format!("assets/{index:02}.bin")),
            vec![0; 1024 * 1024],
        )
        .unwrap();
    }
    assert_eq!(run(&temp, &["scan"])["ok"], true);
    for materialize_report in [false, true] {
        if materialize_report {
            assert_eq!(run(&temp, &["report"])["ok"], true);
        }
        let found = run(&temp, &["find", "--limit", "1", "--", "diagram"]);
        assert_eq!(found["ok"], true);
        assert_eq!(found["result"]["rescan_required"], false);
        let candidate = &found["result"]["matches"][0];
        assert_eq!(candidate["variant_count"], 2);
        assert_eq!(candidate["variant_finding"]["state"], "incomplete_package");
        assert_eq!(
            candidate["variant_finding"]["reason_code"],
            "incomplete_variant_fingerprints"
        );
        assert!(
            !found["result"]["warnings"]
                .to_string()
                .contains("no matching current")
        );
        let variants = candidate["variants"].as_array().unwrap();
        let unavailable = variants
            .iter()
            .find(|v| v["paths"].as_array().unwrap().is_empty())
            .unwrap();
        assert_eq!(unavailable["path_availability"]["state"], "unavailable");
        assert_eq!(
            unavailable["path_availability"]["reason_codes"],
            serde_json::json!(["package_fingerprint_incomplete"])
        );
        let available = variants
            .iter()
            .find(|v| !v["paths"].as_array().unwrap().is_empty())
            .unwrap();
        assert_eq!(available["path_availability"]["state"], "available");
        let loaded = run(
            &temp,
            &[
                "find",
                "--load",
                "--variant-skill-id",
                available["skill_id"].as_str().unwrap(),
                "--",
                "diagram",
            ],
        );
        assert_eq!(loaded["ok"], true, "{loaded:#}");
        let blocked = run(
            &temp,
            &[
                "find",
                "--load",
                "--variant-skill-id",
                unavailable["skill_id"].as_str().unwrap(),
                "--",
                "diagram",
            ],
        );
        assert_eq!(
            blocked["error"]["details"]["reason"],
            "package_fingerprint_incomplete"
        );
        let action = found["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["action"] == "inspect_incomplete_variant_inventory")
            .unwrap();
        let argv = action["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>();
        let output = Command::new(argv[0]).args(&argv[1..]).output().unwrap();
        assert!(output.status.success());
        let inventory: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            inventory
                .to_string()
                .contains("incomplete_package_fingerprints")
        );
        let human = command(&temp, &["find", "--limit", "1", "--", "diagram"]);
        let human = String::from_utf8(human.stdout).unwrap();
        assert!(
            human.contains("inspect incomplete package inventory"),
            "{human}"
        );
        assert!(!human.contains("inspect layout Finding"), "{human}");
    }
    fs::write(healthy.join("changed.txt"), "payload drift").unwrap();
    let drifted = run(&temp, &["find", "--limit", "1", "--", "diagram"]);
    assert_eq!(drifted["result"]["rescan_required"], true);
    assert_eq!(
        drifted["result"]["matches"][0]["variant_finding"]["state"],
        "rescan_required"
    );
}
