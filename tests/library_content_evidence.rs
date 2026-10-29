use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    home: PathBuf,
    state: PathBuf,
    codex: PathBuf,
    claude: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        let codex = home.join(".codex/skills/shared");
        let claude = home.join(".claude/skills/shared");
        for p in [&codex, &claude] {
            fs::create_dir_all(p.join("references")).unwrap();
            fs::write(
                p.join("SKILL.md"),
                "---\nname: shared\ndescription: shared fixture\n---\nbody\n",
            )
            .unwrap();
            fs::write(p.join("references/guide.txt"), "original support\n").unwrap();
        }
        fs::write(home.join("sentinel"), "preserve\n").unwrap();
        Self {
            _temp: temp,
            home,
            state,
            codex,
            claude,
        }
    }
    fn call(&self, args: &[&str], input: Option<&Value>) -> (bool, Value) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_skillroster"))
            .arg("--home")
            .arg(&self.home)
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--json")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(v) = input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(v.to_string().as_bytes())
                .unwrap();
        }
        let out = child.wait_with_output().unwrap();
        (
            out.status.success(),
            serde_json::from_slice(&out.stdout).unwrap(),
        )
    }
    fn ok(&self, args: &[&str], input: Option<&Value>) -> Value {
        let (ok, v) = self.call(args, input);
        assert!(ok, "{v}");
        v
    }
    fn request(&self, state_name: &str, raw: bool) -> Value {
        self.ok(&["scan", "--summary"], None);
        let list = self.ok(&["report", "--findings", "--category", "overlap"], None);
        let id = list["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["kind"] == "exact_duplicate_placements")
            .unwrap()["id"]
            .as_str()
            .unwrap();
        let d = self.ok(&["report", "--finding", id, "--full"], None);
        let result = &d["result"];
        let p = &result["planning"];
        let canonical = p["canonical_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["agent"] == "codex")
            .unwrap()["placement_id"]
            .clone();
        if raw {
            json!({"schema_version":1,"scan_id":p["snapshot_id"],"evidence_ids":[result["primary_evidence_id"]],"library_changes":[{"skill_id":result["affected_skill_ids"][0],"canonical_placement_id":canonical,"placement_ids":result["affected_placement_ids"],"requested_state":state_name}]})
        } else {
            json!({"schema_version":1,"finding_library_changes":[{"finding_id":id,"canonical_placement_id":canonical,"requested_state":state_name}]})
        }
    }
    fn pending(&self) -> u64 {
        self.ok(&["status"], None)["result"]["pending_plan_count"]
            .as_u64()
            .expect("pending Plan count")
    }
    fn edit(&self, path: &Path) {
        let mut s = fs::read_to_string(path).unwrap();
        s.push_str("user edit\n");
        fs::write(path, s).unwrap();
    }
    fn tree(&self) -> Vec<(PathBuf, &'static str, Vec<u8>)> {
        fn walk(p: &Path, out: &mut Vec<(PathBuf, &'static str, Vec<u8>)>) {
            for e in fs::read_dir(p).unwrap() {
                let p = e.unwrap().path();
                let m = fs::symlink_metadata(&p).unwrap();
                if m.is_symlink() {
                    out.push((
                        p.clone(),
                        "symlink",
                        fs::read_link(&p)
                            .unwrap()
                            .as_os_str()
                            .as_encoded_bytes()
                            .to_vec(),
                    ));
                } else if m.is_dir() {
                    out.push((p.clone(), "directory", Vec::new()));
                    walk(&p, out);
                } else {
                    out.push((p.clone(), "file", fs::read(&p).unwrap()));
                }
            }
        }
        let mut out = vec![];
        walk(&self.home, &mut out);
        out.sort();
        out
    }
    fn assert_rescan(&self, v: &Value, entrypoint: &Path, changed: bool) {
        assert_eq!(v["error"]["code"], "library_content_rescan_required");
        assert_eq!(
            v["error"]["details"]["reason"],
            if changed {
                "snapshot_package_content_changed"
            } else {
                "snapshot_package_identity_unavailable"
            }
        );
        assert_eq!(v["error"]["retryable"], true);
        assert_eq!(v["error"]["details"]["next_action"], "scan");
        assert_eq!(v["error"]["details"]["retry_after"], "fresh_scan");
        let expected = v["error"]["details"]["expected_digest"].as_str().unwrap();
        assert!(!expected.is_empty());
        if changed {
            let actual = v["error"]["details"]["actual_digest"].as_str().unwrap();
            assert!(!actual.is_empty());
            assert_ne!(expected, actual);
        } else {
            assert!(v["error"]["details"]["actual_digest"].is_null());
        }
        assert!(
            v["error"]["paths"]
                .as_array()
                .unwrap()
                .contains(&json!(entrypoint))
        );
        for field in ["snapshot_id", "skill_id", "placement_id"] {
            let id = v["error"]["details"][field].as_str().unwrap();
            assert!(!id.is_empty());
            assert!(
                v["error"]["relevant_ids"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(id))
            );
        }
        let actions = v["suggested_actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["action"], "scan");
        assert_eq!(actions[0]["mutates"], false);
        assert_eq!(actions[0]["requires_confirmation"], false);
        let argv = actions[0]["argv"].as_array().unwrap();
        assert_eq!(argv[0], env!("CARGO_BIN_EXE_skillroster"));
        assert!(
            argv.windows(2)
                .any(|pair| pair == [json!("--home"), json!(self.home)])
        );
        assert!(
            argv.windows(2)
                .any(|pair| pair == [json!("--state-dir"), json!(self.state)])
        );
        assert!(argv.ends_with(&[json!("scan"), json!("--summary"), json!("--json")]));
    }
}

#[test]
fn old_finding_and_raw_request_refuse_changed_package_content() {
    for (hosted, raw, noncanonical, support) in [
        (false, false, false, false),
        (true, false, false, true),
        (false, false, true, true),
        (true, true, false, false),
    ] {
        let f = Fixture::new();
        let req = f.request(if hosted { "hosted" } else { "managed" }, raw);
        let pending = f.pending();
        let dir = if noncanonical { &f.claude } else { &f.codex };
        let path = if support {
            dir.join("references/guide.txt")
        } else {
            dir.join("SKILL.md")
        };
        f.edit(&path);
        let before = f.tree();
        let (ok, v) = f.call(&["plan", "--stdin"], Some(&req));
        assert!(!ok, "{v}");
        f.assert_rescan(&v, &dir.join("SKILL.md"), true);
        assert_eq!(f.pending(), pending);
        assert_eq!(f.tree(), before);
    }
}

#[test]
fn apply_refuses_content_change_before_mutation_or_receipt() {
    for state_name in ["managed", "hosted"] {
        let f = Fixture::new();
        let req = f.request(state_name, false);
        let plan = f.ok(&["plan", "--stdin"], Some(&req));
        assert_eq!(plan["result"]["state"], "ready");
        let id = plan["result"]["plan_id"].as_str().unwrap();
        f.edit(&f.codex.join("references/guide.txt"));
        let before = f.tree();
        let status = f.ok(&["status"], None);
        let (ok, v) = f.call(&["apply", id], None);
        assert!(!ok, "{v}");
        f.assert_rescan(&v, &f.codex.join("SKILL.md"), true);
        let after = f.ok(&["status"], None);
        assert_eq!(
            after["result"]
                .get("last_receipt")
                .expect("last receipt field"),
            status["result"]
                .get("last_receipt")
                .expect("last receipt field")
        );
        assert_eq!(f.tree(), before);
    }
}

#[test]
fn preserved_edit_after_undo_cannot_authorize_old_duplicate_finding() {
    let f = Fixture::new();
    let req = f.request("managed", false);
    let p = f.ok(&["plan", "--stdin"], Some(&req));
    let applied = f.ok(&["apply", p["result"]["plan_id"].as_str().unwrap()], None);
    f.edit(&f.codex.join("SKILL.md"));
    let undo = f.ok(
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(undo["result"]["verification"], "passed");
    let before = f.tree();
    let pending = f.pending();
    let (ok, v) = f.call(&["plan", "--stdin"], Some(&req));
    assert!(!ok, "{v}");
    f.assert_rescan(&v, &f.codex.join("SKILL.md"), true);
    assert_eq!(f.pending(), pending);
    assert_eq!(f.tree(), before);
    assert!(
        fs::read_to_string(f.codex.join("SKILL.md"))
            .unwrap()
            .contains("user edit")
    );
    assert!(
        !fs::read_to_string(f.claude.join("SKILL.md"))
            .unwrap()
            .contains("user edit")
    );
}

#[test]
fn complete_snapshot_package_that_becomes_bounded_requires_rescan() {
    let f = Fixture::new();
    let req = f.request("managed", false);
    let pending = f.pending();
    let support = f.codex.join("references/guide.txt");
    fs::File::create(&support)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    let before = f.tree();
    let (ok, value) = f.call(&["plan", "--stdin"], Some(&req));
    assert!(!ok, "{value}");
    f.assert_rescan(&value, &f.codex.join("SKILL.md"), false);
    assert_eq!(f.pending(), pending);
    assert_eq!(f.tree(), before);
}
