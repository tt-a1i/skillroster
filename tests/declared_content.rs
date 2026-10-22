use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    temp: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            temp: TempDir::new().unwrap(),
        };
        fs::create_dir_all(fixture.home().join(".codex")).unwrap();
        fs::write(
            fixture.home().join(".codex/config.toml"),
            "# user configuration\n",
        )
        .unwrap();
        fixture
    }

    fn home(&self) -> PathBuf {
        self.temp.path().join("home")
    }

    fn package(&self, directory: &str, name: &str, source: &str, body: &str) -> PathBuf {
        let directory = self.home().join(".codex/skills").join(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} capability\nsource: {source}\nversion: '1.0'\n---\n{body}\n"),
        )
        .unwrap();
        directory
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
            .args([
                "--home",
                self.home().to_str().unwrap(),
                "--state-dir",
                self.temp.path().join("state").to_str().unwrap(),
                "--json",
            ])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            fs::read_to_string(self.home().join(".codex/config.toml")).unwrap(),
            "# user configuration\n"
        );
        output
    }

    fn ok(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn blocked(&self, args: &[&str], reason: &str) -> Value {
        let output = self.run(args);
        assert!(
            !output.status.success(),
            "unexpected success: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["details"]["reason"], reason, "{value}");
        assert!(value["result"].get("loaded_skill").is_none());
        assert!(!value.to_string().contains("PRIVATE_INSTRUCTIONS"));
        value
    }
}

fn placements(found: &Value) -> Vec<&Value> {
    found["result"]["matches"][0]["content_choice"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["placements"].as_array().unwrap())
        .collect()
}

#[test]
fn declared_identity_preserves_observations_and_requires_exact_content_choice() {
    let fixture = Fixture::new();
    fixture.package(
        "a-deploy",
        "deploy",
        "example/shared",
        "DEPLOY_PRIVATE_INSTRUCTIONS",
    );
    fixture.package(
        "z-research",
        "research",
        "example/shared",
        "RESEARCH_PRIVATE_INSTRUCTIONS",
    );
    let scan = fixture.ok(&["scan"]);
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let research = fixture.ok(&["find", "research", "--limit", "1"]);
    let deploy = fixture.ok(&["find", "deploy", "--limit", "1"]);
    let research_match = &research["result"]["matches"][0];
    let deploy_match = &deploy["result"]["matches"][0];
    assert_eq!(research_match["skill_id"], deploy_match["skill_id"]);
    assert_eq!(research_match["name"], "research");
    assert_eq!(deploy_match["name"], "deploy");
    for found in [&research, &deploy] {
        assert!(!found.to_string().contains("PRIVATE_INSTRUCTIONS"));
        let ranked = &found["result"]["matches"][0];
        assert!(
            ranked["match_reasons"]
                .as_array()
                .unwrap()
                .contains(&json!("exact_name"))
        );
        assert_eq!(ranked["content_choice"]["content_group_count"], 2);
        assert_eq!(ranked["content_choice"]["content_selection_required"], true);
        assert_eq!(ranked["content_choice"]["groups_truncated"], false);
        for placement in placements(found) {
            assert!(
                placement["placement_id"]
                    .as_str()
                    .unwrap()
                    .starts_with("placement_")
            );
            assert!(placement["path"].as_str().unwrap().ends_with("SKILL.md"));
            assert!(placement["read_state"].is_string());
            assert!(placement["content_identity_digest"].is_string());
        }
    }
    let combined = fixture.ok(&["find", "research deploy", "--limit", "1"]);
    let reasons = combined["result"]["matches"][0]["match_reasons"]
        .as_array()
        .unwrap();
    assert!(reasons.contains(&json!("all_text_tokens:1")));
    assert!(!reasons.contains(&json!("all_text_tokens:2")));
    let blocked = fixture.blocked(
        &["find", "research", "--load", "--limit", "1"],
        "declared_identity_content_ambiguous",
    );
    let recovery = blocked["suggested_actions"].as_array().unwrap();
    assert!(recovery.iter().any(|action| {
        let argv = action["argv"].as_array().unwrap();
        argv.contains(&json!("find")) && !argv.contains(&json!("--load"))
    }));
    assert!(
        !recovery
            .iter()
            .any(|action| action["argv"].as_array().unwrap().contains(&json!("scan")))
    );
    for placement in placements(&research) {
        let id = placement["placement_id"].as_str().unwrap();
        let loaded = fixture.ok(&[
            "find",
            "research",
            "--load",
            "--limit",
            "1",
            "--variant-placement-id",
            id,
            "--require-snapshot",
            snapshot,
        ]);
        let selected = &loaded["result"]["loaded_skill"];
        assert_eq!(selected["selection"]["placement_id"], id);
        assert_eq!(selected["selection"]["name"], placement["name"]);
        assert_eq!(
            selected["selection"]["ranking_evidence_scope"],
            "ranked_capability_group"
        );
        assert_eq!(selected["content"]["path"], placement["path"]);
        assert_eq!(selected["content"]["complete"], true);
        assert_eq!(
            selected["content"]["text"],
            fs::read_to_string(placement["path"].as_str().unwrap()).unwrap()
        );
        assert_eq!(loaded["result"]["files_changed"], false);
    }
    let actions = research["suggested_actions"].as_array().unwrap();
    let exact = actions
        .iter()
        .filter(|action| {
            action["argv"]
                .as_array()
                .is_some_and(|args| args.contains(&json!("--variant-placement-id")))
        })
        .collect::<Vec<_>>();
    assert_eq!(exact.len(), 2);
    for action in exact {
        assert!(
            action["argv"]
                .as_array()
                .unwrap()
                .windows(2)
                .any(|pair| pair == [json!("--require-snapshot"), json!(snapshot)])
        );
    }
}

#[test]
fn same_entrypoint_with_different_support_requires_choice_but_gitignore_does_not() {
    for (support_file, divergent) in [("run.sh", true), (".gitignore", false)] {
        let fixture = Fixture::new();
        let first = fixture.package(
            "first",
            "same",
            "example/shared",
            "SAME_PRIVATE_INSTRUCTIONS",
        );
        let second = fixture.package(
            "second",
            "same",
            "example/shared",
            "SAME_PRIVATE_INSTRUCTIONS",
        );
        fs::write(first.join(support_file), "first\n").unwrap();
        fs::write(second.join(support_file), "second\n").unwrap();
        fixture.ok(&["scan"]);
        let found = fixture.ok(&["find", "same"]);
        assert_eq!(
            found["result"]["matches"][0]["content_choice"]["content_group_count"],
            if divergent { 2 } else { 1 }
        );
        if divergent {
            fixture.blocked(
                &["find", "same", "--load"],
                "declared_identity_content_ambiguous",
            );
        } else {
            assert_eq!(
                found["result"]["matches"][0]["content_choice"]["routing_equivalent"],
                true
            );
            assert_eq!(
                found["result"]["matches"][0]["content_choice"]["content_selection_required"],
                false
            );
            let loaded = fixture.ok(&["find", "same", "--load"]);
            assert_eq!(
                loaded["result"]["loaded_skill"]["content"]["complete"],
                true
            );
        }
    }
}

#[test]
fn exact_placement_selection_is_bound_to_top_group_identity_and_snapshot_bytes() {
    for drift_support in [false, true] {
        let fixture = Fixture::new();
        let first = fixture.package(
            "first",
            "research",
            "example/shared",
            "FIRST_PRIVATE_INSTRUCTIONS",
        );
        fixture.package(
            "second",
            "research",
            "example/shared",
            "SECOND_PRIVATE_INSTRUCTIONS",
        );
        fixture.package(
            "other",
            "unique",
            "example/other",
            "OTHER_PRIVATE_INSTRUCTIONS",
        );
        let scan = fixture.ok(&["scan"]);
        let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
        let found = fixture.ok(&["find", "research", "--limit", "1"]);
        let observed = placements(&found);
        let first_placement = observed
            .iter()
            .find(|placement| placement["path"] == first.join("SKILL.md").to_str().unwrap())
            .unwrap();
        let placement_id = first_placement["placement_id"].as_str().unwrap();
        let missing_load =
            fixture.run(&["find", "research", "--variant-placement-id", placement_id]);
        assert!(!missing_load.status.success());
        let missing_load: Value = serde_json::from_slice(&missing_load.stdout).unwrap();
        assert_eq!(missing_load["error"]["code"], "invalid_cli_arguments");
        assert!(!missing_load.to_string().contains("PRIVATE_INSTRUCTIONS"));
        fixture.blocked(
            &[
                "find",
                "unique",
                "--load",
                "--variant-placement-id",
                placement_id,
            ],
            "variant_placement_not_in_top_match",
        );
        let other = fixture.ok(&["find", "unique", "--limit", "1"]);
        let other_id = other["result"]["matches"][0]["skill_id"].as_str().unwrap();
        fixture.blocked(
            &[
                "find",
                "research",
                "--load",
                "--variant-placement-id",
                placement_id,
                "--variant-skill-id",
                other_id,
            ],
            "variant_selectors_disagree",
        );
        let matching_id = found["result"]["matches"][0]["skill_id"].as_str().unwrap();
        fixture.ok(&[
            "find",
            "research",
            "--load",
            "--variant-placement-id",
            placement_id,
            "--variant-skill-id",
            matching_id,
        ]);
        let action = found["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| {
                action["argv"].as_array().is_some_and(|args| {
                    args.windows(2)
                        .any(|pair| pair == [json!("--variant-placement-id"), json!(placement_id)])
                })
            })
            .unwrap();
        let argv = action["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(
            argv.windows(2)
                .any(|pair| pair == ["--require-snapshot", snapshot])
        );
        let (path, reason) = if drift_support {
            (first.join("run.sh"), "package_identity_drift")
        } else {
            (first.join("SKILL.md"), "entrypoint_content_drift")
        };
        fs::write(path, "CHANGED_PRIVATE_INSTRUCTIONS\n").unwrap();
        let replay = Command::new(env!("CARGO_BIN_EXE_skillroster"))
            .args(&argv[1..])
            .output()
            .unwrap();
        assert!(!replay.status.success());
        let replayed: Value = serde_json::from_slice(&replay.stdout).unwrap();
        assert_eq!(replayed["error"]["details"]["reason"], reason, "{replayed}");
        assert!(!replayed.to_string().contains("PRIVATE_INSTRUCTIONS"));
        fixture.blocked(
            &[
                "find",
                "research",
                "--load",
                "--variant-placement-id",
                placement_id,
                "--require-snapshot",
                snapshot,
            ],
            reason,
        );
    }
}

