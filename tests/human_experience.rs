use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

fn command(temp: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skillroster"));
    command.args([
        "--home",
        temp.path().join("home").to_str().unwrap(),
        "--state-dir",
        temp.path().join("state").to_str().unwrap(),
    ]);
    command
}

fn run(temp: &TempDir, args: &[&str], input: Option<Value>) -> Value {
    let mut child = command(temp)
        .arg("--json")
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
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn skill(home: &Path, relative: &str, bytes: impl AsRef<[u8]>) {
    let directory = home.join(relative);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("SKILL.md"), bytes).unwrap();
}

#[test]
fn check_scans_once_reuses_and_refreshes_configuration_and_mutation_invalidations() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    skill(
        &home,
        ".codex/skills/review",
        "---\nname: review\n---\nReview source changes\n",
    );
    let first = run(&temp, &["check"], None);
    assert_eq!(first["result"]["scan_performed"], true);
    assert_eq!(first["result"]["files_changed"], false);
    let cached = run(&temp, &["check"], None);
    assert_eq!(cached["result"]["scan_performed"], false);
    for id in ["snapshot_id", "report_id"] {
        assert_eq!(first["result"][id], cached["result"][id]);
    }
    fs::write(
        home.join(".codex/config.toml"),
        "[[skills.config]]\nname='review'\nenabled=false\n",
    )
    .unwrap();
    let drift = run(&temp, &["check"], None);
    assert_eq!(drift["result"]["scan_performed"], true);
    assert_ne!(
        drift["result"]["snapshot_id"],
        first["result"]["snapshot_id"]
    );
    assert_eq!(drift["result"]["default_exposure"], 0);
    let setup = run(&temp, &["setup", "--agent", "codex"], None);
    let applied = run(
        &temp,
        &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(applied["result"]["verification"], "passed");
    let refreshed = run(&temp, &["check"], None);
    assert_eq!(refreshed["result"]["scan_performed"], true);
    assert_eq!(
        run(&temp, &["check"], None)["result"]["scan_performed"],
        false
    );
    let undone = run(
        &temp,
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(
        run(&temp, &["check"], None)["result"]["scan_performed"],
        true
    );
}

#[test]
fn check_refreshes_when_home_changes_with_the_same_state_directory() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let other_home = temp.path().join("other-home");
    skill(&home, ".codex/skills/first", "First home\n");
    skill(&other_home, ".codex/skills/second", "Second home\n");
    skill(&other_home, ".codex/skills/third", "Third skill\n");
    let first = run(&temp, &["check"], None);
    // Replace only Home, keeping the exact same state database.
    let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .args([
            "--home",
            other_home.to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "--json",
            "check",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let changed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(changed["result"]["scan_performed"], true);
    assert_ne!(
        changed["result"]["snapshot_id"],
        first["result"]["snapshot_id"]
    );
    assert_eq!(changed["result"]["default_exposure"], 2);
    assert_eq!(
        run(&temp, &["check"], None)["result"]["scan_performed"],
        true
    );
    assert_eq!(
        run(&temp, &["check"], None)["result"]["scan_performed"],
        false
    );
}

#[test]
fn check_binds_root_and_source_scope_but_reuses_equivalent_paths_and_order() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let root_a = temp.path().join("root-a");
    let root_b = temp.path().join("root-b");
    let source_a = temp.path().join("source-a");
    let source_b = temp.path().join("source-b");
    for root in [&root_a, &root_b, &source_a, &source_b] {
        skill(root, "example", "Example skill\n");
    }
    let root_a_arg = format!("codex={}", root_a.display());
    let root_b_arg = format!("pi={}", root_b.display());
    let root_a_equivalent = format!("codex={}/../root-a/.", root_a.display());
    let source_a_equivalent = source_a.join("../source-a/.");
    let first = run(&temp, &["scan"], None);
    let added_root = run(&temp, &["--root", &root_a_arg, "check"], None);
    assert_eq!(added_root["result"]["scan_performed"], true);
    assert_ne!(
        first["result"]["snapshot_id"],
        added_root["result"]["snapshot_id"]
    );
    let scopes = [
        vec!["--root", root_a_arg.as_str(), "--root", root_b_arg.as_str()],
        vec!["--root", root_b_arg.as_str()],
        vec!["--source-root", source_a.to_str().unwrap()],
        vec![
            "--source-root",
            source_a.to_str().unwrap(),
            "--source-root",
            source_b.to_str().unwrap(),
        ],
        vec!["--source-root", source_b.to_str().unwrap()],
        vec![],
    ];
    for mut args in scopes {
        args.push("check");
        let refreshed = run(&temp, &args, None);
        assert_eq!(refreshed["result"]["scan_performed"], true, "{args:?}");
        let cached = run(&temp, &args, None);
        assert_eq!(cached["result"]["scan_performed"], false, "{args:?}");
        assert_eq!(
            refreshed["result"]["snapshot_id"],
            cached["result"]["snapshot_id"]
        );
    }
    let original = run(
        &temp,
        &[
            "--root",
            &root_a_arg,
            "--root",
            &root_b_arg,
            "--source-root",
            source_a.to_str().unwrap(),
            "--source-root",
            source_b.to_str().unwrap(),
            "scan",
        ],
        None,
    );
    let equivalent = run(
        &temp,
        &[
            "--root",
            &root_b_arg,
            "--root",
            &root_a_equivalent,
            "--root",
            &root_a_arg,
            "--source-root",
            source_b.to_str().unwrap(),
            "--source-root",
            source_a_equivalent.to_str().unwrap(),
            "--source-root",
            source_a.to_str().unwrap(),
            "check",
        ],
        None,
    );
    assert_eq!(equivalent["result"]["scan_performed"], false);
    assert_eq!(
        original["result"]["snapshot_id"],
        equivalent["result"]["snapshot_id"]
    );

    let missing = temp.path().join("missing");
    let missing_equivalent = temp.path().join("other-missing/../missing/.");
    let first_missing = run(
        &temp,
        &["--source-root", missing.to_str().unwrap(), "check"],
        None,
    );
    let equivalent_missing = run(
        &temp,
        &[
            "--source-root",
            missing_equivalent.to_str().unwrap(),
            "check",
        ],
        None,
    );
    assert_eq!(equivalent_missing["result"]["scan_performed"], false);
    assert_eq!(
        first_missing["result"]["snapshot_id"],
        equivalent_missing["result"]["snapshot_id"]
    );
}

#[test]
fn check_refreshes_a_snapshot_without_configuration_binding_once() {
    let temp = TempDir::new().unwrap();
    let first = run(&temp, &["scan"], None);
    let store =
        skillroster::sqlite::StateStore::open(temp.path().join("state/skillroster.db")).unwrap();
    let (id, mut payload) = store.latest_scan_payload::<Value>().unwrap().unwrap();
    payload
        .as_object_mut()
        .unwrap()
        .remove("snapshot_configuration");
    store.save_scan_payload(&id, &payload).unwrap();
    drop(store);
    let refreshed = run(&temp, &["check"], None);
    assert_eq!(refreshed["result"]["scan_performed"], true);
    assert_ne!(
        refreshed["result"]["snapshot_id"],
        first["result"]["snapshot_id"]
    );
    let cached = run(&temp, &["check"], None);
    assert_eq!(cached["result"]["scan_performed"], false);
    assert_eq!(
        cached["result"]["snapshot_id"],
        refreshed["result"]["snapshot_id"]
    );
}

#[test]
fn exposed_bytes_count_utf8_crlf_copies_and_unknowns_without_counting_hidden_or_source_only() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let front = "---\r\nname: review\r\ndescription: 审查代码\r\n---\r\n";
    let text = format!("{front}# 审查\n");
    skill(&home, ".codex/skills/review", &text);
    skill(&home, ".claude/skills/review", &text);
    skill(&home, ".codex/skills/plain", "body only\n");
    skill(&home, ".codex/skills/unclosed", "---\nname: unclosed\n");
    skill(&home, ".codex/skills/invalid", [0xff, 0xfe]);
    skill(
        &home,
        ".codex/skills/hidden",
        "---\nname: hidden\n---\nHidden\n",
    );
    skill(
        &home,
        "sources/source-only",
        "---\nname: source-only\n---\nSource\n",
    );
    fs::write(
        home.join(".codex/config.toml"),
        "[[skills.config]]\nname='hidden'\nenabled=false\n",
    )
    .unwrap();
    let source = home.join("sources");
    let scan = run(
        &temp,
        &["--source-root", source.to_str().unwrap(), "scan"],
        None,
    );
    let footprint = &scan["result"]["exposed_metadata_footprint"];
    assert_eq!(footprint["default_exposed_placement_count"], 5);
    assert_eq!(
        footprint["exposed_metadata_bytes"],
        2 * text.len() + "body only\n".len() + "---\nname: unclosed\n".len()
    );
    assert_eq!(footprint["exposed_frontmatter_bytes"], 2 * front.len());
    assert_eq!(footprint["metadata_unknown_placement_count"], 1);
    assert_eq!(footprint["frontmatter_unknown_placement_count"], 2);
    assert_eq!(
        footprint["by_agent"]["claude-code"]["exposed_metadata_bytes"],
        text.len()
    );
    for args in [
        &["report"][..],
        &["report", "--full"],
        &["report", "--findings"],
        &["check"],
    ] {
        assert_eq!(
            skillroster::footprint::compact(
                run(&temp, args, None)["result"]["exposed_metadata_footprint"].clone()
            ),
            skillroster::footprint::compact(footprint.clone())
        );
    }
    let summary = run(
        &temp,
        &[
            "--source-root",
            source.to_str().unwrap(),
            "scan",
            "--summary",
        ],
        None,
    );
    assert_eq!(
        summary["result"]["exposed_metadata_footprint"],
        skillroster::footprint::compact(footprint.clone())
    );
}

