#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    temp: TempDir,
    home: PathBuf,
    state: PathBuf,
    shared: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        let shared = home.join(".agents_skills");
        fs::create_dir_all(&shared).unwrap();
        for name in ["alpha", "beta", "gamma", "delta"] {
            let dir = shared.join(name);
            fs::create_dir_all(dir.join("references/nested")).unwrap();
            fs::write(dir.join("SKILL.md"), format!("---\nname: {name}\ndescription: Unique {name} capability\n---\n{name} instructions\n")).unwrap();
            fs::write(
                dir.join("references/nested/data.txt"),
                format!("{name} support bytes\n"),
            )
            .unwrap();
        }
        fs::create_dir_all(shared.join(".system/internal")).unwrap();
        fs::write(
            shared.join(".system/internal/SKILL.md"),
            "---\nname: internal\n---\nprotected\n",
        )
        .unwrap();
        let external = temp.path().join("external");
        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("SKILL.md"), "external secret bytes").unwrap();
        symlink(&external, shared.join("external")).unwrap();
        for dir in [".agents", ".codex", ".pi/agent", ".claude/skills"] {
            fs::create_dir_all(home.join(dir)).unwrap();
        }
        symlink("../.agents_skills", home.join(".agents/skills")).unwrap();
        symlink(&shared, home.join(".codex/skills")).unwrap();
        symlink("../../.agents_skills", home.join(".pi/agent/skills")).unwrap();
        symlink(shared.join("alpha"), home.join(".claude/skills/alpha")).unwrap();
        fs::write(home.join(".codex/config.toml"), "# user config\n").unwrap();
        Self {
            temp,
            home,
            state,
            shared,
        }
    }

    fn run(&self, args: &[&str], input: Option<&Value>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skillroster"));
        command
            .args(["--home"])
            .arg(&self.home)
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--json")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().unwrap();
        if let Some(value) = input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(value.to_string().as_bytes())
                .unwrap();
        }
        child.wait_with_output().unwrap()
    }

    fn ok(&self, args: &[&str], input: Option<&Value>) -> Value {
        let out = self.run(args, input);
        assert!(
            out.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["ok"], true);
        value
    }

    fn plan(&self) -> Value {
        let check = self.ok(&["check"], None);
        let mut request = check["result"]["shared_adoption"]["request_template"].clone();
        assert!(
            request.is_object(),
            "check must provide the explicit adoption request template"
        );
        assert_eq!(request["scan_id"], check["result"]["snapshot_id"]);
        request["adopt_shared_skills"]["core_budget"] = json!(4);
        self.ok(&["plan", "--stdin"], Some(&request))
    }

    fn apply(&self, plan: &Value) -> Value {
        self.ok(
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
            None,
        )
    }
}

fn link(path: &Path) -> PathBuf {
    fs::read_link(path).unwrap()
}

