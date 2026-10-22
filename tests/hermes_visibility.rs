#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        Self(TempDir::new().unwrap())
    }

    fn home(&self) -> PathBuf {
        self.0.path().join("home")
    }

    fn root(&self) -> PathBuf {
        self.home().join(".hermes/skills")
    }

    fn package(&self, path: impl AsRef<Path>, name: &str) -> PathBuf {
        let directory = self.root().join(path);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {name} capability\n---\n{name}_PRIVATE_BODY\n"
            ),
        )
        .unwrap();
        directory
    }

    fn command(&self, args: &[&str], input: Option<&Value>) -> Value {
        let mut child = Command::new(env!("CARGO_BIN_EXE_skillroster"))
            .args([
                "--home",
                self.home().to_str().unwrap(),
                "--state-dir",
                self.0.path().join("state").to_str().unwrap(),
                "--json",
            ])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(input) = input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.to_string().as_bytes())
                .unwrap();
        }
        let output = child.wait_with_output().unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{args:?}: {error}; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        });
        assert_eq!(output.status.success(), value["ok"] == true, "{value}");
        value
    }

    fn ok(&self, args: &[&str]) -> Value {
        let value = self.command(args, None);
        assert_eq!(value["ok"], true, "{value}");
        value
    }

    fn database(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.0.path().join("state/skillroster.db")).unwrap()
    }

    fn snapshot(&self) -> Value {
        let raw: String = self
            .database()
            .query_row(
                "SELECT payload_json FROM scan_payloads ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    fn roster_proposal(&self, scan: &Value, skill_id: &str, agent: &str, state: &str) -> Value {
        let snapshot_id = scan["result"]["snapshot_id"].as_str().unwrap();
        let evidence: String = self.database().query_row(
            "SELECT id FROM evidence WHERE scan_id=?1 AND subject_type='skill' AND subject_id=?2 ORDER BY id LIMIT 1",
            rusqlite::params![snapshot_id, skill_id], |row| row.get(0),
        ).unwrap();
        json!({"schema_version":1,"scan_id":snapshot_id,"evidence_ids":[evidence],
            "roster_changes":[{"agent":agent,"skill_id":skill_id,"state":state}]})
    }
}

impl Fixture {
    fn config(&self, text: &str) {
        let path = self.home().join(".hermes/config.yaml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn named_placement(&self, name: &str) -> Value {
        let snapshot = self.snapshot();
        snapshot["placements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|placement| {
                placement["entrypoint"] == json!(self.root().join(name).join("SKILL.md"))
            })
            .unwrap()
            .clone()
    }

    fn visibility(&self, placement: &Value) -> Value {
        self.snapshot()["hermes_visibility"]["placements"][placement["id"].as_str().unwrap()]
            .clone()
    }

    fn assert_rescan(&self, args: &[&str], reason: &str) {
        let blocked = self.command(args, None);
        assert_eq!(blocked["ok"], false, "{args:?}: {blocked}");
        assert_eq!(
            blocked["error"]["code"], "content_identity_rescan_required",
            "{blocked}"
        );
        assert_eq!(blocked["error"]["details"]["reason"], reason, "{blocked}");
        let actions = blocked["suggested_actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1, "{blocked}");
        assert!(
            actions[0]["argv"]
                .as_array()
                .unwrap()
                .contains(&json!("scan"))
        );
        assert!(!blocked.to_string().contains("PRIVATE_BODY"));
    }
}

fn other_desktop_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "linux"
    } else {
        "windows"
    }
}