#[test]
fn check_refreshes_legacy_byte_observations_once() {
    let temp = TempDir::new().unwrap();
    skill(
        &temp.path().join("home"),
        ".codex/skills/review",
        "---\nname: review\n---\nBody\n",
    );
    let first = run(&temp, &["check"], None);
    let store =
        skillroster::sqlite::StateStore::open(temp.path().join("state/skillroster.db")).unwrap();
    let (id, mut payload) = store.latest_scan_payload::<Value>().unwrap().unwrap();
    payload.as_object_mut().unwrap().remove("entrypoint_bytes");
    payload
        .as_object_mut()
        .unwrap()
        .remove("entrypoint_bytes_observed");
    store.save_scan_payload(&id, &payload).unwrap();
    drop(store);
    let refreshed = run(&temp, &["check"], None);
    assert_eq!(refreshed["result"]["scan_performed"], true);
    assert_ne!(
        refreshed["result"]["snapshot_id"],
        first["result"]["snapshot_id"]
    );
    assert_eq!(
        run(&temp, &["check"], None)["result"]["scan_performed"],
        false
    );
}

#[cfg(unix)]
#[test]
fn check_refreshes_previous_hermes_discovery_contract_once() {
    let temp = TempDir::new().unwrap();
    skill(
        &temp.path().join("home"),
        ".hermes/skills/review",
        "---\nname: review\n---\nBody\n",
    );
    let first = run(&temp, &["check"], None);
    let store =
        skillroster::sqlite::StateStore::open(temp.path().join("state/skillroster.db")).unwrap();
    let (id, mut payload) = store.latest_scan_payload::<Value>().unwrap().unwrap();
    payload["hermes_discovery_contract_version"] = json!(1);
    payload
        .as_object_mut()
        .unwrap()
        .remove("skill_discovery_gaps");
    store.save_scan_payload(&id, &payload).unwrap();
    drop(store);
    let refreshed = run(&temp, &["check"], None);
    assert_eq!(refreshed["result"]["scan_performed"], true);
    assert_ne!(
        refreshed["result"]["snapshot_id"],
        first["result"]["snapshot_id"]
    );
    assert_eq!(
        run(&temp, &["check"], None)["result"]["scan_performed"],
        false
    );
}

