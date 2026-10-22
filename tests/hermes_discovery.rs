#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
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

    fn roster_proposal(&self, scan: &Value, skill_id: &str) -> Value {
        let snapshot_id = scan["result"]["snapshot_id"].as_str().unwrap();
        let evidence: String = self.database().query_row(
            "SELECT id FROM evidence WHERE scan_id=?1 AND subject_type='skill' AND subject_id=?2 ORDER BY id LIMIT 1",
            rusqlite::params![snapshot_id, skill_id], |row| row.get(0),
        ).unwrap();
        json!({"schema_version":1,"scan_id":snapshot_id,"evidence_ids":[evidence],
            "roster_changes":[{"agent":"hermes","skill_id":skill_id,"state":"on_demand"}]})
    }

    fn hermes_root<'a>(&self, scan: &'a Value) -> &'a Value {
        scan["result"]["roots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|root| root["agent"] == "hermes" && root["kind"] == "skills")
            .unwrap()
    }
}

#[test]
fn discovery_matches_native_recursive_categories_without_a_runtime_dependency() {
    let fixture = Fixture::new();
    for (path, name) in [
        ("plain", "plain"),
        ("parent", "parent"),
        ("parent/group/deep", "deep"),
        ("target/legit", "legit"),
        ("scripts/category", "category"),
        (".backup/hidden", "hidden"),
        ("direct-original", "original"),
        ("category-original/child", "child"),
        ("parent/scripts/support", "support"),
        (".archive/archived", "archived"),
    ] {
        fixture.package(path, name);
    }
    symlink(
        fixture.root().join("direct-original"),
        fixture.root().join("direct-link"),
    )
    .unwrap();
    symlink(
        fixture.root().join("category-original"),
        fixture.root().join("category-link"),
    )
    .unwrap();
    let scan = fixture.ok(&["scan"]);
    assert_eq!(fixture.hermes_root(&scan)["discovery_complete"], true);
    let snapshot = fixture.snapshot();
    let exposed: BTreeSet<_> = snapshot["placements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|placement| placement["agent"] == "hermes" && placement["default_exposed"] == true)
        .map(|placement| {
            Path::new(placement["entrypoint"].as_str().unwrap())
                .strip_prefix(fixture.root())
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    // Hermes 0.20.0 native-paths.json, plus the deliberately materialized
    // category-original backing directory used by this portable fixture.
    assert_eq!(
        exposed,
        BTreeSet::from(
            [
                ".backup/hidden/SKILL.md",
                "category-link/child/SKILL.md",
                "category-original/child/SKILL.md",
                "direct-link/SKILL.md",
                "direct-original/SKILL.md",
                "parent/SKILL.md",
                "parent/group/deep/SKILL.md",
                "plain/SKILL.md",
                "scripts/category/SKILL.md",
                "target/legit/SKILL.md",
            ]
            .map(String::from)
        )
    );
    for name in ["child", "deep", "legit"] {
        let loaded = fixture.ok(&["find", name, "--load", "--limit", "1"]);
        assert!(
            loaded["result"]["loaded_skill"]["content"]["text"]
                .as_str()
                .unwrap()
                .contains(&format!("{name}_PRIVATE_BODY"))
        );
    }
    let direct = snapshot["placements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|placement| {
            placement["entrypoint"] == json!(fixture.root().join("direct-link/SKILL.md"))
        })
        .unwrap();
    assert_eq!(direct["mutation_scope"], "mutable");
}

#[test]
fn unapproved_category_is_unread_and_explicit_source_approval_recovers_complete_discovery() {
    let fixture = Fixture::new();
    fixture.package("ordinary", "ordinary");
    let external = fixture.0.path().join("external-category");
    fixture.package(external.join("child"), "externalchild");
    symlink(&external, fixture.root().join("external-link")).unwrap();
    let scan = fixture.ok(&["scan"]);
    assert_eq!(
        fixture.hermes_root(&scan)["discovery_complete"],
        false,
        "{scan}"
    );
    let snapshot = fixture.snapshot();
    assert!(!snapshot.to_string().contains("externalchild"));
    assert!(
        !snapshot["placements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|placement| {
                placement["entrypoint"]
                    == json!(fixture.root().join("external-link/child/SKILL.md"))
            })
    );
    let found = fixture.ok(&["find", "externalchild"]);
    assert_eq!(found["result"]["matches"], json!([]));
    let approved = fixture.ok(&["--source-root", external.to_str().unwrap(), "scan"]);
    assert_eq!(fixture.hermes_root(&approved)["discovery_complete"], true);
    let loaded = fixture.ok(&["find", "externalchild", "--load"]);
    assert_eq!(
        loaded["result"]["loaded_skill"]["governance"]["mutation_scope"],
        "shared_read_only"
    );
    let skill_id = loaded["result"]["matches"][0]["skill_id"].as_str().unwrap();
    let proposal = fixture.roster_proposal(&approved, skill_id);
    let blocked = fixture.command(&["plan", "--stdin"], Some(&proposal));
    assert_eq!(blocked["ok"], false, "{blocked}");
    assert!(
        blocked.to_string().contains("shared_read_only"),
        "{blocked}"
    );
    assert!(external.join("child/SKILL.md").is_file());
}

#[test]
fn category_aliases_and_physical_aliases_are_loadable_but_cannot_be_roster_mutation_targets() {
    let fixture = Fixture::new();
    let package = fixture.package("category/child", "aliaschild");
    symlink(
        fixture.root().join("category"),
        fixture.root().join("category-link"),
    )
    .unwrap();
    symlink(&package, fixture.root().join("direct-alias")).unwrap();
    let scan = fixture.ok(&["scan"]);
    let snapshot = fixture.snapshot();
    let placements = snapshot["placements"].as_array().unwrap();
    assert_eq!(placements.len(), 3);
    assert!(
        placements
            .iter()
            .all(|placement| placement["mutation_scope"] == "shared_read_only")
    );
    let loaded = fixture.ok(&["find", "aliaschild", "--load"]);
    assert_eq!(
        loaded["result"]["loaded_skill"]["content"]["complete"],
        true
    );
    let skill_id = placements[0]["skill_id"].as_str().unwrap();
    let proposal = fixture.roster_proposal(&scan, skill_id);
    let blocked = fixture.command(&["plan", "--stdin"], Some(&proposal));
    assert_eq!(blocked["ok"], false, "{blocked}");
    assert!(
        blocked.to_string().contains("shared_read_only"),
        "{blocked}"
    );
    assert!(fixture.root().join("category-link").is_symlink());
    assert!(fixture.root().join("direct-alias").is_symlink());
    assert!(package.join("SKILL.md").is_file());
    let plan_count: u64 = fixture
        .database()
        .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get(0))
        .unwrap();
    assert_eq!(plan_count, 0);
}