#[test]
fn global_and_os_disabled_remain_inventory_and_explicitly_loadable_with_honest_summary() {
    let fixture = Fixture::new();
    fixture.package("blocked", "blocked");
    let os = fixture.package("otheros", "otheros");
    fs::write(
        os.join("SKILL.md"),
        format!(
            "---\nname: otheros\nplatforms: [{}]\n---\notheros_PRIVATE_BODY\n",
            other_desktop_platform()
        ),
    )
    .unwrap();
    fixture.package("active", "active");
    fixture.package("channel", "channel");
    fixture
        .config("skills:\n  disabled: [blocked]\n  platform_disabled:\n    telegram: [channel]\n");
    let scan = fixture.ok(&["scan"]);
    assert_eq!(scan["result"]["placement_count"], 4);
    let summary = &scan["result"]["hermes_visibility"];
    assert_eq!(summary["state_counts"]["off"], 2, "{summary}");
    assert_eq!(summary["state_counts"]["on"], 1);
    assert_eq!(summary["state_counts"]["unknown"], 1);
    assert_eq!(summary["unknown_count"], 1);
    assert_eq!(summary["host_config_validity"], "unverified");
    assert_eq!(summary["session_catalog_observed"], false);
    for (name, visibility, exposed) in [
        ("blocked", "off", false),
        ("otheros", "off", false),
        ("active", "on", true),
        ("channel", "unknown", true),
    ] {
        let placement = fixture.named_placement(name);
        assert_eq!(fixture.visibility(&placement), visibility, "{placement}");
        assert_eq!(placement["default_exposed"], exposed);
        let loaded = fixture.ok(&["find", name, "--load", "--limit", "1"]);
        assert!(
            loaded["result"]["loaded_skill"]["content"]["text"]
                .as_str()
                .unwrap()
                .contains("PRIVATE_BODY")
        );
    }
    let report = fixture.ok(&["report", "--full"]);
    assert_eq!(report["result"]["default_exposure"], 2);
    assert_eq!(&report["result"]["hermes_visibility"], summary);
}

#[test]
fn restricted_core_is_blocked_but_ordinary_cross_agent_core_can_be_planned() {
    let fixture = Fixture::new();
    fixture.package("blocked", "blocked");
    fixture.package("channel", "channel");
    let source = fixture.home().join(".codex/skills/ordinary");
    fixture.package(&source, "ordinary");
    fixture
        .config("skills:\n  disabled: [blocked]\n  platform_disabled:\n    telegram: [channel]\n");
    let scan = fixture.ok(&["scan"]);
    for name in ["blocked", "channel"] {
        let placement = fixture.named_placement(name);
        let proposal = fixture.roster_proposal(
            &scan,
            placement["skill_id"].as_str().unwrap(),
            "hermes",
            "core",
        );
        let blocked = fixture.command(&["plan", "--stdin"], Some(&proposal));
        assert_eq!(blocked["ok"], false, "{blocked}");
        assert_eq!(
            blocked["error"]["code"],
            "native_visibility_blocks_roster_change"
        );
    }
    let found = fixture.ok(&["find", "ordinary", "--limit", "1"]);
    let proposal = fixture.roster_proposal(
        &scan,
        found["result"]["matches"][0]["skill_id"].as_str().unwrap(),
        "hermes",
        "core",
    );
    let planned = fixture.command(&["plan", "--stdin"], Some(&proposal));
    assert_eq!(planned["ok"], true, "{planned}");
    assert!(planned["result"]["plan_id"].is_string());
    assert!(!fixture.root().join("ordinary").exists());
    let original = fs::read(source.join("SKILL.md")).unwrap();
    let applied = fixture.ok(&["apply", planned["result"]["plan_id"].as_str().unwrap()]);
    assert_eq!(
        fs::read(fixture.root().join("ordinary/SKILL.md")).unwrap(),
        original
    );
    let undone = fixture.ok(&["undo", applied["result"]["receipt_id"].as_str().unwrap()]);
    assert_eq!(undone["result"]["verification"], "passed");
    assert!(!fixture.root().join("ordinary").exists());
    assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), original);
}

