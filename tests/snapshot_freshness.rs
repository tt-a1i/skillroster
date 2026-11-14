use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};
use tempfile::TempDir;

fn command(temp: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skillroster"));
    command.args([
        "--home",
        temp.path().join("home").to_str().unwrap(),
        "--state-dir",
        temp.path().join("state").to_str().unwrap(),
    ]);
    command
}
fn check(temp: &TempDir) -> Value {
    let output = command(temp).args(["check", "--json"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
}
fn skill(temp: &TempDir, relative: &str, text: &str) {
    let path = temp.path().join("home").join(relative);
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: review\ndescription: {text}\n---\n{text}\n"),
    )
    .unwrap();
}
fn mcp_find(temp: &TempDir, snapshot: Option<&Value>) -> Value {
    let mut args = json!({"task":"review"});
    if let Some(snapshot) = snapshot {
        args["require_snapshot"] = snapshot.clone();
    }
    let frames = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"find_skill","arguments":args}}),
    ];
    let mut child = command(temp)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for frame in frames {
        writeln!(input, "{frame}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response: Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    response["result"]["structuredContent"].clone()
}

#[test]
fn check_refreshes_external_add_delete_and_same_length_edit_and_reuses_fresh_snapshot() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("home")).unwrap();
    let first = check(&temp);
    assert_eq!(check(&temp)["scan_performed"], false);
    // A previously missing shared root becomes present.
    skill(&temp, ".agents/skills/review", "review alpha");
    let added = check(&temp);
    assert_eq!(added["scan_performed"], true);
    assert_ne!(added["snapshot_id"], first["snapshot_id"]);
    assert_eq!(check(&temp)["scan_performed"], false);
    skill(&temp, ".agents/skills/review", "review bravo");
    assert_eq!(check(&temp)["scan_performed"], true);
    let reference = temp.path().join("home/.agents/skills/review/reference.md");
    fs::write(&reference, "support alpha").unwrap();
    assert_eq!(check(&temp)["scan_performed"], true);
    fs::write(&reference, "support bravo").unwrap();
    assert_eq!(check(&temp)["scan_performed"], true);
    fs::remove_dir_all(temp.path().join("home/.agents/skills/review")).unwrap();
    let deleted = check(&temp);
    assert_eq!(deleted["scan_performed"], true);
    assert_eq!(check(&temp)["snapshot_id"], deleted["snapshot_id"]);
}

#[test]
fn mcp_pinned_snapshot_refuses_external_change_without_rescan_and_unpinned_refreshes() {
    let temp = TempDir::new().unwrap();
    skill(&temp, ".codex/skills/review", "review alpha");
    let first = check(&temp);
    skill(&temp, ".codex/skills/review", "review bravo");
    let pinned = mcp_find(&temp, Some(&first["snapshot_id"]));
    assert_eq!(pinned["ok"], false, "{pinned}");
    assert_eq!(
        pinned["error"]["details"]["reason"],
        "filesystem_snapshot_changed"
    );
    // The rejected pinned read did not make a new Snapshot.
    let output = command(&temp).args(["report", "--json"]).output().unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["result"]["snapshot_id"], first["snapshot_id"]);
    let fresh = mcp_find(&temp, None);
    assert_eq!(fresh["ok"], true, "{fresh}");
    assert_eq!(fresh["result"]["scan_performed"], true);
    assert_eq!(mcp_find(&temp, None)["result"]["scan_performed"], false);
    skill(&temp, ".codex/skills/second", "review second");
    assert_eq!(mcp_find(&temp, None)["result"]["scan_performed"], true);
    fs::remove_dir_all(temp.path().join("home/.codex/skills/second")).unwrap();
    assert_eq!(mcp_find(&temp, None)["result"]["scan_performed"], true);
}