#[test]
fn content_choice_groups_are_bounded_without_hiding_total_count() {
    let fixture = Fixture::new();
    for index in 0..11 {
        fixture.package(
            &format!("copy-{index}"),
            "bounded",
            "example/shared",
            &format!("{index}_PRIVATE_INSTRUCTIONS"),
        );
    }
    fixture.ok(&["scan"]);
    let found = fixture.ok(&["find", "bounded", "--limit", "1"]);
    let choice = &found["result"]["matches"][0]["content_choice"];
    assert_eq!(choice["content_group_count"], 11);
    assert_eq!(choice["groups_truncated"], true);
    assert_eq!(choice["groups"].as_array().unwrap().len(), 10);
    fixture.blocked(
        &["find", "bounded", "--load"],
        "declared_identity_content_ambiguous",
    );
}

#[test]
fn same_name_variant_selection_cannot_bypass_content_choice_within_one_identity() {
    let fixture = Fixture::new();
    fixture.package(
        "a",
        "shared",
        "example/shared",
        "FIRST_PRIVATE_INSTRUCTIONS",
    );
    fixture.package(
        "b",
        "shared",
        "example/shared",
        "SECOND_PRIVATE_INSTRUCTIONS",
    );
    fixture.package("c", "shared", "example/other", "OTHER_PRIVATE_INSTRUCTIONS");
    fixture.ok(&["scan"]);
    let found = fixture.ok(&["find", "shared", "--limit", "1"]);
    let ranked = &found["result"]["matches"][0];
    assert_eq!(ranked["variant_count"], 2);
    let variant = ranked["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variant| variant["paths"].as_array().unwrap().len() == 2)
        .unwrap();
    fixture.blocked(
        &[
            "find",
            "shared",
            "--load",
            "--variant-skill-id",
            variant["skill_id"].as_str().unwrap(),
        ],
        "declared_identity_content_ambiguous",
    );
}

#[test]
fn report_content_conflict_offers_exact_read_recovery_without_automatic_plan() {
    let fixture = Fixture::new();
    fixture.package(
        "a",
        "research",
        "example/shared",
        "FIRST_PRIVATE_INSTRUCTIONS",
    );
    fixture.package(
        "b",
        "deploy",
        "example/shared",
        "SECOND_PRIVATE_INSTRUCTIONS",
    );
    fixture.ok(&["scan"]);
    let report = fixture.ok(&["report", "--full"]);
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Declared identity has divergent local content")
        .unwrap();
    let detail = fixture.ok(&[
        "report",
        "--finding",
        finding["id"].as_str().unwrap(),
        "--full",
    ]);
    assert_eq!(
        detail["result"]["resolution"]["decision"],
        "choose_declared_identity_content"
    );
    assert_eq!(
        detail["result"]["resolution"]["content_choice"]["content_group_count"],
        2
    );
    assert_eq!(
        detail["result"]["resolution"]["automatic_change_supported"],
        false
    );
    let actions = detail["suggested_actions"].as_array().unwrap();
    for group in detail["result"]["resolution"]["content_choice"]["groups"]
        .as_array()
        .unwrap()
    {
        for placement in group["placements"].as_array().unwrap() {
            let argv = placement["load_argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap())
                .collect::<Vec<_>>();
            assert!(argv.contains(&"--variant-placement-id"));
            assert!(argv.contains(&"--require-snapshot"));
            assert!(
                argv.windows(2)
                    .any(|pair| pair == ["--home", fixture.home().to_str().unwrap()])
            );
            assert!(argv.windows(2).any(|pair| pair
                == [
                    "--state-dir",
                    fixture.temp.path().join("state").to_str().unwrap()
                ]));
            let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
                .args(&argv[1..])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            let loaded: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                loaded["result"]["loaded_skill"]["selection"]["placement_id"],
                placement["placement_id"]
            );
        }
    }
    assert!(!actions.iter().any(|action| {
        action["argv"]
            .as_array()
            .is_some_and(|args| args.contains(&json!("plan")))
    }));
}

#[test]
fn legacy_snapshot_without_content_observations_requires_a_rescan() {
    let fixture = Fixture::new();
    fixture.package(
        "first",
        "research",
        "example/shared",
        "FIRST_PRIVATE_INSTRUCTIONS",
    );
    fixture.package(
        "second",
        "research",
        "example/shared",
        "SECOND_PRIVATE_INSTRUCTIONS",
    );
    fixture.ok(&["scan"]);
    let database =
        rusqlite::Connection::open(fixture.temp.path().join("state/skillroster.db")).unwrap();
    database.execute("UPDATE scan_payloads SET payload_json = json_remove(payload_json, '$.skill_content_variants')", []).unwrap();
    let output = fixture.run(&["find", "research", "--load"]);
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        error
            .to_string()
            .contains("legacy_content_observations_require_rescan"),
        "{error}"
    );
    assert!(!error.to_string().contains("PRIVATE_INSTRUCTIONS"));
    assert!(
        error["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["argv"]
                .as_array()
                .is_some_and(|args| args.contains(&json!("scan"))))
    );
}

