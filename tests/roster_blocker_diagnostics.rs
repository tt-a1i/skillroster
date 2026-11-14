use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tempfile::TempDir;

struct Fixture {
    temp: TempDir,
    home: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new(variants: usize, shared: bool) -> Self {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let codex = home.join(".codex/skills");
        let claude = home.join(".claude/skills");
        let source = temp.path().join("source");
        for root in [&codex, &claude, &source] {
            fs::create_dir_all(root).unwrap();
        }
        for index in 0..51 {
            let name = format!("aaa-{index:03}");
            write_skill(&codex, &name);
        }
        for index in 0..variants {
            let name = format!("zzz-variant-{index:02}");
            for root in [&codex, &claude, &source] {
                write_skill(root, &name);
            }
            fs::write(claude.join(&name).join(".gitignore"), "local-only\n").unwrap();
        }
        if shared {
            write_skill(&home.join(".agents/skills"), "zzz-shared-readonly");
        }
        Self { temp, home, source }
    }
    fn call(&self, args: &[&str], input: Option<&Value>) -> (bool, Value) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_skillroster"))
            .arg("--home")
            .arg(&self.home)
            .arg("--state-dir")
            .arg(self.temp.path().join("state"))
            .arg("--source-root")
            .arg(&self.source)
            .arg("--json")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(value) = input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(value.to_string().as_bytes())
                .unwrap();
        }
        let out = child.wait_with_output().unwrap();
        (
            out.status.success(),
            serde_json::from_slice(&out.stdout).unwrap(),
        )
    }
    fn ok(&self, args: &[&str], input: Option<&Value>) -> Value {
        let (ok, out) = self.call(args, input);
        assert!(ok, "{out}");
        out
    }
    fn finding(&self) -> String {
        self.ok(&["scan", "--summary"], None);
        let report = self.ok(&["report", "--findings", "--category", "exposure"], None);
        report["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["kind"] == "large_default_roster")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn source_bytes(&self) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(path: &Path, result: &mut Vec<(PathBuf, Vec<u8>)>) {
            if !path.exists() {
                return;
            }
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, result);
                } else {
                    result.push((path.clone(), fs::read(path).unwrap()));
                }
            }
        }
        let mut result = Vec::new();
        walk(&self.home, &mut result);
        walk(&self.source, &mut result);
        result.sort();
        result
    }
}
fn write_skill(root: &Path, name: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: fixture\n---\nsame payload\n"),
    )
    .unwrap();
}
const FINGERPRINT: &str = "multiple_package_fingerprints_require_explicit_preservation";
fn assert_no_plan(detail: &Value) {
    assert!(
        detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["action"] != "plan")
    );
}
fn protect(planning: &Value) -> &Value {
    planning["resolution_choices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["choice"] == "protect_blocked_skills_as_core")
        .unwrap()
}
fn original_request(id: &str) -> Value {
    json!({"schema_version":1,"finding_roster_changes":[{"finding_id":id,"core_budget":50,"protected_skill_ids":[]}]})
}

#[test]
fn fingerprint_only_reports_reason_and_preserves_safe_protection() {
    for count in [1, 6] {
        let f = Fixture::new(count, false);
        let before = f.source_bytes();
        let id = f.finding();
        let compact = f.ok(&["report", "--finding", &id], None);
        let planning = &compact["result"]["planning"];
        assert_eq!(planning["supported"], false);
        assert_eq!(
            planning["reason"],
            "package_fingerprint_variants_block_roster_change"
        );
        assert_eq!(planning["decision"], "review_package_fingerprint_variants");
        assert!(planning.get("mutation_scopes").is_none());
        assert_eq!(planning["blocked_reasons"], json!([FINGERPRINT]));
        assert_eq!(planning["blocked_skill_count"], count);
        assert_no_plan(&compact);
        if count > 5 {
            assert_eq!(planning["blocked_skills_truncated"], true);
            assert_eq!(protect(planning)["plan_request_template_available"], false);
        }
        let full = f.ok(&["report", "--finding", &id, "--full"], None);
        let p = &full["result"]["planning"];
        assert_eq!(p["blocked_reasons"], json!([FINGERPRINT]));
        assert_eq!(p["blocked_skills_truncated"], false);
        assert_no_plan(&full);
        let choice = protect(p);
        assert_eq!(choice["protected_skill_ids_complete"], true);
        assert_eq!(choice["plan_request_template_available"], true);
        let (ok, blocked) = f.call(&["plan", "--stdin"], Some(&original_request(&id)));
        assert!(!ok);
        assert_eq!(
            blocked["error"]["code"],
            "roster_package_fingerprint_variants"
        );
        assert_eq!(blocked["error"]["details"]["files_changed"], false);
        let ready = f.ok(&["plan", "--stdin"], Some(&choice["plan_request_template"]));
        assert_eq!(ready["result"]["state"], "ready");
        assert_eq!(ready["result"]["confirmation_required"], true);
        assert_eq!(f.source_bytes(), before);
    }
}

#[test]
fn mixed_shared_read_only_preserves_immutable_core_and_reports_fingerprint() {
    let f = Fixture::new(1, true);
    let before = f.source_bytes();
    let id = f.finding();
    let detail = f.ok(&["report", "--finding", &id, "--full"], None);
    let p = &detail["result"]["planning"];
    assert_eq!(
        p["reason"],
        "package_fingerprint_variants_block_roster_change"
    );
    assert!(p.get("mutation_scopes").is_none());
    assert_eq!(p["blocked_reasons"], json!([FINGERPRINT]));
    assert_no_plan(&detail);
    let choice = protect(p);
    assert_eq!(choice["plan_request_template_available"], true);
    let ready = f.ok(&["plan", "--stdin"], Some(&choice["plan_request_template"]));
    assert_eq!(ready["result"]["state"], "ready");
    assert_eq!(f.source_bytes(), before);
}

#[cfg(unix)]
#[test]
fn mixed_untrusted_preserves_external_core_without_read_permission() {
    let f = Fixture::new(1, false);
    let external = f.temp.path().join("external");
    write_skill(&external, "zzz-outside");
    std::os::unix::fs::symlink(
        external.join("zzz-outside"),
        f.home.join(".codex/skills/zzz-outside"),
    )
    .unwrap();
    let original_external = fs::read(external.join("zzz-outside/SKILL.md")).unwrap();
    let id = f.finding();
    let detail = f.ok(&["report", "--finding", &id, "--full"], None);
    let p = &detail["result"]["planning"];
    assert_eq!(
        p["reason"],
        "package_fingerprint_variants_block_roster_change"
    );
    assert_eq!(p["blocked_reasons"], json!([FINGERPRINT]));
    assert!(p.get("source_confirmation_finding").is_none());
    let choice = protect(p);
    assert_eq!(choice["plan_request_template_available"], true);
    let ready = f.ok(&["plan", "--stdin"], Some(&choice["plan_request_template"]));
    assert_eq!(ready["result"]["state"], "ready");
    assert_no_plan(&detail);
    assert_eq!(
        fs::read(external.join("zzz-outside/SKILL.md")).unwrap(),
        original_external
    );
}

#[test]
fn an_immutable_codex_roster_does_not_hide_a_safe_hermes_plan() {
    let fixture = Fixture::new(0, false);
    for index in 0..60 {
        write_skill(
            &fixture.home.join(".agents/skills"),
            &format!("shared-{index:03}"),
        );
        write_skill(
            &fixture.home.join(".hermes/skills"),
            &format!("hermes-{index:03}"),
        );
    }
    let before = fixture.source_bytes();
    fixture.ok(&["scan", "--summary"], None);
    let summary = fixture.ok(&["report"], None);
    let available = summary["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["actionability"] == "plan_available")
        .expect("first screen includes a safe Plan");
    assert_eq!(available["affected_agents"], json!(["hermes"]));
    let full = fixture.ok(&["report", "--full"], None);
    assert!(
        full["result"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .take(3)
            .any(|finding| finding["actionability"] == "plan_available")
    );
    let codex = full["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["kind"] == "large_default_roster"
                && finding["affected_agents"] == json!(["codex"])
        })
        .unwrap();
    let blocked = fixture.ok(
        &["report", "--finding", codex["id"].as_str().unwrap()],
        None,
    );
    assert_eq!(blocked["result"]["planning"]["minimum_core_budget"], 60);
    assert_eq!(blocked["result"]["planning"]["supported"], false);
    let detail = fixture.ok(
        &["report", "--finding", available["id"].as_str().unwrap()],
        None,
    );
    let ready = fixture.ok(
        &["plan", "--stdin"],
        Some(&detail["result"]["planning"]["plan_request_template"]),
    );
    assert_eq!(ready["result"]["state"], "ready");
    assert_eq!(ready["result"]["confirmation_required"], true);
    assert_eq!(fixture.source_bytes(), before);
}

#[cfg(unix)]
#[test]
fn hermes_preserves_an_unapproved_directory_branch_while_right_sizing_mutable_skills() {
    let (fixture, root, outside, branch) = hermes_external_branch_fixture();
    let scan = fixture.ok(&["check"], None);
    assert!(!scan.to_string().contains("SECRET-UNREAD-SKILL"));
    let report = fixture.ok(&["report", "--full"], None);
    assert!(
        report["result"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["kind"] == "configured_roots_bounded")
    );
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["kind"] == "large_default_roster"
                && finding["affected_agents"] == json!(["hermes"])
        })
        .unwrap();
    let detail = fixture.ok(
        &["report", "--finding", finding["id"].as_str().unwrap()],
        None,
    );
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], true, "{planning}");
    let plan = fixture.ok(
        &["plan", "--stdin"],
        Some(&planning["plan_request_template"]),
    );
    assert_eq!(plan["result"]["state"], "ready");
    assert_eq!(plan["result"]["confirmation_required"], true);
    assert_eq!(plan["result"]["impact"]["before_default_exposure"], 112);
    assert_eq!(plan["result"]["impact"]["after_default_exposure"], 101);
    assert!(!plan.to_string().contains("SECRET-UNREAD-SKILL"));
    let external_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["kind"] == "escaping_link_source_confirmation")
        .unwrap()["affected_skill_ids"][0]
        .as_str()
        .unwrap();
    let full_plan = fixture.ok(
        &[
            "plan",
            "--show",
            plan["result"]["plan_id"].as_str().unwrap(),
        ],
        None,
    );
    assert!(
        full_plan["result"]["roster_changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["skill_id"] == external_id && change["state"] == "core")
    );
    let applied = fixture.ok(
        &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(applied["result"]["verification"], "passed");
    assert!(branch.is_symlink());
    assert!(outside.join("SECRET-UNREAD-SKILL/SKILL.md").is_file());
    let post = fixture.ok(&["scan", "--summary"], None);
    assert!(!post.to_string().contains("SECRET-UNREAD-SKILL"));
    fixture.ok(
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 61);
    assert!(branch.is_symlink());
}

#[cfg(unix)]
fn hermes_external_branch_fixture() -> (Fixture, PathBuf, PathBuf, PathBuf) {
    let fixture = Fixture::new(0, false);
    let root = fixture.home.join(".hermes/skills");
    for index in 0..60 {
        write_skill(&root, &format!("hermes-{index:03}"));
    }
    let outside = fixture.temp.path().join("unapproved-branch");
    write_skill(&outside, "SECRET-UNREAD-SKILL");
    let branch = root.join("zzz-external");
    std::os::unix::fs::symlink(&outside, &branch).unwrap();
    (fixture, root, outside, branch)
}

#[cfg(unix)]
#[test]
fn hermes_preserved_branch_retarget_blocks_apply_before_mutation() {
    let (fixture, root, _outside, branch) = hermes_external_branch_fixture();
    fixture.ok(&["check"], None);
    let report = fixture.ok(&["report", "--full"], None);
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["kind"] == "large_default_roster"
                && finding["affected_agents"] == json!(["hermes"])
        })
        .unwrap();
    let detail = fixture.ok(
        &["report", "--finding", finding["id"].as_str().unwrap()],
        None,
    );
    let plan = fixture.ok(
        &["plan", "--stdin"],
        Some(&detail["result"]["planning"]["plan_request_template"]),
    );
    let replacement = fixture.temp.path().join("replacement-branch");
    fs::create_dir(&replacement).unwrap();
    fs::remove_file(&branch).unwrap();
    std::os::unix::fs::symlink(&replacement, &branch).unwrap();
    let (ok, rejected) = fixture.call(
        &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        None,
    );
    assert!(!ok);
    assert_eq!(
        rejected["error"]["code"],
        "roster_skill_root_discovery_incomplete"
    );
    assert_eq!(fs::read_dir(root).unwrap().count(), 61);
    assert_eq!(fs::read_link(&branch).unwrap(), replacement);
}