#[cfg(unix)]
#[test]
fn check_observes_approved_linked_skill_changes_but_does_not_follow_external_sources() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    skill(&temp, ".agents_skills/review", "review alpha");
    fs::create_dir_all(temp.path().join("home/.codex/skills")).unwrap();
    symlink(
        temp.path().join("home/.agents_skills/review"),
        temp.path().join("home/.codex/skills/review"),
    )
    .unwrap();
    check(&temp);
    assert_eq!(check(&temp)["scan_performed"], false);
    skill(&temp, ".agents_skills/review", "review bravo");
    assert_eq!(check(&temp)["scan_performed"], true);
    let external = temp.path().join("external");
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("SKILL.md"), "external alpha").unwrap();
    symlink(&external, temp.path().join("home/.codex/skills/external")).unwrap();
    check(&temp);
    fs::write(external.join("SKILL.md"), "external bravo").unwrap();
    assert_eq!(check(&temp)["scan_performed"], false);
}

#[test]
fn legacy_missing_signature_is_rejected_when_pinned_and_refreshed_once_when_unpinned() {
    let temp = TempDir::new().unwrap();
    skill(&temp, ".codex/skills/review", "review alpha");
    let first = check(&temp);
    let store =
        skillroster::sqlite::StateStore::open(temp.path().join("state/skillroster.db")).unwrap();
    let (id, mut payload) = store.latest_scan_payload::<Value>().unwrap().unwrap();
    payload
        .as_object_mut()
        .unwrap()
        .remove("filesystem_fingerprint");
    store.save_scan_payload(&id, &payload).unwrap();
    drop(store);
    let pinned = mcp_find(&temp, Some(&first["snapshot_id"]));
    assert_eq!(pinned["ok"], false, "{pinned}");
    assert_eq!(
        pinned["error"]["details"]["reason"],
        "filesystem_snapshot_fingerprint_missing"
    );
    assert_eq!(check(&temp)["scan_performed"], true);
    assert_eq!(check(&temp)["scan_performed"], false);
}

#[test]
fn installed_plugin_skill_changes_refresh_without_scanning_uninstalled_cache_contents() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("home")).unwrap();
    check(&temp);
    let plugin = temp
        .path()
        .join("home/.codex/plugins/cache/market/reviewer");
    skill(
        &temp,
        ".codex/plugins/cache/market/reviewer/1.0/skills/review",
        "review plugin alpha",
    );
    // Cache alone does not establish installation or an approved discovery root.
    assert_eq!(check(&temp)["scan_performed"], false);
    fs::write(
        plugin.join(".codex-remote-plugin-install.json"),
        r#"{"schema_version":1,"remote_plugin_id":"reviewer_market"}"#,
    )
    .unwrap();
    assert_eq!(check(&temp)["scan_performed"], true);
    assert_eq!(check(&temp)["scan_performed"], false);
    skill(
        &temp,
        ".codex/plugins/cache/market/reviewer/1.0/skills/review",
        "review plugin bravo",
    );
    assert_eq!(check(&temp)["scan_performed"], true);
    fs::remove_file(plugin.join(".codex-remote-plugin-install.json")).unwrap();
    assert_eq!(check(&temp)["scan_performed"], true);
}

#[test]
fn project_skill_directories_are_only_explicit_selection_hints() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("home")).unwrap();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join(".agents/skills/review")).unwrap();
    fs::write(
        project.join(".agents/skills/review/SKILL.md"),
        "project skill bytes",
    )
    .unwrap();
    let output = command(&temp)
        .current_dir(&project)
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: Value = serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone();
    let hints = result["project_local_skill_roots"].as_array().unwrap();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0]["included_in_snapshot"], false);
    assert_eq!(hints[0]["contents_read"], false);
    assert_eq!(hints[0]["agent"], "codex");
    assert_eq!(hints[0]["argv"][0], env!("CARGO_BIN_EXE_skillroster"));
    let argv = hints[0]["argv"].as_array().unwrap();
    assert!(argv.iter().any(|arg| arg == "--root"));
    assert!(argv.iter().any(|arg| arg == "--state-dir"));
    let store =
        skillroster::sqlite::StateStore::open(temp.path().join("state/skillroster.db")).unwrap();
    let (_, payload) = store.latest_scan_payload::<Value>().unwrap().unwrap();
    assert!(payload["skills"].as_array().unwrap().is_empty());
    assert!(
        fs::read_dir(project.join(".agents/skills"))
            .unwrap()
            .count()
            == 1
    );
}
