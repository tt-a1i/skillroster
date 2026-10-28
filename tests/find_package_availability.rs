use serde_json::Value;
use std::{fs, process::Command};
use tempfile::TempDir;

fn run(temp: &TempDir, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .arg("--home")
        .arg(temp.path().join("home"))
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn fresh_oversized_package_is_unavailable_not_stale_and_loading_stays_closed() {
    let temp = TempDir::new().unwrap();
    let package = temp.path().join("home/.codex/skills/roster-eval-diagram");
    fs::create_dir_all(package.join("assets")).unwrap();
    fs::write(package.join("SKILL.md"), "---\nname: roster-eval-diagram\ndescription: Render architecture diagram for evaluation.\n---\nRender the requested architecture diagram.\n").unwrap();
    for index in 0..17 {
        fs::write(
            package.join(format!("assets/{index:02}.bin")),
            vec![0; 1024 * 1024],
        )
        .unwrap();
    }
    for _ in 0..2 {
        assert_eq!(run(&temp, &["scan", "--summary"])["ok"], true);
        let found = run(
            &temp,
            &["find", "--limit", "1", "--", "roster-eval-diagram"],
        );
        assert_eq!(found["ok"], true);
        assert_eq!(found["result"]["rescan_required"], false);
        let candidate = &found["result"]["matches"][0];
        assert_eq!(candidate["paths"], serde_json::json!([]));
        assert_eq!(
            candidate["path_availability"]["state"],
            "incomplete_package"
        );
        assert_eq!(
            candidate["path_availability"]["package_budget_bytes"],
            16777216
        );
        assert_eq!(
            candidate["path_availability"]["rescan_alone_sufficient"],
            false
        );
        assert!(
            candidate["path_availability"]["placements"][0]["detail"]
                .as_str()
                .unwrap()
                .contains("16777216")
        );
        assert!(
            !found["result"]["warnings"]
                .to_string()
                .contains("run skillroster scan")
        );
        let loaded = run(
            &temp,
            &[
                "find",
                "--load",
                "--limit",
                "1",
                "--",
                "roster-eval-diagram",
            ],
        );
        assert_eq!(loaded["ok"], false);
        assert_eq!(
            loaded["error"]["details"]["reason"],
            "package_fingerprint_incomplete"
        );
        assert!(
            !loaded["suggested_actions"]
                .to_string()
                .contains("refresh_snapshot")
        );
    }
    let entrypoint = package.join("SKILL.md");
    let original = fs::read(&entrypoint).unwrap();
    fs::remove_file(&entrypoint).unwrap();
    let deleted = run(
        &temp,
        &["find", "--limit", "1", "--", "roster-eval-diagram"],
    );
    assert_eq!(deleted["result"]["rescan_required"], true, "{deleted:#}");
    fs::write(&entrypoint, &original).unwrap();
    fs::write(
        &entrypoint,
        [original.as_slice(), b"changed instructions"].concat(),
    )
    .unwrap();
    let changed = run(
        &temp,
        &["find", "--limit", "1", "--", "roster-eval-diagram"],
    );
    assert_eq!(changed["result"]["rescan_required"], true, "{changed:#}");
    fs::write(&entrypoint, &original).unwrap();
    #[cfg(unix)]
    {
        let saved = temp.path().join("original-package");
        let replacement = temp.path().join("replacement-package");
        fs::rename(&package, &saved).unwrap();
        fs::create_dir(&replacement).unwrap();
        fs::write(replacement.join("SKILL.md"), &original).unwrap();
        std::os::unix::fs::symlink(&replacement, &package).unwrap();
        let retargeted = run(
            &temp,
            &["find", "--limit", "1", "--", "roster-eval-diagram"],
        );
        assert_eq!(
            retargeted["result"]["rescan_required"], true,
            "{retargeted:#}"
        );
        fs::remove_file(&package).unwrap();
        fs::rename(saved, &package).unwrap();
    }
    fs::remove_dir_all(package.join("assets")).unwrap();
    assert_eq!(run(&temp, &["scan", "--summary"])["ok"], true);
    let recovered = run(
        &temp,
        &[
            "find",
            "--load",
            "--limit",
            "1",
            "--",
            "roster-eval-diagram",
        ],
    );
    assert_eq!(recovered["ok"], true, "{recovered:#}");
    fs::write(package.join("new-payload.txt"), "changed after Scan").unwrap();
    let drift = run(
        &temp,
        &["find", "--limit", "1", "--", "roster-eval-diagram"],
    );
    assert_eq!(drift["result"]["rescan_required"], true);
    assert!(
        drift["result"]["warnings"]
            .to_string()
            .contains("run skillroster scan")
    );
}

#[cfg(unix)]
#[test]
fn fresh_permission_failure_is_not_empty_content_or_snapshot_drift_and_recovers() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let package = temp.path().join("home/.codex/skills/unreadable-skill");
    fs::create_dir_all(&package).unwrap();
    let entrypoint = package.join("SKILL.md");
    let contents = "---\nname: unreadable-skill\ndescription: Inspect a synthetic permission failure.\n---\nRead only.\n";
    fs::write(&entrypoint, contents).unwrap();
    fs::set_permissions(&entrypoint, fs::Permissions::from_mode(0o000)).unwrap();
    // Privileged test runners cannot model denied access with mode bits.
    if fs::read(&entrypoint).is_ok() {
        fs::set_permissions(&entrypoint, fs::Permissions::from_mode(0o644)).unwrap();
        return;
    }
    for _ in 0..2 {
        let scanned = run(&temp, &["scan", "--summary"]);
        assert_eq!(scanned["ok"], true);
        let database =
            rusqlite::Connection::open(temp.path().join("state/skillroster.db")).unwrap();
        let payload: String = database
            .query_row(
                "SELECT payload_json FROM scan_payloads WHERE scan_id=?1",
                [scanned["result"]["snapshot_id"].as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let snapshot: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(
            snapshot["placements"][0]["entrypoint_read_error"],
            "permission_denied"
        );
        assert!(snapshot["placements"][0]["entrypoint_digest"].is_null());
        let found = run(&temp, &["find", "--limit", "1", "--", "unreadable-skill"]);
        assert_eq!(found["result"]["rescan_required"], false, "{found:#}");
        assert!(
            !found["result"]["warnings"]
                .to_string()
                .contains("run skillroster scan")
        );
        assert_eq!(
            found["result"]["matches"][0]["path_availability"]["placements"][0]["entrypoint_read_error"],
            "permission_denied"
        );
        assert_eq!(
            run(
                &temp,
                &["find", "--load", "--limit", "1", "--", "unreadable-skill"]
            )["ok"],
            false
        );
    }
    // A disappearance is different from the frozen permission failure.
    fs::remove_file(&entrypoint).unwrap();
    assert_eq!(
        run(&temp, &["find", "--limit", "1", "--", "unreadable-skill"])["result"]["rescan_required"],
        true
    );
    fs::write(&entrypoint, contents).unwrap();
    let changed = run(&temp, &["find", "--limit", "1", "--", "unreadable-skill"]);
    assert_eq!(changed["result"]["rescan_required"], true, "{changed:#}");
    // Repair access then rescan, without replacing state or forgetting history.
    assert_eq!(run(&temp, &["scan", "--summary"])["ok"], true);
    let loaded = run(
        &temp,
        &["find", "--load", "--limit", "1", "--", "unreadable-skill"],
    );
    assert_eq!(loaded["ok"], true, "{loaded:#}");
}

#[test]
fn fresh_invalid_or_over_budget_entrypoints_explain_read_failure_without_rescan_loop() {
    for bytes in [vec![0xff, 0xfe], vec![b'x'; 2 * 1024 * 1024 + 1]] {
        let temp = TempDir::new().unwrap();
        let package = temp.path().join("home/.codex/skills/unreadable-entrypoint");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("SKILL.md"), bytes).unwrap();
        assert_eq!(run(&temp, &["scan", "--summary"])["ok"], true);
        let found = run(
            &temp,
            &["find", "--limit", "1", "--", "unreadable-entrypoint"],
        );
        assert_eq!(found["result"]["rescan_required"], false, "{found:#}");
        assert_eq!(
            found["result"]["matches"][0]["path_availability"]["placements"][0]["entrypoint_read_error"],
            "invalid_data"
        );
        assert_eq!(
            run(
                &temp,
                &[
                    "find",
                    "--load",
                    "--limit",
                    "1",
                    "--",
                    "unreadable-entrypoint"
                ]
            )["ok"],
            false
        );
        fs::write(package.join("SKILL.md"), "---\nname: unreadable-entrypoint\ndescription: Repaired entrypoint.\n---\nRead only.\n").unwrap();
        assert_eq!(run(&temp, &["scan", "--summary"])["ok"], true);
        let loaded = run(
            &temp,
            &[
                "find",
                "--load",
                "--limit",
                "1",
                "--",
                "unreadable-entrypoint",
            ],
        );
        assert_eq!(loaded["ok"], true, "{loaded:#}");
    }
}