#[test]
fn retargeted_category_refuses_snapshot_bound_exact_load() {
    let fixture = Fixture::new();
    fixture.package("first/child", "originalchild");
    let second = fixture.package("second/child", "replacementchild");
    let alias = fixture.root().join("category-link");
    symlink(fixture.root().join("first"), &alias).unwrap();
    let scan = fixture.ok(&["scan"]);
    let snapshot = fixture.snapshot();
    let placement = snapshot["placements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|placement| placement["entrypoint"] == json!(alias.join("child/SKILL.md")))
        .unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(second.parent().unwrap(), &alias).unwrap();
    let blocked = fixture.command(
        &[
            "find",
            "originalchild",
            "--load",
            "--variant-placement-id",
            placement["id"].as_str().unwrap(),
            "--require-snapshot",
            scan["result"]["snapshot_id"].as_str().unwrap(),
        ],
        None,
    );
    assert_eq!(blocked["ok"], false, "{blocked}");
    assert!(blocked["result"].get("loaded_skill").is_none());
    assert!(!blocked.to_string().contains("PRIVATE_BODY"));
}

#[test]
fn old_hermes_discovery_snapshot_requires_one_typed_rescan_before_reuse() {
    let fixture = Fixture::new();
    let package = fixture.package("parent/group/child", "oldchild");
    let scan = fixture.ok(&["scan"]);
    let snapshot = fixture.snapshot();
    let proposal = fixture.roster_proposal(
        &scan,
        snapshot["placements"][0]["skill_id"].as_str().unwrap(),
    );
    let plan = fixture.command(&["plan", "--stdin"], Some(&proposal));
    assert_eq!(plan["ok"], true, "{plan}");
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    fixture.database().execute(
        "UPDATE scan_payloads SET payload_json=json_remove(payload_json,'$.hermes_discovery_contract_version')", [],
    ).unwrap();
    for args in [
        &["report"][..],
        &["find", "oldchild", "--load"][..],
        &["setup", "--check"][..],
        &["apply", plan_id][..],
    ] {
        let blocked = fixture.command(args, None);
        assert_eq!(blocked["ok"], false, "{blocked}");
        assert_eq!(blocked["error"]["code"], "content_identity_rescan_required");
        assert_eq!(
            blocked["error"]["details"]["reason"],
            "legacy_hermes_discovery_requires_rescan"
        );
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
    let shown = fixture.ok(&["plan", "--show", plan_id]);
    assert_eq!(shown["result"]["rescan_required"], true);
    assert_eq!(
        shown["result"]["rescan_reason"],
        "legacy_hermes_discovery_requires_rescan"
    );
    let actions = shown["suggested_actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert!(
        actions[0]["argv"]
            .as_array()
            .unwrap()
            .contains(&json!("scan"))
    );
    assert!(package.join("SKILL.md").is_file());
    let status: String = fixture
        .database()
        .query_row("SELECT status FROM plans WHERE id=?1", [plan_id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "ready");
    fixture.ok(&["scan"]);
    fixture.ok(&["find", "oldchild", "--load"]);
}

#[test]
fn linked_hermes_root_cannot_adopt_or_install_bootstrap_into_shared_contents() {
    for populated in [true, false] {
        let fixture = Fixture::new();
        let shared = fixture.0.path().join("shared");
        fs::create_dir_all(&shared).unwrap();
        fs::create_dir_all(fixture.root().parent().unwrap()).unwrap();
        symlink(&shared, fixture.root()).unwrap();
        let package = shared.join("skillroster");
        let original = "---\nname: skillroster\ndescription: custom bootstrap\n---\nKeep my custom instructions.\n";
        if populated {
            fs::create_dir_all(&package).unwrap();
            fs::write(package.join("SKILL.md"), original).unwrap();
        }
        let scan = fixture.ok(&["scan"]);
        assert_eq!(fixture.hermes_root(&scan)["shared"], true);
        let snapshot = fixture.snapshot();
        if populated {
            assert_eq!(snapshot["placements"].as_array().unwrap().len(), 1);
            assert_eq!(
                snapshot["placements"][0]["mutation_scope"],
                "shared_read_only"
            );
        }
        let setup = fixture.command(&["setup", "--modified-choice", "adopt-current"], None);
        assert!(setup["result"]["plan_id"].is_null(), "{setup}");
        let plans: u64 = fixture
            .database()
            .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get(0))
            .unwrap();
        assert_eq!(plans, 0);
        if populated {
            assert_eq!(
                fs::read_to_string(package.join("SKILL.md")).unwrap(),
                original
            );
            assert!(!package.join("references").exists());
        } else {
            assert!(!package.exists());
        }
    }
}

#[test]
fn ordinary_file_symlink_does_not_create_a_category_or_incomplete_discovery() {
    let fixture = Fixture::new();
    let package = fixture.package("filelinks", "filelinks");
    fs::write(package.join("README.md"), "Package documentation.\n").unwrap();
    symlink("README.md", package.join("guide.md")).unwrap();
    fixture.package("ordinary", "ordinary");
    let scan = fixture.ok(&["scan"]);
    assert_eq!(
        fixture.hermes_root(&scan)["discovery_complete"],
        true,
        "{scan}"
    );
    let snapshot = fixture.snapshot();
    let placements = snapshot["placements"].as_array().unwrap();
    assert_eq!(placements.len(), 2, "{placements:?}");
    let ordinary = placements
        .iter()
        .find(|placement| {
            placement["entrypoint"] == json!(fixture.root().join("ordinary/SKILL.md"))
        })
        .unwrap();
    let proposal = fixture.roster_proposal(&scan, ordinary["skill_id"].as_str().unwrap());
    let plan = fixture.command(&["plan", "--stdin"], Some(&proposal));
    assert_eq!(plan["ok"], true, "{plan}");
    assert!(plan["result"]["plan_id"].is_string());
    assert!(package.join("guide.md").is_symlink());
}

#[test]
fn ready_bootstrap_plan_rechecks_readonly_snapshot_overlap_before_apply() {
    let fixture = Fixture::new();
    let package = fixture.package("skillroster", "skillroster");
    let original = fs::read(package.join("SKILL.md")).unwrap();
    let scan = fixture.ok(&["scan"]);
    // Ordinary mutable packages still support explicit adoption.
    let setup = fixture.ok(&["setup", "--modified-choice", "adopt-current"]);
    let plan_id = setup["result"]["plan_id"].as_str().unwrap();
    let mut snapshot = fixture.snapshot();
    // Model an existing Ready Bootstrap Plan whose bound Snapshot records a
    // read-only overlap. Keep its content and physical identity facts intact.
    snapshot["placements"][0]["mutation_scope"] = json!("shared_read_only");
    snapshot["placements"][0]["governable"] = json!(false);
    fixture
        .database()
        .execute(
            "UPDATE scan_payloads SET payload_json=?1 WHERE scan_id=?2",
            rusqlite::params![
                snapshot.to_string(),
                scan["result"]["snapshot_id"].as_str().unwrap()
            ],
        )
        .unwrap();
    let blocked = fixture.command(&["apply", plan_id], None);
    assert_eq!(blocked["ok"], false, "{blocked}");
    assert_eq!(blocked["error"]["code"], "roster_mutation_scope_read_only");
    assert_eq!(fs::read(package.join("SKILL.md")).unwrap(), original);
    assert!(!package.join("references").exists());
    let status: String = fixture
        .database()
        .query_row("SELECT status FROM plans WHERE id=?1", [plan_id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "ready");
}
