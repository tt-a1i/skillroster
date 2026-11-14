use std::fs;
use std::process::{Command, Stdio};

use serde_json::Value;
use tempfile::TempDir;

fn command(temp: &TempDir) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_skillroster"));
    cmd.args([
        "--home",
        temp.path().join("home").to_str().unwrap(),
        "--state-dir",
        temp.path().join("state").to_str().unwrap(),
    ]);
    cmd.stdin(Stdio::null());
    cmd
}

fn json(temp: &TempDir, args: &[&str]) -> Value {
    let out = command(temp).arg("--json").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn init_refusal_reports_no_mutation_before_creating_state() {
    let temp = TempDir::new().unwrap();
    let out = command(&temp).args(["init", "--json"]).output().unwrap();
    assert!(!out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["error"]["details"]["files_changed"], false);
    assert_eq!(value["error"]["details"]["mutation_started"], false);
    assert!(!temp.path().join("state").exists());
    let out = command(&temp).arg("init").output().unwrap();
    let human = String::from_utf8(out.stderr).unwrap();
    assert!(human.contains("File changes: none."), "{human}");
    assert!(!human.contains("File changes: unknown"));
}

#[test]
fn undo_preview_is_bounded_and_non_tty_refusal_keeps_receipt_undoable() {
    let temp = TempDir::new().unwrap();
    for index in 0..60 {
        let p = temp
            .path()
            .join(format!("home/.codex/skills/capability-{index:02}"));
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join("SKILL.md"),
            format!(
                "---\nname: capability-{index:02}\ndescription: Specific capability {index}\n---\n"
            ),
        )
        .unwrap();
    }
    let check = json(&temp, &["check"]);
    let finding = check["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "large_default_roster")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let plan = json(&temp, &["plan", "--finding", finding, "--core-limit", "2"]);
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let applied = json(&temp, &["apply", plan_id]);
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let out = command(&temp).args(["undo", receipt_id]).output().unwrap();
    assert!(!out.status.success());
    let human = String::from_utf8(out.stderr).unwrap();
    assert!(human.lines().count() < 35, "{human}");
    assert!(human.contains("Recorded operations"), "{human}");
    assert!(human.contains("Only this Receipt's changes"));
    assert!(!human.contains("/backups/"), "{human}");
    assert!(human.contains("File changes: none."), "{human}");
    assert!(!human.contains("File changes: unknown"));
    let undone = json(&temp, &["undo", receipt_id]);
    assert_eq!(undone["result"]["verification"], "passed");
    for index in 0..60 {
        let p = temp
            .path()
            .join(format!("home/.codex/skills/capability-{index:02}/SKILL.md"));
        assert_eq!(
            fs::read_to_string(p).unwrap(),
            format!(
                "---\nname: capability-{index:02}\ndescription: Specific capability {index}\n---\n"
            )
        );
    }
}
