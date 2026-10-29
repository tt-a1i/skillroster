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
fn mixed_shared_read_only_and_fingerprint_report_both_reasons() {
    let f = Fixture::new(1, true);
    let before = f.source_bytes();
    let id = f.finding();
    let detail = f.ok(&["report", "--finding", &id, "--full"], None);
    let p = &detail["result"]["planning"];
    assert_eq!(p["reason"], "mutation_scope_blocks_roster_change");
    assert!(
        p["mutation_scopes"]
            .as_array()
            .unwrap()
            .contains(&json!("shared_read_only"))
    );
    assert_eq!(
        p["blocked_reasons"],
        json!([FINGERPRINT, "non_mutable_placement_blocks_mutation"])
    );
    assert_no_plan(&detail);
    let choice = protect(p);
    assert_eq!(choice["plan_request_template_available"], true);
    let ready = f.ok(&["plan", "--stdin"], Some(&choice["plan_request_template"]));
    assert_eq!(ready["result"]["state"], "ready");
    assert_eq!(f.source_bytes(), before);
}

#[cfg(unix)]
#[test]
fn mixed_untrusted_and_fingerprint_keeps_narrow_source_confirmation() {
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
    assert_eq!(p["reason"], "trusted_canonical_sources_required");
    assert_eq!(
        p["blocked_reasons"],
        json!([FINGERPRINT, "untrusted_external_placement_blocks_mutation"])
    );
    let blocked = p["blocked_changes"].as_array().unwrap();
    let untrusted = blocked
        .iter()
        .find(|v| v["reason"] == "untrusted_external_placement_blocks_mutation")
        .unwrap();
    let fingerprints = blocked.iter().find(|v| v["reason"] == FINGERPRINT).unwrap();
    assert_ne!(untrusted["skill_id"], fingerprints["skill_id"]);
    let reference = &p["source_confirmation_finding"];
    assert_eq!(reference["state"], "available");
    let finding_id = reference["finding_id"].as_str().unwrap();
    let source_detail = f.ok(&["report", "--finding", finding_id, "--full"], None);
    let encoded = source_detail.to_string();
    assert!(encoded.contains(untrusted["skill_id"].as_str().unwrap()));
    assert!(encoded.contains(external.join("zzz-outside").to_str().unwrap()));
    assert!(!encoded.contains(fingerprints["skill_id"].as_str().unwrap()));
    assert_no_plan(&detail);
    assert_eq!(
        fs::read(external.join("zzz-outside/SKILL.md")).unwrap(),
        original_external
    );
}