#[cfg(unix)]
#[test]
fn unapproved_external_links_are_unknown_bytes_until_an_explicit_source_scan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let external = temp.path().join("external");
    skill(
        &external,
        "review",
        "---\nname: review\n---\nPrivate source bytes\n",
    );
    fs::create_dir_all(home.join(".codex/skills")).unwrap();
    std::os::unix::fs::symlink(external.join("review"), home.join(".codex/skills/review")).unwrap();
    let check = run(&temp, &["check"], None);
    let bytes = &check["result"]["exposed_metadata_footprint"];
    assert_eq!(bytes["exposed_metadata_bytes"], 0);
    assert_eq!(bytes["metadata_unknown_placement_count"], 1);
    let authorized = run(
        &temp,
        &["--source-root", external.to_str().unwrap(), "scan"],
        None,
    );
    let bytes = &authorized["result"]["exposed_metadata_footprint"];
    assert_eq!(bytes["metadata_unknown_placement_count"], 0);
    assert_eq!(
        bytes["exposed_metadata_bytes"],
        fs::metadata(external.join("review/SKILL.md"))
            .unwrap()
            .len()
    );
}

#[test]
fn roster_plan_projects_measured_bytes_and_apply_matches_the_preview() {
    for copies in [false, true] {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        for index in 0..51 {
            let name = format!("skill{index:02}");
            skill(
                &home,
                &format!(".codex/skills/{name}"),
                format!(
                    "---\nname: {name}\ndescription: Distinct capability {index}\n---\nBody {index}\n"
                ),
            );
        }
        if copies {
            let original = home.join(".codex/skills/skill00/SKILL.md");
            skill(
                &home,
                ".codex/skills/category/copy",
                fs::read(original).unwrap(),
            );
        }
        run(&temp, &["check"], None);
        let findings = run(
            &temp,
            &["report", "--findings", "--category", "exposure"],
            None,
        );
        let finding = findings["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["title"] == "Large default Rosters need review")
            .unwrap();
        let plan = run(
            &temp,
            &["plan", "--stdin"],
            Some(
                json!({"schema_version":1, "finding_roster_changes":[{"finding_id":finding["id"], "core_budget":5, "protected_skill_ids":[]}]}),
            ),
        );
        let impact = &plan["result"]["impact"]["exposed_metadata_footprint"];
        assert_eq!(
            impact["before"]["default_exposed_placement_count"],
            if copies { 52 } else { 51 }
        );
        assert_eq!(
            impact["after"]["default_exposed_placement_count"],
            if copies { 6 } else { 5 }
        );
        assert!(
            impact["after"]["exposed_metadata_bytes"].as_u64().unwrap()
                < impact["before"]["exposed_metadata_bytes"].as_u64().unwrap()
        );
        let id = plan["result"]["plan_id"].as_str().unwrap();
        assert_eq!(
            run(&temp, &["plan", "--show", id], None)["result"]["impact"]["exposed_metadata_footprint"],
            *impact
        );
        run(&temp, &["apply", id], None);
        assert_eq!(
            run(&temp, &["check"], None)["result"]["exposed_metadata_footprint"],
            skillroster::footprint::compact(impact["after"].clone())
        );
    }
}

#[test]
fn init_refuses_json_and_piped_input_before_creating_state_or_agent_files() {
    for json in [false, true] {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("home/.codex/skills")).unwrap();
        let mut command = command(&temp);
        command.arg("init");
        if json {
            command.arg("--json");
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let message = if json {
            String::from_utf8(output.stdout).unwrap()
        } else {
            String::from_utf8(output.stderr).unwrap()
        };
        assert!(message.contains("interactive TTY"));
        assert!(message.contains("setup --json"));
        assert!(!temp.path().join("state").exists());
        assert!(!temp.path().join("home/.codex/skills/skillroster").exists());
    }
}

#[test]
fn previous_official_bootstrap_upgrades_reversibly_without_becoming_a_local_conflict() {
    let temp = TempDir::new().unwrap();
    let package = temp.path().join("home/.codex/skills/skillroster");
    fs::create_dir_all(package.join("references")).unwrap();
    let old = [
        ("SKILL.md", include_str!("fixtures/bootstrap-v1.8.53.md")),
        (
            "references/routing.md",
            include_str!("fixtures/bootstrap-routing-v1.8.53.md"),
        ),
        (
            "references/governance.md",
            include_str!("fixtures/bootstrap-governance-v1.8.53.md"),
        ),
        (
            "references/mutation.md",
            include_str!("fixtures/bootstrap-mutation-v1.8.53.md"),
        ),
    ];
    for (path, content) in old {
        fs::write(package.join(path), content).unwrap();
    }
    run(&temp, &["check"], None);
    let setup = run(&temp, &["setup", "--agent", "codex"], None);
    assert_eq!(setup["result"]["state"], "preview_ready");
    assert_eq!(setup["result"]["targets"][0]["status"], "official_outdated");
    assert_eq!(setup["result"]["targets"][0]["installed_version"], "1.8.53");
    let applied = run(
        &temp,
        &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        None,
    );
    assert_eq!(
        fs::read_to_string(package.join("SKILL.md")).unwrap(),
        include_str!("../skill/skillroster/SKILL.md")
    );
    run(
        &temp,
        &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        None,
    );
    for (path, content) in old {
        assert_eq!(fs::read_to_string(package.join(path)).unwrap(), content);
    }
}

#[cfg(unix)]
fn tty_init(temp: &TempDir, key: Option<u8>, extra: &[&str]) -> (bool, String) {
    tty_init_input(temp, key.as_ref().map(std::slice::from_ref), extra)
}

#[cfg(unix)]
fn tty_init_input(temp: &TempDir, input: Option<&[u8]>, extra: &[&str]) -> (bool, String) {
    let attempts = if input.is_some_and(|input| input.len() > 1 && input[0] == 27) {
        5
    } else {
        1
    };
    for _ in 0..attempts {
        if let Some(result) = tty_init_attempt(temp, input, extra) {
            return result;
        }
    }
    panic!("TTY test could not deliver the split Escape sequence within 25 ms");
}

#[cfg(unix)]
fn tty_init_attempt(
    temp: &TempDir,
    input: Option<&[u8]>,
    extra: &[&str],
) -> Option<(bool, String)> {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::time::{Duration, Instant};
    let mut master = -1;
    let mut slave = -1;
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    let mut master = unsafe { fs::File::from_raw_fd(master) };
    let slave = unsafe { fs::File::from_raw_fd(slave) };
    let mut shell_input = slave.try_clone().unwrap();
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    assert_eq!(
        unsafe { libc::tcgetattr(master.as_raw_fd(), &mut original) },
        0
    );
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
    }
    let mut child = command(temp)
        .args(["init", "--agent", "codex"])
        .args(extra)
        .env("NO_COLOR", "1")
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave)
        .spawn()
        .unwrap();
    let mut output = Vec::new();
    let mut answered = false;
    let mut timely_input = true;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let mut bytes = [0; 8192];
        while let Ok(count) = master.read(&mut bytes) {
            if count == 0 {
                break;
            }
            output.extend_from_slice(&bytes[..count]);
        }
        if !answered && String::from_utf8_lossy(&output).contains("Enter confirm") {
            let input = input.expect("unexpected confirmation");
            let sent_at = Instant::now();
            master.write_all(&input[..1]).unwrap();
            if input.len() > 1 {
                // Keep this tiny test-only gap off macOS's coalesced sleep timers.
                let tail_at = Instant::now() + Duration::from_millis(10);
                while Instant::now() < tail_at {
                    std::hint::spin_loop();
                }
                master.write_all(&input[1..]).unwrap();
                // A delayed test thread can resume after the CLI's 30 ms Escape
                // window. Retry only late delivery, never a timely input leak.
                if input[0] == 27 {
                    timely_input = sent_at.elapsed() <= Duration::from_millis(25);
                }
            }
            answered = true;
        }
        if let Some(status) = child.try_wait().unwrap() {
            while let Ok(count) = master.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                output.extend_from_slice(&bytes[..count]);
            }
            let mut restored = unsafe { std::mem::zeroed::<libc::termios>() };
            assert_eq!(
                unsafe { libc::tcgetattr(master.as_raw_fd(), &mut restored) },
                0
            );
            assert_eq!(restored.c_lflag, original.c_lflag);
            let mut inspect = restored;
            inspect.c_lflag &= !(libc::ICANON | libc::ECHO);
            inspect.c_cc[libc::VMIN] = 0;
            inspect.c_cc[libc::VTIME] = 0;
            assert_eq!(
                unsafe { libc::tcsetattr(shell_input.as_raw_fd(), libc::TCSANOW, &inspect) },
                0
            );
            let mut remaining = [0; 64];
            if timely_input {
                assert_eq!(
                    shell_input.read(&mut remaining).unwrap(),
                    0,
                    "terminal input leaked after confirmation"
                );
                return Some((status.success(), String::from_utf8(output).unwrap()));
            }
            // Drain this rejected trial's late bytes before closing its PTY.
            while shell_input.read(&mut remaining).unwrap() > 0 {}
            return None;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("TTY timeout: {}", String::from_utf8_lossy(&output));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn init_tty_escape_cancels_enter_applies_receipt_and_undo_restores_files() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("home/.codex/skills")).unwrap();
    let (ok, cancelled) = tty_init(&temp, Some(27), &[]);
    assert!(ok, "{cancelled}");
    assert!(cancelled.contains("Cancelled"));
    assert!(!temp.path().join("home/.codex/skills/skillroster").exists());
    let (ok, interrupted) = tty_init(&temp, Some(3), &[]);
    assert!(ok, "{interrupted}");
    assert!(interrupted.contains("Cancelled"));
    let (ok, applied) = tty_init(&temp, Some(b'\r'), &[]);
    assert!(ok, "{applied}");
    assert!(applied.contains("Copy to your Agent:"), "{applied}");
    assert!(applied.contains("verification") || applied.contains("Verification"));
    assert!(
        temp.path()
            .join("home/.codex/skills/skillroster/SKILL.md")
            .is_file()
    );
    let status = run(&temp, &["status"], None);
    let db = rusqlite::Connection::open(temp.path().join("state/skillroster.db")).unwrap();
    let receipt: String = db
        .query_row(
            "SELECT id FROM receipts WHERE status = 'applied'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status["result"]["snapshot_state"], "rescan_required");
    let (ok, current) = tty_init(&temp, None, &[]);
    assert!(ok, "{current}");
    assert!(current.contains("Up to date"));
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    run(&temp, &["undo", &receipt], None);
    assert!(!temp.path().join("home/.codex/skills/skillroster").exists());
}