#[test]
fn adoption_applies_scans_loads_and_undoes_exact_shared_layout() {
    let f = Fixture::new();
    let before = f.ok(&["scan"], None);
    let plan = f.plan();
    let impact = &plan["result"]["impact"];
    assert_eq!(impact["kind"], "shared_adoption");
    assert_eq!(impact["hosted_package_count"], 4);
    assert_eq!(impact["host_configuration_files_changed"], false);
    let applied = f.apply(&plan);
    assert_eq!(applied["result"]["verification"], "passed");
    let receipt = applied["result"]["receipt_id"].as_str().unwrap();
    assert!(!receipt.is_empty());
    let after = f.ok(&["scan"], None);
    assert_eq!(
        after["result"]["default_exposure"],
        impact["after_default_exposure"]
    );
    assert_eq!(
        before["result"]["default_exposure"],
        impact["before_default_exposure"]
    );
    assert_eq!(
        after["result"]["exposed_metadata_footprint"]["exposed_metadata_bytes"],
        impact["exposed_metadata_footprint"]["after"]["exposed_metadata_bytes"]
    );
    assert!(
        after["result"]["default_exposure"].as_u64().unwrap()
            < before["result"]["default_exposure"].as_u64().unwrap()
    );
    assert!(f.shared.join(".system/internal/SKILL.md").exists());
    assert_eq!(
        link(&f.shared.join("external")),
        f.temp.path().join("external")
    );
    assert_eq!(
        fs::read(f.temp.path().join("external/SKILL.md")).unwrap(),
        b"external secret bytes"
    );
    let mut loaded = false;
    for name in ["alpha", "beta", "gamma", "delta"] {
        let found = f.ok(&["find", name, "--load", "--limit", "1"], None);
        if found["result"]["matches"][0]["roster_state"] == "on_demand" {
            assert_eq!(found["result"]["loaded_skill"]["content"]["complete"], true);
            loaded = true;
        }
    }
    assert!(
        loaded,
        "at least one adopted Skill must be On-demand and loadable"
    );
    let undone = f.ok(&["undo", receipt], None);
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(
        link(&f.home.join(".agents/skills")),
        PathBuf::from("../.agents_skills")
    );
    assert_eq!(link(&f.home.join(".codex/skills")), f.shared);
    assert_eq!(
        link(&f.home.join(".pi/agent/skills")),
        PathBuf::from("../../.agents_skills")
    );
    assert_eq!(
        link(&f.home.join(".claude/skills/alpha")),
        f.shared.join("alpha")
    );
    for name in ["alpha", "beta", "gamma", "delta"] {
        assert_eq!(
            fs::read_to_string(f.shared.join(name).join("references/nested/data.txt")).unwrap(),
            format!("{name} support bytes\n")
        );
        assert_eq!(
            fs::read_to_string(f.shared.join(name).join("SKILL.md")).unwrap(),
            format!(
                "---\nname: {name}\ndescription: Unique {name} capability\n---\n{name} instructions\n"
            )
        );
    }
    assert_eq!(
        fs::read(f.home.join(".codex/config.toml")).unwrap(),
        b"# user config\n"
    );
}

#[test]
fn adoption_rejects_budget_below_immutable_core_minimum() {
    let f = Fixture::new();
    let check = f.ok(&["check"], None);
    let request = json!({"schema_version": 1, "scan_id": check["result"]["snapshot_id"],
        "adopt_shared_skills": {"core_budget": 2, "protected_skill_ids": []}});
    let out = f.run(&["plan", "--stdin"], Some(&request));
    assert!(!out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["error"]["details"]["minimum_core_budget"], 3);
    assert_eq!(value["error"]["details"]["files_changed"], false);
    assert!(f.shared.join("alpha/SKILL.md").exists());
    assert_eq!(link(&f.home.join(".codex/skills")), f.shared);
}

#[test]
fn adoption_rejects_drift_before_any_mutation() {
    for drift in ["support", "retarget", "replace", "entry"] {
        let f = Fixture::new();
        let plan = f.plan();
        match drift {
            "support" => fs::write(
                f.shared.join("alpha/references/nested/data.txt"),
                "user edit",
            )
            .unwrap(),
            "retarget" => {
                fs::remove_file(f.home.join(".codex/skills")).unwrap();
                symlink(f.temp.path(), f.home.join(".codex/skills")).unwrap();
            }
            "replace" => {
                fs::remove_file(f.home.join(".pi/agent/skills")).unwrap();
                fs::create_dir(f.home.join(".pi/agent/skills")).unwrap();
            }
            "entry" => fs::write(f.shared.join("new-user-file"), "user data").unwrap(),
            _ => unreachable!(),
        }
        let out = f.run(
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
            None,
        );
        assert!(
            !out.status.success(),
            "{drift}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            f.shared.join("beta/SKILL.md").exists(),
            "{drift} moved package before rejecting"
        );
        assert_eq!(
            link(&f.home.join(".agents/skills")),
            PathBuf::from("../.agents_skills")
        );
        assert_eq!(
            fs::read(f.home.join(".codex/config.toml")).unwrap(),
            b"# user config\n"
        );
    }
}

