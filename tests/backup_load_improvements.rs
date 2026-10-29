use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

fn run(temp: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .args([
            "--home",
            temp.path().join("home").to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "--json",
        ])
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn skill(path: &Path, body: &str, support: &str) {
    fs::create_dir_all(path.join("references")).unwrap();
    fs::write(path.join("SKILL.md"), body).unwrap();
    fs::write(path.join("references/guide.md"), support).unwrap();
}

const BODY: &str =
    "---\nname: route-helper\ndescription: Route one exact task\n---\nRead references/guide.md.\n";

#[test]
fn find_load_skips_stale_alternative_when_exact_copy_remains() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    for name in ["a-copy", "z-copy"] {
        skill(&root.join(name), BODY, "same support");
    }
    success(run(&temp, &["scan"]));
    fs::remove_dir_all(root.join("a-copy")).unwrap();
    let found = success(run(&temp, &["find", "route-helper", "--load"]));
    assert_eq!(
        found["result"]["loaded_skill"]["content"]["path"],
        json!(root.join("z-copy/SKILL.md"))
    );
    // Loading one verified copy does not erase the stale placement's own facts.
    assert_eq!(found["result"]["rescan_required"], true);
    let groups = found["result"]["matches"][0]["content_choice"]["groups"]
        .as_array()
        .unwrap();
    assert!(
        groups
            .iter()
            .flat_map(|g| g["placements"].as_array().unwrap())
            .any(|p| p["read_state"] == "entrypoint_unreadable")
    );
}

#[test]
fn explicit_stale_placement_never_falls_back_to_an_exact_copy() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    for name in ["a-copy", "z-copy"] {
        skill(&root.join(name), BODY, "same support");
    }
    success(run(&temp, &["scan"]));
    let found = success(run(&temp, &["find", "route-helper"]));
    let placements = found["result"]["matches"][0]["content_choice"]["groups"][0]["placements"]
        .as_array()
        .unwrap();
    let id = placements
        .iter()
        .find(|p| p["path"] == json!(root.join("a-copy/SKILL.md")))
        .unwrap()["placement_id"]
        .as_str()
        .unwrap();
    fs::remove_dir_all(root.join("a-copy")).unwrap();
    let output = run(
        &temp,
        &[
            "find",
            "route-helper",
            "--load",
            "--variant-placement-id",
            id,
        ],
    );
    assert!(!output.status.success());
    let blocked: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(blocked["result"]["loaded_skill"].is_null());
    assert!(String::from_utf8_lossy(&output.stdout).contains("entrypoint_unreadable"));
}

#[test]
fn exact_copy_fallback_rechecks_supporting_package_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    for name in ["a-copy", "z-copy"] {
        skill(&root.join(name), BODY, "same support");
    }
    success(run(&temp, &["scan"]));
    fs::remove_dir_all(root.join("a-copy")).unwrap();
    fs::write(
        root.join("z-copy/references/guide.md"),
        "changed after scan",
    )
    .unwrap();
    let output = run(&temp, &["find", "route-helper", "--load"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("entrypoint_unreadable"));
}

#[test]
fn routing_equivalence_does_not_authorize_fallback_to_another_package_fingerprint() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    for name in ["a-copy", "z-copy"] {
        skill(&root.join(name), BODY, "same support");
    }
    fs::write(root.join("a-copy/.gitignore"), "cache-a\n").unwrap();
    fs::write(root.join("z-copy/.gitignore"), "cache-b\n").unwrap();
    success(run(&temp, &["scan"]));
    let found = success(run(&temp, &["find", "route-helper"]));
    assert_eq!(
        found["result"]["matches"][0]["content_choice"]["routing_equivalent"],
        true
    );
    fs::remove_dir_all(root.join("a-copy")).unwrap();
    let output = run(&temp, &["find", "route-helper", "--load"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("entrypoint_unreadable"));
}

#[test]
fn divergent_declared_identity_never_falls_back_without_content_selection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    let declared = "---\nname: route-helper\nsource: https://example.test/route-helper\nversion: '1'\ndescription: Route one exact task\n---\nRead references/guide.md.\n";
    skill(&root.join("a-copy"), declared, "support A");
    skill(&root.join("z-copy"), declared, "support B");
    success(run(&temp, &["scan"]));
    fs::remove_dir_all(root.join("a-copy")).unwrap();
    let output = run(&temp, &["find", "route-helper", "--load"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("declared_identity_content_ambiguous")
    );
}