#[test]
fn supplementary_name_finding_preserves_capability_scope_and_resolvable_evidence() {
    let fixture = Fixture::new();
    let research = fixture.package(
        "a-research",
        "research",
        "example/shared",
        "RESEARCH_PRIVATE_INSTRUCTIONS",
    );
    let shared_deploy = fixture.package(
        "b-deploy",
        "deploy",
        "example/shared",
        "SHARED_DEPLOY_PRIVATE_INSTRUCTIONS",
    );
    let other_deploy = fixture.package(
        "c-deploy",
        "deploy",
        "example/other",
        "OTHER_DEPLOY_PRIVATE_INSTRUCTIONS",
    );
    let scan = fixture.ok(&["scan"]);
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let report = fixture.ok(&["report", "--full"]);
    let findings = report["result"]["findings"].as_array().unwrap();
    let database =
        rusqlite::Connection::open(fixture.temp.path().join("state/skillroster.db")).unwrap();
    for finding in findings {
        for evidence in finding["evidence_ids"].as_array().unwrap() {
            let count: i64 = database
                .query_row(
                    "SELECT count(*) FROM evidence WHERE id = ?1 AND scan_id = ?2",
                    [evidence.as_str().unwrap(), snapshot],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "unresolvable evidence {evidence} in {finding}");
        }
    }
    let found = fixture.ok(&["find", "deploy", "--limit", "1"]);
    let matched = &found["result"]["matches"][0];
    assert_eq!(matched["name"], "deploy");
    assert_eq!(matched["variant_count"], 2);
    assert_eq!(matched["variant_finding"]["state"], "available");
    assert_eq!(matched["variant_finding"]["snapshot_id"], snapshot);
    let finding_id = matched["variant_finding"]["finding_id"].as_str().unwrap();
    let detail = fixture.ok(&["report", "--finding", finding_id, "--full"]);
    assert_eq!(
        detail["result"]["resolution"]["decision"],
        "choose_same_name_variant"
    );
    let variants = detail["result"]["resolution"]["variants"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 2);
    let expected_paths = [
        shared_deploy.join("SKILL.md"),
        other_deploy.join("SKILL.md"),
    ];
    for variant in variants {
        assert_eq!(variant["name"], "deploy");
        assert_eq!(variant["observed_names"], json!(["deploy"]));
        for path in variant["paths"].as_array().unwrap() {
            assert!(
                expected_paths
                    .iter()
                    .any(|expected| path == expected.to_str().unwrap())
            );
            assert_ne!(path, research.join("SKILL.md").to_str().unwrap());
        }
        for observation in variant["observations"].as_array().unwrap() {
            assert_eq!(observation["name"], "deploy");
            assert_ne!(
                observation["path"],
                research.join("SKILL.md").to_str().unwrap()
            );
        }
    }
    let actions = found["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["action"] == "load_exact_content_for_comparison")
        .collect::<Vec<_>>();
    assert_eq!(actions.len(), 3);
    for action in actions {
        let argv = action["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(
            argv.windows(2)
                .any(|pair| pair == ["--home", fixture.home().to_str().unwrap()])
        );
        assert!(
            argv.windows(2)
                .any(|pair| pair == ["--require-snapshot", snapshot])
        );
        let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
            .args(&argv[1..])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let loaded: Value = serde_json::from_slice(&output.stdout).unwrap();
        let selected = &loaded["result"]["loaded_skill"];
        let path = selected["content"]["path"].as_str().unwrap();
        assert_eq!(
            selected["content"]["text"],
            fs::read_to_string(path).unwrap()
        );
        assert_eq!(
            selected["selection"]["name"],
            if path == research.join("SKILL.md").to_str().unwrap() {
                "research"
            } else {
                "deploy"
            }
        );
        assert_eq!(
            selected["selection"]["ranking_evidence_scope"],
            "ranked_capability_group"
        );
    }
}