#[test]
fn hermes_keeps_unknown_native_visibility_core_without_authorizing_any_change_to_it() {
    let fixture = Fixture::new(0, false);
    let root = fixture.home.join(".hermes/skills");
    for index in 0..50 {
        write_skill(&root, &format!("known-{index:03}"));
    }
    let unknown = root.join("zzz-unknown");
    fs::create_dir_all(&unknown).unwrap();
    let original =
        "---\nname: zzz-unknown\nplatforms: [android]\n---\nunknown runtime restriction\n";
    fs::write(unknown.join("SKILL.md"), original).unwrap();
    fixture.ok(&["check"], None);
    let report = fixture.ok(&["report", "--full"], None);
    assert_eq!(report["result"]["hermes_visibility"]["unknown_count"], 1);
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["kind"] == "large_default_roster"
                && finding["affected_agents"] == json!(["hermes"])
        })
        .unwrap();
    let detail = fixture.ok(
        &["report", "--finding", finding["id"].as_str().unwrap()],
        None,
    );
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], true, "{planning}");
    let selection = planning["agents"][0]["core_preview"]
        .as_array()
        .unwrap()
        .iter()
        .find(|selection| selection["reason"] == "native_visibility_unknown_retained")
        .unwrap();
    let unknown_id = selection["skill_id"].as_str().unwrap();
    let plan = fixture.ok(
        &["plan", "--stdin"],
        Some(&planning["plan_request_template"]),
    );
    let full = fixture.ok(
        &[
            "plan",
            "--show",
            plan["result"]["plan_id"].as_str().unwrap(),
        ],
        None,
    );
    assert!(
        full["result"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|operation| !operation.to_string().contains("zzz-unknown"))
    );
    for (agent, state) in [("hermes", "on_demand"), ("codex", "core")] {
        let request = json!({"schema_version": 1, "scan_id": report["result"]["snapshot_id"], "evidence_ids": finding["evidence_ids"], "roster_changes": [{"agent": agent, "skill_id": unknown_id, "state": state}]});
        let (ok, rejected) = fixture.call(&["plan", "--stdin"], Some(&request));
        assert!(!ok, "{rejected}");
        assert_eq!(
            rejected["error"]["code"], "native_visibility_blocks_roster_change",
            "{rejected}"
        );
    }
    let applied = fixture.ok(
        &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(applied["result"]["verification"], "passed");
    assert_eq!(
        fs::read_to_string(unknown.join("SKILL.md")).unwrap(),
        original
    );
    assert!(!unknown.is_symlink());
    fixture.ok(
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(
        fs::read_to_string(unknown.join("SKILL.md")).unwrap(),
        original
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 51);
}

#[test]
fn hermes_plan_does_not_mutate_unrelated_shared_codex_copy_with_unknown_visibility() {
    let fixture = Fixture::new(0, false);
    let root = fixture.home.join(".hermes/skills");
    for index in 0..51 {
        write_skill(&root, &format!("aaa-hermes-{index:03}"));
    }
    let name = "zzz-shared-copy";
    write_skill(&root, name);
    let shared = fixture.home.join(".agents/skills");
    write_skill(&shared, name);
    let shared_file = shared.join(name).join("SKILL.md");
    let original = fs::read(&shared_file).unwrap();
    assert_eq!(
        fs::read(root.join(name).join("SKILL.md")).unwrap(),
        original
    );
    let config = fixture.home.join(".codex/config.toml");
    let settings = b"[[skills.config]\ninvalid toml\n";
    fs::write(&config, settings).unwrap();

    fixture.ok(&["check"], None);
    let report = fixture.ok(&["report", "--full"], None);
    assert!(
        report["result"]["codex_visibility"]["unknown_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["kind"] == "large_default_roster"
                && finding["affected_agents"] == json!(["hermes"])
        })
        .unwrap();
    let detail = fixture.ok(
        &["report", "--finding", finding["id"].as_str().unwrap()],
        None,
    );
    let plan = fixture.ok(
        &["plan", "--stdin"],
        Some(&detail["result"]["planning"]["plan_request_template"]),
    );
    assert_eq!(plan["result"]["state"], "ready");
    let applied = fixture.ok(
        &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(applied["result"]["verification"], "passed");
    assert!(!root.join(name).exists());
    assert!(fs::read_dir(&root).unwrap().count() < 52);
    assert_eq!(fs::read(&shared_file).unwrap(), original);
    assert!(!shared.join(name).is_symlink());
    assert_eq!(fs::read(&config).unwrap(), settings);
    let undone = fixture.ok(
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(
        fs::read(root.join(name).join("SKILL.md")).unwrap(),
        original
    );
    assert_eq!(fs::read(&shared_file).unwrap(), original);
    assert_eq!(fs::read(&config).unwrap(), settings);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 52);
}
