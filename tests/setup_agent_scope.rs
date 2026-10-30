use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, process::Command};
use tempfile::TempDir;

fn run(t: &TempDir, args: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .args([
            "--home",
            t.path().join("home").to_str().unwrap(),
            "--state-dir",
            t.path().join("state").to_str().unwrap(),
            "--json",
        ])
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}
fn ok(t: &TempDir, args: &[&str]) -> Value {
    let (code, v) = run(t, args);
    assert_eq!(code, 0, "{v}");
    v
}
fn fixture() -> TempDir {
    let t = TempDir::new().unwrap();
    for p in [
        "home/.codex/skills",
        "home/.claude/skills",
        "home/unrelated/empty",
    ] {
        fs::create_dir_all(t.path().join(p)).unwrap();
    }
    fs::write(t.path().join("home/unrelated/sentinel"), b"preserve me").unwrap();
    fs::write(
        t.path().join("home/.claude/skills/sentinel"),
        b"claude user file",
    )
    .unwrap();
    t
}
#[cfg(unix)]
fn permission_bits(m: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode()
}
#[cfg(not(unix))]
fn permission_bits(m: &fs::Metadata) -> u32 {
    u32::from(m.permissions().readonly())
}
fn tree(path: &Path) -> BTreeMap<String, (String, u32, Vec<u8>)> {
    fn walk(base: &Path, p: &Path, out: &mut BTreeMap<String, (String, u32, Vec<u8>)>) {
        let m = fs::symlink_metadata(p).unwrap();
        let key = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
        let (kind, data) = if m.file_type().is_symlink() {
            (
                "link",
                fs::read_link(p)
                    .unwrap()
                    .to_string_lossy()
                    .as_bytes()
                    .to_vec(),
            )
        } else if m.is_dir() {
            ("dir", vec![])
        } else {
            ("file", fs::read(p).unwrap())
        };
        out.insert(key, (kind.into(), permission_bits(&m), data));
        if m.is_dir() {
            for e in fs::read_dir(p).unwrap() {
                walk(base, &e.unwrap().path(), out)
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(path, path, &mut out);
    out
}
#[test]
fn selected_setup_previews_applies_and_undoes_only_requested_agent() {
    let t = fixture();
    let home = t.path().join("home");
    let initial = tree(&home);
    let claude = tree(&home.join(".claude"));
    ok(&t, &["scan"]);
    let all = ok(&t, &["setup"]);
    assert_eq!(
        all["result"]["affected"]["agents"],
        json!(["claude-code", "codex"])
    );
    assert_eq!(all["result"]["operation_count"], 12);
    assert!(all["result"].get("requested_agent").is_none());
    let check = ok(&t, &["setup", "--agent", "codex", "--check"]);
    assert_eq!(check["result"]["requested_agent"], "codex");
    assert_eq!(
        check["result"]["detected_agents"].as_array().unwrap().len(),
        2
    );
    assert_eq!(check["result"]["targets"].as_array().unwrap().len(), 1);
    assert!(check["result"]["plan_id"].is_null());
    let preview = ok(&t, &["setup", "--agent", "codex"]);
    assert_eq!(preview["result"]["affected"]["agents"], json!(["codex"]));
    assert_eq!(preview["result"]["operation_count"], 6);
    assert_eq!(tree(&home), initial);
    let id = preview["result"]["plan_id"].as_str().unwrap();
    assert_ne!(id, all["result"]["plan_id"].as_str().unwrap());
    assert_eq!(
        ok(&t, &["setup", "--agent", "codex"])["result"]["plan_id"],
        id
    );
    let detail = ok(&t, &["plan", "--show", id]);
    for op in detail["result"]["operations"].as_array().unwrap() {
        assert!(
            op["target"].as_str().unwrap().starts_with(
                fs::canonicalize(home.join(".codex/skills"))
                    .unwrap()
                    .to_str()
                    .unwrap()
            )
        );
    }
    let applied = ok(&t, &["apply", id]);
    assert_eq!(applied["result"]["verification"], "passed");
    assert_eq!(tree(&home.join(".claude")), claude);
    for f in [
        "SKILL.md",
        "references/routing.md",
        "references/governance.md",
        "references/mutation.md",
    ] {
        assert!(home.join(".codex/skills/skillroster").join(f).is_file());
    }
    let receipt = applied["result"]["receipt_id"].as_str().unwrap();
    let undo = ok(&t, &["undo", receipt]);
    assert_eq!(undo["result"]["verification"], "passed");
    assert_eq!(undo["result"]["rescan_required"], true);
    assert!(undo["suggested_actions"].to_string().contains("scan"));
    assert_eq!(tree(&home), initial);
    ok(&t, &["scan"]);
}
#[test]
fn current_selected_agent_ignores_unselected_modified_package() {
    let t = fixture();
    let home = t.path().join("home");
    let custom = home.join(".claude/skills/skillroster/SKILL.md");
    fs::create_dir_all(custom.parent().unwrap()).unwrap();
    let bytes = b"---\nname: skillroster\ndescription: local\n---\ncustom\n";
    fs::write(&custom, bytes).unwrap();
    let initial = tree(&home);
    ok(&t, &["scan"]);
    let preview = ok(&t, &["setup", "--agent", "codex"]);
    let applied = ok(
        &t,
        &["apply", preview["result"]["plan_id"].as_str().unwrap()],
    );
    assert_eq!(applied["result"]["verification"], "passed");
    ok(&t, &["scan"]);
    let current = ok(&t, &["setup", "--agent", "codex"]);
    assert_eq!(current["result"]["state"], "up_to_date");
    assert!(current["result"]["plan_id"].is_null());
    assert_eq!(fs::read(&custom).unwrap(), bytes);
    let undo = ok(
        &t,
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
    );
    assert_eq!(undo["result"]["verification"], "passed");
    assert_eq!(undo["result"]["rescan_required"], true);
    assert_eq!(tree(&home), initial);
    ok(&t, &["scan"]);
}
#[test]
fn selector_refuses_absent_invalid_and_preserves_modified_choice_context() {
    let t = fixture();
    let home = t.path().join("home");
    let p = home.join(".codex/skills/skillroster/SKILL.md");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        &p,
        "---\nname: skillroster\ndescription: local\n---\ncustom\n",
    )
    .unwrap();
    ok(&t, &["scan"]);
    let absent = ok(&t, &["setup", "--agent", "hermes"]);
    assert_eq!(absent["result"]["state"], "no_supported_agent");
    assert!(absent["result"]["plan_id"].is_null());
    for args in [
        &["setup", "--agent", "bogus"][..],
        &["setup", "--agent", "codex", "--agent", "codex"][..],
        &["setup", "--agent", "codex,claude-code"][..],
    ] {
        let (code, v) = run(&t, args);
        assert_eq!(code, 2);
        assert_eq!(v["error"]["code"], "invalid_cli_arguments");
    }
    let modified = ok(&t, &["setup", "--agent", "codex", "--check"]);
    assert_eq!(modified["result"]["state"], "modified_choice_required");
    assert!(modified["result"]["plan_id"].is_null());
    let actions = modified["suggested_actions"].as_array().unwrap();
    assert_eq!(actions.len(), 2);
    for action in actions {
        let argv = action["argv"].as_array().unwrap();
        let strings: Vec<&str> = argv.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(strings.windows(2).any(|a| a == ["--agent", "codex"]));
        assert!(
            strings
                .windows(2)
                .any(|a| a == ["--home", home.to_str().unwrap()])
        );
        assert!(
            strings
                .windows(2)
                .any(|a| a == ["--state-dir", t.path().join("state").to_str().unwrap()])
        );
    }
    let retain = ok(
        &t,
        &[
            "setup",
            "--agent",
            "codex",
            "--modified-choice",
            "retain-local",
        ],
    );
    assert!(retain["result"]["plan_id"].is_null());
    let adopt = ok(
        &t,
        &[
            "setup",
            "--agent",
            "codex",
            "--modified-choice",
            "adopt-current",
        ],
    );
    assert_eq!(adopt["result"]["affected"]["agents"], json!(["codex"]));
    assert_eq!(
        adopt["result"]["plan_id"],
        ok(
            &t,
            &[
                "setup",
                "--agent",
                "codex",
                "--modified-choice",
                "adopt-current"
            ]
        )["result"]["plan_id"]
    );
}
#[cfg(unix)]
#[test]
fn selected_alias_refuses_prospective_writes_while_default_deduplicates() {
    use std::os::unix::fs::symlink;
    let t = TempDir::new().unwrap();
    let home = t.path().join("home");
    let shared = home.join("shared");
    fs::create_dir_all(&shared).unwrap();
    for agent in [".codex", ".claude"] {
        fs::create_dir_all(home.join(agent)).unwrap();
        symlink(&shared, home.join(agent).join("skills")).unwrap();
    }
    let before = tree(&home);
    ok(&t, &["scan"]);
    for agent in ["codex", "claude-code"] {
        for suffix in [&["--check"][..], &[][..]] {
            let mut args = vec!["setup", "--agent", agent];
            args.extend_from_slice(suffix);
            let v = ok(&t, &args);
            assert_eq!(v["result"]["state"], "selected_scope_not_isolatable");
            assert!(v["result"]["plan_id"].is_null());
            assert_eq!(v["result"]["detected_agents"].as_array().unwrap().len(), 2);
        }
    }
    let all = ok(&t, &["setup"]);
    assert_eq!(all["result"]["state"], "preview_ready");
    assert_eq!(all["result"]["operation_count"], 6);
    assert_eq!(
        all["result"]["affected"]["agents"],
        json!(["claude-code", "codex"])
    );
    assert_eq!(tree(&home), before);
}