#[cfg(unix)]
#[test]
fn init_preserves_modified_bootstrap_until_explicit_choice() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("home/.codex/skills/skillroster/SKILL.md");
    skill(
        &temp.path().join("home"),
        ".codex/skills/skillroster",
        "user-owned content\n",
    );
    let (ok, blocked) = tty_init(&temp, None, &[]);
    assert!(ok, "{blocked}");
    assert!(blocked.contains("--modified-choice retain-local"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "user-owned content\n");
    let (ok, retained) = tty_init(&temp, None, &["--modified-choice", "retain-local"]);
    assert!(ok, "{retained}");
    assert_eq!(fs::read_to_string(&file).unwrap(), "user-owned content\n");
}

#[cfg(unix)]
#[test]
fn init_discards_split_escape_and_arrow_sequences_before_returning_to_the_shell() {
    for sequence in [
        b"\x1b[A".as_slice(),
        b"\x1bOB",
        b"\x1b[1;5D",
        b"\x1b[200~pasted text\x1b[201~",
    ] {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("home/.codex/skills")).unwrap();
        let (ok, output) = tty_init_input(&temp, Some(sequence), &[]);
        assert!(ok && output.contains("Cancelled"), "{output}");
        assert!(!temp.path().join("home/.codex/skills/skillroster").exists());
        let db = rusqlite::Connection::open(temp.path().join("state/skillroster.db")).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
}