fn finding_detail(temp: &TempDir) -> Value {
    let report = success(run(temp, &["report", "--findings", "--category", "layout"]));
    let id = report["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "same_name_divergent_content")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    success(run(temp, &["report", "--finding", id]))
}

#[test]
fn same_name_finding_exposes_identical_entrypoints_and_supporting_package_ambiguity() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    skill(&root.join("a-copy"), BODY, "support A");
    skill(&root.join("z-copy"), BODY, "support B");
    success(run(&temp, &["scan"]));
    let detail = finding_detail(&temp);
    let resolution = &detail["result"]["resolution"];
    assert_eq!(resolution["decision"], "choose_same_name_variant");
    assert_eq!(
        resolution["entrypoint_comparison"]["state"],
        "identical_across_variants"
    );
    assert_eq!(
        resolution["entrypoint_comparison"]["difference_scope"],
        "supporting_package_files"
    );
    assert_eq!(
        resolution["entrypoint_comparison"]["variant_count_with_digest"],
        2
    );
    let found = success(run(&temp, &["find", "route-helper"]));
    assert_eq!(
        found["result"]["matches"][0]["entrypoint_comparison"],
        resolution["entrypoint_comparison"]
    );
    let selected = found["result"]["matches"][0]["variants"][0]["skill_id"]
        .as_str()
        .unwrap();
    success(run(
        &temp,
        &[
            "find",
            "route-helper",
            "--load",
            "--variant-skill-id",
            selected,
        ],
    ));
}

#[test]
fn same_name_comparison_reports_different_entrypoints() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    skill(&root.join("a-copy"), BODY, "support A");
    skill(
        &root.join("z-copy"),
        &format!("{BODY}Different instructions."),
        "support B",
    );
    success(run(&temp, &["scan"]));
    let detail = finding_detail(&temp);
    assert_eq!(
        detail["result"]["resolution"]["entrypoint_comparison"]["state"],
        "different_across_variants"
    );
    assert_eq!(
        detail["result"]["resolution"]["entrypoint_comparison"]["difference_scope"],
        "entrypoint_content"
    );
}

#[test]
fn missing_snapshot_entrypoint_digest_keeps_comparison_unknown() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    skill(&root.join("a-copy"), BODY, "support A");
    skill(&root.join("z-copy"), BODY, "support B");
    let scan = success(run(&temp, &["scan"]));
    finding_detail(&temp);
    let db = rusqlite::Connection::open(temp.path().join("state/skillroster.db")).unwrap();
    let id = scan["result"]["snapshot_id"].as_str().unwrap();
    let payload: String = db
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let mut payload: Value = serde_json::from_str(&payload).unwrap();
    payload["placements"][0]["entrypoint_digest"] = Value::Null;
    db.execute(
        "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
        rusqlite::params![payload.to_string(), id],
    )
    .unwrap();
    let detail = finding_detail(&temp);
    assert_eq!(
        detail["result"]["resolution"]["entrypoint_comparison"]["state"],
        "unknown"
    );
    assert_eq!(
        detail["result"]["resolution"]["entrypoint_comparison"]["difference_scope"],
        "unknown"
    );
}

#[test]
fn entrypoint_comparison_covers_variants_beyond_the_bounded_preview() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    for index in 0..12 {
        skill(
            &root.join(format!("copy-{index:02}")),
            &format!("{BODY}Instructions {index}."),
            "support",
        );
    }
    success(run(&temp, &["scan"]));
    let found = success(run(&temp, &["find", "route-helper"]));
    let matched = &found["result"]["matches"][0];
    assert_eq!(matched["variants_truncated"], true);
    let comparison = &matched["entrypoint_comparison"];
    assert_eq!(comparison["variant_count"], 12);
    assert_eq!(comparison["distinct_digest_count"], 12);
    assert_eq!(comparison["digests"].as_array().unwrap().len(), 10);
    assert_eq!(comparison["digests_truncated"], true);
}

#[test]
fn entrypoint_comparison_excludes_other_named_observations_of_a_declared_identity() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("home/.codex/skills");
    let declared = "---\nname: route-helper\nsource: https://example.test/route-helper\nversion: '1'\ndescription: Route one exact task\n---\nRead references/guide.md.\n";
    skill(&root.join("a-route"), declared, "support A");
    skill(
        &root.join("b-other"),
        &declared.replace("name: route-helper", "name: other-name"),
        "support B",
    );
    // The second identity has the same instructions but no source declaration.
    // Those two route-helper entrypoints have two digests; other-name must not
    // add a third digest merely because it shares the first identity.
    skill(&root.join("z-route"), BODY, "support C");
    success(run(&temp, &["scan"]));
    let found = success(run(&temp, &["find", "route-helper"]));
    let comparison = &found["result"]["matches"][0]["entrypoint_comparison"];
    assert_eq!(comparison["variant_count"], 2);
    assert_eq!(comparison["distinct_digest_count"], 2);
    // The other-name entrypoint would introduce a third distinct digest.
}