#[test]
fn configuration_drift_and_legacy_projection_require_rescan_without_losing_ready_plan() {
    for scenario in ["settings", "legacy", "host", "root"] {
        let fixture = Fixture::new();
        let package = fixture.package("active", "active");
        let original = fs::read(package.join("SKILL.md")).unwrap();
        fixture.config("skills: {}\n");
        let scan = fixture.ok(&["scan"]);
        fixture.ok(&["report"]);
        let placement = fixture.named_placement("active");
        let proposal = fixture.roster_proposal(
            &scan,
            placement["skill_id"].as_str().unwrap(),
            "hermes",
            "on_demand",
        );
        let plan = fixture.command(&["plan", "--stdin"], Some(&proposal));
        assert_eq!(plan["ok"], true, "{plan}");
        let plan_id = plan["result"]["plan_id"].as_str().unwrap();
        let reason = match scenario {
            "legacy" => {
                fixture.database().execute("UPDATE scan_payloads SET payload_json=json_remove(payload_json,'$.hermes_visibility')", []).unwrap();
                "legacy_hermes_visibility_requires_rescan"
            }
            "host" => {
                fixture.database().execute("UPDATE scan_payloads SET payload_json=json_set(payload_json,'$.hermes_visibility.host_platform','different-host-os')", []).unwrap();
                "hermes_visibility_host_platform_changed"
            }
            "root" => {
                fs::rename(fixture.root(), fixture.0.path().join("old-root")).unwrap();
                fs::create_dir_all(&package).unwrap();
                fs::write(package.join("SKILL.md"), &original).unwrap();
                "hermes_visibility_root_identity_changed"
            }
            _ => {
                fixture.config("skills:\n  disabled: [active]\n");
                "hermes_visibility_settings_changed"
            }
        };
        for args in [
            vec!["find", "active", "--load"],
            vec!["report"],
            vec!["setup", "--check"],
            vec!["apply", plan_id],
        ] {
            fixture.assert_rescan(&args, reason);
        }
        let blocked_plan = fixture.command(&["plan", "--stdin"], Some(&proposal));
        assert_eq!(blocked_plan["ok"], false, "{blocked_plan}");
        assert_eq!(blocked_plan["error"]["details"]["reason"], reason);
        assert_eq!(
            blocked_plan["suggested_actions"].as_array().unwrap().len(),
            1
        );
        let inspected = fixture.ok(&["plan", "--show", plan_id]);
        assert_eq!(inspected["result"]["state"], "ready");
        let status: String = fixture
            .database()
            .query_row("SELECT status FROM plans WHERE id=?1", [plan_id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(status, "ready");
        assert_eq!(fs::read(package.join("SKILL.md")).unwrap(), original);
        fixture.ok(&["scan"]);
        fixture.ok(&["find", "active", "--load"]);
        fixture.ok(&["report"]);
    }
}

#[test]
fn malformed_or_symlinked_configuration_is_unknown_and_never_exposes_config_contents() {
    for symlinked in [false, true] {
        let fixture = Fixture::new();
        fixture.package("active", "active");
        let secret = "PRIVATE_CONFIG_SECRET_DO_NOT_READ";
        if symlinked {
            let target = fixture.0.path().join("private-config");
            fs::write(
                &target,
                format!("skills:\n  disabled: [active]\nsecret: {secret}\n"),
            )
            .unwrap();
            symlink(&target, fixture.home().join(".hermes/config.yaml")).unwrap();
        } else {
            fixture.config(&format!("skills: [broken\n{secret}\n"));
        }
        let scan = fixture.ok(&["scan"]);
        assert_eq!(
            scan["result"]["hermes_visibility"]["state_counts"]["unknown"], 1,
            "{scan}"
        );
        let placement = fixture.named_placement("active");
        if symlinked {
            assert_eq!(
                fixture.snapshot()["hermes_visibility"]["settings_files"]
                    [fixture.home().join(".hermes/config.yaml").to_str().unwrap()],
                "unavailable"
            );
        }
        assert_eq!(fixture.visibility(&placement), "unknown");
        assert_eq!(placement["default_exposed"], true);
        let report = fixture.ok(&["report"]);
        for value in [&scan, &report, &fixture.snapshot()] {
            assert!(!value.to_string().contains(secret));
        }
        let proposal = fixture.roster_proposal(
            &scan,
            placement["skill_id"].as_str().unwrap(),
            "hermes",
            "core",
        );
        let blocked = fixture.command(&["plan", "--stdin"], Some(&proposal));
        assert_eq!(blocked["ok"], false, "{blocked}");
        assert_eq!(
            blocked["error"]["code"],
            "native_visibility_blocks_roster_change"
        );
        assert!(!blocked.to_string().contains(secret));
    }
}

#[test]
fn source_update_cannot_reenable_a_disabled_hermes_package_by_renaming() {
    let fixture = Fixture::new();
    let package = fixture.package("blocked", "blocked");
    let original =
        "---\nname: blocked\nsource: github:example/source-skill\nversion: v1\n---\nold\n";
    let upstream =
        "---\nname: enabled\nsource: github:example/source-skill\nversion: v2\n---\nnew\n";
    fs::write(package.join("SKILL.md"), original).unwrap();
    fixture.config("skills:\n  disabled: [blocked]\n");
    let scan = fixture.ok(&["scan"]);
    let placement = fixture.named_placement("blocked");
    let mut request = fixture.roster_proposal(
        &scan,
        placement["skill_id"].as_str().unwrap(),
        "hermes",
        "core",
    );
    request.as_object_mut().unwrap().remove("roster_changes");
    request["source_updates"] = json!([{
        "skill_id": placement["skill_id"], "placement_id": placement["id"], "source": "github:example/source-skill",
        "current_revision": "v1", "current_fingerprint": placement["content_digest"],
        "upstream_revision": "v2", "upstream_content": upstream,
        "upstream_digest": format!("{:x}", Sha256::digest(upstream.as_bytes())), "choice": "adopt_upstream"
    }]);
    let blocked = fixture.command(&["plan", "--stdin"], Some(&request));
    assert_eq!(blocked["ok"], false, "{blocked}");
    assert_eq!(
        blocked["error"]["code"],
        "native_visibility_blocks_roster_change"
    );
    assert_eq!(
        fs::read_to_string(package.join("SKILL.md")).unwrap(),
        original
    );
}

#[test]
fn bootstrap_setup_preserves_native_restrictions_and_allows_ordinary_targets() {
    for scenario in ["disabled_name", "other_os", "ordinary"] {
        let fixture = Fixture::new();
        let package = fixture.package("skillroster", "skillroster");
        let original = match scenario {
            "disabled_name" => "---\nname: blocked\n---\nLocal custom bootstrap.\n".to_owned(),
            "other_os" => format!(
                "---\nname: skillroster\nplatforms: [{}]\n---\nLocal custom bootstrap.\n",
                other_desktop_platform()
            ),
            _ => "---\nname: skillroster\n---\nLocal custom bootstrap.\n".to_owned(),
        };
        fs::write(package.join("SKILL.md"), &original).unwrap();
        fixture.config("skills:\n  disabled: [blocked]\n");
        fixture.ok(&["scan"]);
        let setup = fixture.command(&["setup", "--modified-choice", "adopt-current"], None);
        if scenario == "ordinary" {
            assert_eq!(setup["ok"], true, "{setup}");
            assert!(setup["result"]["plan_id"].is_string(), "{setup}");
        } else {
            assert_eq!(setup["ok"], false, "{scenario}: {setup}");
            assert_eq!(
                setup["error"]["code"],
                "native_visibility_blocks_roster_change"
            );
            let plans: u64 = fixture
                .database()
                .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get(0))
                .unwrap();
            assert_eq!(plans, 0);
        }
        assert_eq!(
            fs::read_to_string(package.join("SKILL.md")).unwrap(),
            original
        );
        assert!(!package.join("references").exists());
    }

    for config in [
        "skills:\n  disabled: [skillroster]\n",
        "skills:\n  platform_disabled:\n    telegram: [skillroster]\n",
        "skills: {}\n",
    ] {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root()).unwrap();
        fixture.config(config);
        fixture.ok(&["scan"]);
        let setup = fixture.command(&["setup"], None);
        let package = fixture.root().join("skillroster");
        assert!(!package.exists());
        if config == "skills: {}\n" {
            assert_eq!(setup["ok"], true, "{setup}");
            let applied = fixture.ok(&["apply", setup["result"]["plan_id"].as_str().unwrap()]);
            assert!(package.join("SKILL.md").is_file());
            let undone = fixture.ok(&["undo", applied["result"]["receipt_id"].as_str().unwrap()]);
            assert_eq!(undone["result"]["verification"], "passed");
            assert!(!package.exists());
        } else {
            assert_eq!(setup["ok"], false, "{setup}");
            assert_eq!(
                setup["error"]["code"],
                "native_visibility_blocks_roster_change"
            );
            let plans: u64 = fixture
                .database()
                .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get(0))
                .unwrap();
            assert_eq!(plans, 0);
        }
        assert_eq!(
            fs::read_to_string(fixture.home().join(".hermes/config.yaml")).unwrap(),
            config
        );
    }
}