#[test]
fn undo_preserves_new_dedicated_root_files_and_hosted_content_edits() {
    for edit in ["root", "hosted"] {
        let f = Fixture::new();
        let plan = f.plan();
        let applied = f.apply(&plan);
        let user_file = if edit == "root" {
            f.home.join(".codex/skills/user-created")
        } else {
            let entry = fs::read_dir(f.state.join("library"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            entry.join("references/nested/data.txt")
        };
        fs::write(&user_file, "user bytes after apply").unwrap();
        let out = f.run(
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
            None,
        );
        // A safe Undo may refuse to remove or move changed user-owned content.
        let original_or_restored = fs::read(&user_file).ok().or_else(|| {
            ["alpha", "beta", "gamma", "delta"].iter().find_map(|name| {
                let bytes =
                    fs::read(f.shared.join(name).join("references/nested/data.txt")).ok()?;
                (bytes == b"user bytes after apply").then_some(bytes)
            })
        });
        assert_eq!(
            original_or_restored.as_deref(),
            Some(b"user bytes after apply".as_slice()),
            "Undo lost user bytes: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn nested_external_category_is_preserved_without_binding_external_content() {
    let f = Fixture::new();
    fs::create_dir_all(f.shared.join("category")).unwrap();
    symlink(
        f.temp.path().join("external"),
        f.shared.join("category/external"),
    )
    .unwrap();
    let check = f.ok(&["check"], None);
    let request = json!({"schema_version": 1, "scan_id": check["result"]["snapshot_id"],
        "adopt_shared_skills": {"core_budget": 6, "protected_skill_ids": []}});
    let plan = f.ok(&["plan", "--stdin"], Some(&request));
    fs::write(
        f.temp.path().join("external/SKILL.md"),
        "changed outside user bytes",
    )
    .unwrap();
    let applied = f.apply(&plan);
    assert_eq!(applied["result"]["verification"], "passed");
    assert_eq!(
        link(&f.shared.join("category/external")),
        f.temp.path().join("external")
    );
    assert_eq!(
        fs::read(f.temp.path().join("external/SKILL.md")).unwrap(),
        b"changed outside user bytes"
    );
    f.ok(
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(
        link(&f.shared.join("category/external")),
        f.temp.path().join("external")
    );
}

#[test]
fn adoption_rejects_support_file_drift_for_same_identity_packages() {
    let f = Fixture::new();
    let identical = "---\nname: duplicate\ndescription: Identical declared source\nsource: example/duplicate\nversion: 1\n---\nShared entrypoint\n";
    for name in ["alpha", "beta"] {
        fs::write(f.shared.join(name).join("SKILL.md"), identical).unwrap();
    }
    let plan = f.plan();
    fs::write(
        f.shared.join("beta/references/nested/data.txt"),
        "edited support",
    )
    .unwrap();
    let out = f.run(
        &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        None,
    );
    assert!(!out.status.success());
    assert_eq!(
        fs::read(f.shared.join("beta/references/nested/data.txt")).unwrap(),
        b"edited support"
    );
    assert!(f.shared.join("alpha/SKILL.md").exists());
    assert_eq!(link(&f.home.join(".codex/skills")), f.shared);
}

#[test]
fn nested_known_consumer_inside_moved_package_is_rejected() {
    let f = Fixture::new();
    let nested = f.shared.join("alpha/references/nested");
    fs::write(
        nested.join("SKILL.md"),
        "---\nname: nested-consumer\n---\nNested\n",
    )
    .unwrap();
    fs::remove_file(f.home.join(".claude/skills/alpha")).unwrap();
    symlink(&nested, f.home.join(".claude/skills/nested")).unwrap();
    let check = f.ok(&["check"], None);
    let request = json!({"schema_version": 1, "scan_id": check["result"]["snapshot_id"],
        "adopt_shared_skills": {"core_budget": 4, "protected_skill_ids": []}});
    let out = f.run(&["plan", "--stdin"], Some(&request));
    assert!(!out.status.success());
    assert!(f.shared.join("alpha/SKILL.md").exists());
    assert_eq!(link(&f.home.join(".claude/skills/nested")), nested);
}

#[test]
fn adoption_refuses_disabled_future_codex_library_path() {
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    f.ok(&["scan"], None);
    let found = f.ok(&["find", "alpha", "--limit", "1"], None);
    let id = found["result"]["matches"][0]["skill_id"].as_str().unwrap();
    let suffix = hex::encode(Sha256::digest(b"alpha"));
    let target = f
        .state
        .join("library")
        .join(format!("{id}-{}", &suffix[..12]))
        .join("SKILL.md");
    fs::write(
        f.home.join(".codex/config.toml"),
        format!(
            "[[skills.config]]\npath={:?}\nenabled=false\n",
            target.to_str().unwrap()
        ),
    )
    .unwrap();
    let scan = f.ok(&["scan"], None);
    let request = json!({"schema_version": 1, "scan_id": scan["result"]["snapshot_id"],
        "adopt_shared_skills": {"core_budget": 6, "protected_skill_ids": [id]}});
    let out = f.run(&["plan", "--stdin"], Some(&request));
    assert!(
        !out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("visibility"));
    assert!(f.shared.join("alpha/SKILL.md").exists());
}

#[test]
fn adoption_does_not_select_disabled_or_unknown_claude_destination() {
    for settings in [
        r#"{"skillOverrides":{"alpha":"off"}}"#,
        r#"{"skillOverrides":{"alpha":"perhaps"}}"#,
    ] {
        let f = Fixture::new();
        fs::remove_file(f.home.join(".claude/skills/alpha")).unwrap();
        fs::remove_dir(f.home.join(".claude/skills")).unwrap();
        symlink(&f.shared, f.home.join(".claude/skills")).unwrap();
        fs::write(f.home.join(".claude/settings.json"), settings).unwrap();
        let scan = f.ok(&["scan"], None);
        let found = f.ok(&["find", "alpha", "--limit", "1"], None);
        let id = found["result"]["matches"][0]["skill_id"].as_str().unwrap();
        let request = json!({"schema_version": 1, "scan_id": scan["result"]["snapshot_id"],
            "adopt_shared_skills": {"core_budget": 6, "protected_skill_ids": [id]}});
        let out = f.run(&["plan", "--stdin"], Some(&request));
        if out.status.success() {
            let value: Value = serde_json::from_slice(&out.stdout).unwrap();
            let agents = value["result"]["selection_evidence"]["agents"]
                .as_array()
                .unwrap();
            let claude = agents
                .iter()
                .find(|agent| agent["agent"] == "claude-code")
                .unwrap();
            assert!(
                !claude["core_preview"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["skill_id"] == id)
            );
            assert_eq!(claude["core_preview_truncated"], false);
        } else {
            let value: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert!(
                value["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("visibility"),
                "{value}"
            );
        }
        assert!(f.shared.join("alpha/SKILL.md").exists());
        assert_eq!(link(&f.home.join(".claude/skills")), f.shared);
    }
}

#[test]
fn adoption_preserves_distinct_content_variants_of_the_same_identity() {
    let f = Fixture::new();
    let metadata = "---\nname: duplicate\ndescription: Shared declared identity\nsource: example/duplicate\nversion: 1\n---\n";
    for (name, content) in [("alpha", "alpha variant\n"), ("beta", "beta variant\n")] {
        fs::write(
            f.shared.join(name).join("SKILL.md"),
            format!("{metadata}{content}"),
        )
        .unwrap();
    }
    let check = f.ok(&["check"], None);
    let mut request = check["result"]["shared_adoption"]["request_template"].clone();
    assert!(request.is_object());
    request["adopt_shared_skills"]["core_budget"] = json!(50);
    let plan = f.ok(&["plan", "--stdin"], Some(&request));
    let applied = f.apply(&plan);
    assert_eq!(applied["result"]["verification"], "passed");
    let alpha = f.home.join(".codex/skills/alpha");
    let beta = f.home.join(".codex/skills/beta");
    let alpha_target = fs::canonicalize(&alpha).unwrap();
    let beta_target = fs::canonicalize(&beta).unwrap();
    assert_ne!(alpha_target, beta_target);
    let library = fs::canonicalize(f.state.join("library")).unwrap();
    assert!(alpha_target.starts_with(&library));
    assert!(beta_target.starts_with(&library));
    assert_eq!(
        fs::read_to_string(alpha.join("SKILL.md")).unwrap(),
        format!("{metadata}alpha variant\n")
    );
    assert_eq!(
        fs::read_to_string(beta.join("SKILL.md")).unwrap(),
        format!("{metadata}beta variant\n")
    );
    let undo = f.ok(
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(undo["result"]["verification"], "passed");
    for (name, content) in [("alpha", "alpha variant\n"), ("beta", "beta variant\n")] {
        assert_eq!(
            fs::read_to_string(f.shared.join(name).join("SKILL.md")).unwrap(),
            format!("{metadata}{content}")
        );
    }
}
