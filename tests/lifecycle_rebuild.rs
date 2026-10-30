use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn invoke(args: &[&str]) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .args(args)
        .output()
        .unwrap();
    (
        output.status.success(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}
fn skill(root: &Path, name: &str) -> (std::path::PathBuf, Vec<u8>) {
    let path = root.join(name).join("SKILL.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = format!("---\nname: {name}\ndescription: {name} capability\n---\n{name} body\n")
        .into_bytes();
    fs::write(&path, &bytes).unwrap();
    (path, Sha256::digest(bytes).to_vec())
}

#[test]
fn delete_returns_context_bound_scan_that_rebuilds_inventory_without_changing_skills() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("custom-state");
    let codex = temp.path().join("codex-root");
    let claude = temp.path().join("claude-root");
    let source = temp.path().join("source-root");
    let files = [
        skill(&codex, "alpha"),
        skill(&claude, "beta"),
        skill(&source, "gamma"),
        skill(&state.join("library"), "delta"),
    ];
    let codex_arg = format!("codex={}", codex.display());
    let claude_arg = format!("claude-code={}", claude.display());
    let context = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--root",
        &codex_arg,
        "--root",
        &claude_arg,
        "--source-root",
        source.to_str().unwrap(),
        "--json",
    ];
    let (ok, before) = invoke(&[context.as_slice(), &["scan", "--summary"]].concat());
    assert!(ok, "{before}");
    assert_eq!(before["result"]["skill_count"], 4);
    let (ok, deleted) = invoke(
        &[
            context.as_slice(),
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
    );
    assert!(ok, "{deleted}");
    assert!(!state.join("skillroster.db").exists());
    let action = &deleted["suggested_actions"][0];
    assert_eq!(deleted["suggested_actions"].as_array().unwrap().len(), 1);
    assert_eq!(action["action"], "scan");
    assert_eq!(action["mutates"], false);
    assert_eq!(action["requires_confirmation"], true);
    let argv = action["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(argv[0], env!("CARGO_BIN_EXE_skillroster"));
    assert!(argv.windows(2).any(|w| w == ["--root", &codex_arg]));
    assert!(argv.windows(2).any(|w| w == ["--root", &claude_arg]));
    assert!(
        argv.windows(2)
            .any(|w| w == ["--source-root", source.to_str().unwrap()])
    );
    let output = Command::new(argv[0]).args(&argv[1..]).output().unwrap();
    let rebuilt: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{rebuilt}");
    assert_eq!(
        rebuilt["result"]["skill_count"],
        before["result"]["skill_count"]
    );
    assert!(state.join("skillroster.db").exists());
    assert!(!home.join(".skillroster/skillroster.db").exists());
    for (path, digest) in files {
        assert_eq!(Sha256::digest(fs::read(path).unwrap()).to_vec(), digest);
    }
}

#[test]
fn delete_requires_confirmation_and_recovery_before_offering_rebuild() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let context = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    assert!(invoke(&[context.as_slice(), &["scan"]].concat()).0);
    let (ok, refused) = invoke(
        &[
            context.as_slice(),
            &["lifecycle", "delete", "--confirm", "wrong"],
        ]
        .concat(),
    );
    assert!(!ok);
    assert!(
        !refused["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["action"] == "scan")
    );
    let (ok, setup) = invoke(&[context.as_slice(), &["setup"]].concat());
    assert!(ok, "{setup}");
    let (ok, applied) = invoke(
        &[
            context.as_slice(),
            &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
    );
    assert!(ok, "{applied}");
    let receipt = applied["result"]["receipt_id"].as_str().unwrap();
    let mut orphan: Value = serde_json::from_slice(
        &fs::read(state.join("receipts").join(format!("{receipt}.json"))).unwrap(),
    )
    .unwrap();
    let orphan_id = "receipt_00000000000000000000000000";
    orphan["id"] = json!(orphan_id);
    orphan["status"] = json!("recovery_required");
    let orphan_path = state.join("receipts").join(format!("{orphan_id}.json"));
    fs::write(&orphan_path, serde_json::to_vec(&orphan).unwrap()).unwrap();
    let (ok, blocked) = invoke(
        &[
            context.as_slice(),
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
    );
    assert!(!ok, "{blocked}");
    assert!(
        !blocked["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["action"] == "scan")
    );
    fs::remove_file(orphan_path).unwrap();
    // Undo removes receipt-owned external recovery material before deletion.
    let (ok, undone) = invoke(&[context.as_slice(), &["undo", receipt]].concat());
    assert!(ok, "{undone}");
    let (ok, deleted) = invoke(
        &[
            context.as_slice(),
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
    );
    assert!(ok, "{deleted}");
    assert_eq!(
        deleted["suggested_actions"][0]["requires_confirmation"],
        false
    );
}
