use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::fs::{FileTimes, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use fs2::FileExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

fn run(args: &[&str], stdin: Option<&str>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skillroster"));
    command.args(args);
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn run_with_columns(args: &[&str], columns: usize) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .args(args)
        .env("COLUMNS", columns.to_string())
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn executable_copy_is_busy(error: &std::io::Error) -> bool {
    #[cfg(target_os = "linux")]
    {
        const ETXTBSY: i32 = 26;
        error.raw_os_error() == Some(ETXTBSY)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = error;
        false
    }
}

fn output_after_executable_copy(command: &mut Command) -> std::process::Output {
    const MAX_ATTEMPTS: usize = 5;

    for attempt in 1..=MAX_ATTEMPTS {
        match command.output() {
            Ok(output) => return output,
            // Linux CI can briefly retain a write lease after publishing a
            // copied executable. Retry only ETXTBSY; every other spawn error
            // and a persistent lease still fail the test immediately.
            Err(error) if executable_copy_is_busy(&error) && attempt < MAX_ATTEMPTS => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("cannot run copied test executable: {error}"),
        }
    }

    unreachable!("the bounded executable-copy retry loop always returns or panics")
}

fn continuation_argv(output: &str) -> Vec<String> {
    let mut argv = Vec::new();
    let mut current = None::<String>;
    let mut in_continuation = false;
    for line in output.lines() {
        if line == "Continue argv:" {
            in_continuation = true;
            continue;
        }
        if !in_continuation || line.trim().is_empty() {
            continue;
        }
        let trimmed = line.trim_start();
        let payload = if let Some(close) = trimmed
            .strip_prefix('[')
            .and_then(|line| line.find(']').map(|close| (line, close)))
        {
            if let Some(argument) = current.take() {
                argv.push(argument);
            }
            close.0[close.1 + 1..].trim_start()
        } else if trimmed.starts_with('"') {
            trimmed
        } else {
            break;
        };
        let chunk = payload.strip_suffix(" +").unwrap_or(payload);
        let decoded: String = serde_json::from_str(chunk).unwrap();
        current.get_or_insert_with(String::new).push_str(&decoded);
    }
    if let Some(argument) = current {
        argv.push(argument);
    }
    argv
}

#[cfg(unix)]
fn run_with_umask(args: &[&str], umask: libc::mode_t) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skillroster"));
    command.args(args);
    // SAFETY: this callback runs after fork and before exec, calls only the
    // async-signal-safe umask syscall, and captures a Copy integer.
    unsafe {
        command.pre_exec(move || {
            libc::umask(umask);
            Ok(())
        });
    }
    command.output().unwrap()
}

fn json_output(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn skill_evidence_id(database: &rusqlite::Connection, snapshot_id: &str, skill_id: &str) -> String {
    database
        .query_row(
            "SELECT id FROM evidence
             WHERE scan_id = ?1 AND subject_type = 'skill' AND subject_id = ?2
             ORDER BY id LIMIT 1",
            rusqlite::params![snapshot_id, skill_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|error| panic!("missing Evidence for Skill {skill_id}: {error}"))
}

fn assert_setup_versions(output: &Value) {
    assert_eq!(output["result"]["cli_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(output["result"]["bootstrap_content_version"], "1.8.29");
    assert_eq!(output["result"]["bootstrap_version"], "1.8.29");
}

#[cfg(unix)]
#[test]
fn local_state_is_private_even_with_a_permissive_umask() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let args = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
        "status",
    ];

    json_output(&run_with_umask(&args, 0));

    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for name in ["skillroster.db", "write.lock"] {
        assert_eq!(
            fs::metadata(state.join(name)).unwrap().permissions().mode() & 0o777,
            0o600,
            "unexpected permissions for {name}"
        );
    }

    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(
        state.join("skillroster.db"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    fs::set_permissions(state.join("write.lock"), fs::Permissions::from_mode(0o644)).unwrap();

    json_output(&run(&args, None));

    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(state.join("skillroster.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(state.join("write.lock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn lifecycle_delete_refuses_a_symlink_state_root_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), "keep").unwrap();
    let state = temp.path().join("state");
    symlink(&outside, &state).unwrap();
    let output = run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "lifecycle",
            "delete",
            "--confirm",
            "DELETE-LOCAL-STATE",
        ],
        None,
    );

    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(outside.join("sentinel")).unwrap(),
        "keep"
    );
    assert!(!outside.join("skillroster.db").exists());
    assert!(!outside.join("write.lock").exists());
}

#[cfg(unix)]
#[test]
fn startup_refuses_database_and_lock_symlinks_before_opening_them() {
    use std::os::unix::fs::symlink;

    for name in ["skillroster.db", "write.lock"] {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let outside = temp.path().join(format!("outside-{name}"));
        symlink(&outside, state.join(name)).unwrap();

        let output = run(
            &[
                "--home",
                home.to_str().unwrap(),
                "--state-dir",
                state.to_str().unwrap(),
                "--json",
                "status",
            ],
            None,
        );

        assert!(!output.status.success(), "{name} symlink was accepted");
        assert!(!outside.exists(), "{name} symlink target was created");
    }
}

#[cfg(unix)]
#[test]
fn startup_leaves_unrecognized_control_files_untouched() {
    use std::os::unix::fs::PermissionsExt;

    let cases = [
        ("receipts", "notes.txt", "user note"),
        (
            "source-confirmation",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV.json",
            "{}",
        ),
    ];
    for (directory, name, content) in cases {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        let control = state.join(directory);
        fs::create_dir_all(&control).unwrap();
        let unknown = control.join(name);
        fs::write(&unknown, content).unwrap();
        fs::set_permissions(&unknown, fs::Permissions::from_mode(0o644)).unwrap();

        let output = run(
            &[
                "--home",
                home.to_str().unwrap(),
                "--state-dir",
                state.to_str().unwrap(),
                "--json",
                "status",
            ],
            None,
        );

        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(&unknown).unwrap(), content);
        assert_eq!(
            fs::metadata(&unknown).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
}

fn context_action_argv(home: &Path, state: &Path, tail: &[&str]) -> Value {
    let mut argv = vec![
        env!("CARGO_BIN_EXE_skillroster").to_owned(),
        "--state-dir".to_owned(),
        state.to_string_lossy().into_owned(),
        "--home".to_owned(),
        home.to_string_lossy().into_owned(),
    ];
    argv.extend(tail.iter().map(|value| (*value).to_owned()));
    json!(argv)
}

fn run_suggested_action(action: &Value) -> std::process::Output {
    let argv = action["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        argv.first().map(String::as_str),
        Some(env!("CARGO_BIN_EXE_skillroster"))
    );
    Command::new(&argv[0]).args(&argv[1..]).output().unwrap()
}

#[test]
fn suggested_action_argv_stays_bound_to_the_running_executable() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let shadow_bin = temp.path().join("shadow-bin");
    fs::create_dir_all(&shadow_bin).unwrap();
    let shadow_name = if cfg!(windows) {
        "skillroster.exe"
    } else {
        "skillroster"
    };
    fs::write(shadow_bin.join(shadow_name), b"not the running executable").unwrap();

    let archive = temp.path().join("skillroster-1.8.31-test-target");
    fs::create_dir(&archive).unwrap();
    let executable = archive.join(shadow_name);
    let staged_executable = archive.join(format!("staged-{shadow_name}"));
    fs::copy(env!("CARGO_BIN_EXE_skillroster"), &staged_executable).unwrap();
    fs::rename(staged_executable, &executable).unwrap();
    let executable = executable.to_str().unwrap();
    let mut initial_command = Command::new(executable);
    initial_command
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--home",
            home.to_str().unwrap(),
            "--json",
        ])
        .env("PATH", &shadow_bin);
    let initial = json_output(&output_after_executable_copy(&mut initial_command));
    let argv = initial["suggested_actions"][0]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(argv.first().copied(), Some(executable));
    let continued = json_output(
        &Command::new(argv[0])
            .args(&argv[1..])
            .env("PATH", &shadow_bin)
            .output()
            .unwrap(),
    );
    assert_eq!(continued["command"], "scan");
    assert_eq!(continued["ok"], true);

    let human = Command::new(executable)
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--home",
            home.to_str().unwrap(),
        ])
        .env("PATH", &shadow_bin)
        .output()
        .unwrap();
    assert!(human.status.success());
    let human_argv = continuation_argv(&String::from_utf8(human.stdout).unwrap());
    assert_eq!(human_argv.first().map(String::as_str), Some(executable));

    let report_argv = continued["suggested_actions"][0]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(report_argv.first().copied(), Some(executable));
    let report = json_output(
        &Command::new(report_argv[0])
            .args(&report_argv[1..])
            .env("PATH", &shadow_bin)
            .output()
            .unwrap(),
    );
    assert_eq!(report["command"], "report");
    assert_eq!(report["ok"], true);
}

#[cfg(target_os = "linux")]
#[test]
fn non_unicode_running_executable_fails_closed_without_a_path_fallback() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let executable = temp
        .path()
        .join(OsString::from_vec(b"skillroster-\xff".to_vec()));
    let staged_executable = temp.path().join("staged-skillroster");
    fs::copy(env!("CARGO_BIN_EXE_skillroster"), &staged_executable).unwrap();
    fs::rename(staged_executable, &executable).unwrap();

    let output = Command::new(&executable)
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--home",
            home.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let failure: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(failure["ok"], false);
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains("current executable path must be valid Unicode")
    );
    assert_eq!(failure["suggested_actions"], json!([]));
    assert!(!state.exists());
}

#[test]
fn suggested_action_argv_replays_the_same_discovery_context() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let agent_root = temp.path().join("agent-root");
    let source_root = temp.path().join("source-root");
    fs::create_dir_all(agent_root.join("visible-skill")).unwrap();
    fs::create_dir_all(source_root.join("library-skill")).unwrap();
    fs::write(
        agent_root.join("visible-skill/SKILL.md"),
        "---\nname: visible-skill\ndescription: visible fixture\n---\n",
    )
    .unwrap();
    fs::write(
        source_root.join("library-skill/SKILL.md"),
        "---\nname: library-skill\ndescription: library fixture\n---\n",
    )
    .unwrap();
    let root = format!("codex={}", agent_root.display());

    let scan = json_output(&run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--root",
            &root,
            "--source-root",
            source_root.to_str().unwrap(),
            "--json",
            "scan",
        ],
        None,
    ));
    assert_eq!(
        scan["suggested_actions"][0]["argv"],
        json!([
            env!("CARGO_BIN_EXE_skillroster"),
            "--state-dir",
            state,
            "--home",
            home,
            "--root",
            root,
            "--source-root",
            source_root,
            "report",
            "--json"
        ])
    );

    let report = json_output(&run_suggested_action(&scan["suggested_actions"][0]));
    assert_eq!(
        report["result"]["snapshot_id"],
        scan["result"]["snapshot_id"]
    );
    assert_eq!(report["result"]["skill_count"], 2);
}

#[test]
fn scan_summary_preserves_report_facts_and_bounds_agent_context() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let full_state = temp.path().join("full-state");
    let summary_state = temp.path().join("summary-state");
    let agent_roots = [
        (".codex/skills", ".codex/sessions"),
        (".claude/skills", ".claude/projects"),
        (".pi/agent/skills", ".pi/agent/sessions"),
        (
            ".config/opencode/skills",
            ".local/share/opencode/storage/session",
        ),
        (".hermes/skills", ".hermes/sessions"),
        (".cursor/skills", ".cursor/projects"),
        (".gemini/skills", ".gemini/tmp"),
        (".copilot/skills", ".copilot/session-state"),
    ];
    for (agent_index, (skill_root, session_root)) in agent_roots.iter().enumerate() {
        for skill_index in 0..8 {
            let name = format!("fixture-{agent_index}-{skill_index}");
            let skill = home.join(skill_root).join(&name);
            fs::create_dir_all(&skill).unwrap();
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: Representative task helper\n---\n"),
            )
            .unwrap();
        }
        if agent_index < 7 {
            let sessions = home.join(session_root);
            fs::create_dir_all(&sessions).unwrap();
            fs::write(
                sessions.join("representative.jsonl"),
                "{\"type\":\"message\",\"content\":\"bounded fixture\"}\n",
            )
            .unwrap();
        }
    }

    let scan_args = |state: &Path, summary: bool| {
        let mut args = vec![
            "--home".to_owned(),
            home.to_string_lossy().into_owned(),
            "--state-dir".to_owned(),
            state.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ];
        for index in 0..12 {
            args.push("--root".to_owned());
            args.push(format!(
                "codex={}",
                temp.path().join(format!("missing-{index}")).display()
            ));
        }
        args.push("scan".to_owned());
        if summary {
            args.push("--summary".to_owned());
        }
        args
    };
    let full_args = scan_args(&full_state, false);
    let summary_args = scan_args(&summary_state, true);
    let full_output = run(
        &full_args.iter().map(String::as_str).collect::<Vec<_>>(),
        None,
    );
    let summary_output = run(
        &summary_args.iter().map(String::as_str).collect::<Vec<_>>(),
        None,
    );
    let full = json_output(&full_output);
    let summary = json_output(&summary_output);

    assert!(full["result"]["roots"].is_array());
    assert!(full["result"]["coverage"].is_array());
    assert_eq!(summary["result"]["view"], "summary");
    assert!(summary["result"].get("roots").is_none());
    assert!(summary["result"].get("coverage").is_none());
    assert_eq!(summary["result"]["root_issues"]["returned"], 10);
    assert_eq!(summary["result"]["root_issues"]["truncated"], true);
    assert!(summary["result"]["root_issues"]["total"].as_u64().unwrap() > 10);
    assert_eq!(
        summary["result"]["session_coverage"]["inference_boundary"]["unused_claim_supported"],
        false
    );
    assert_eq!(
        summary["result"]["session_coverage"]["inference_boundary"]["automatic_governance_supported"],
        false
    );
    assert_eq!(summary["result"]["skill_count"], 64);
    assert_eq!(summary["result"]["placement_count"], 64);
    assert_eq!(
        summary["result"]["session_coverage"]["agents"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert!(
        summary["result"]["session_coverage"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|agent| agent["state"].is_string())
    );
    let missing_coverage = summary["result"]["session_coverage"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["state"] == "missing")
        .unwrap();
    assert_eq!(missing_coverage["agent"], "github-copilot");
    let missing_limitation = summary["result"]["session_coverage"]["limitation_groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|limitation| limitation["code"] == "root_missing")
        .unwrap();
    assert!(
        missing_limitation["agents"]
            .as_array()
            .unwrap()
            .contains(&json!("github-copilot"))
    );
    for field in ["scope", "count_kind", "observed", "unit", "source"] {
        assert!(
            missing_limitation.get(field).is_some(),
            "typed limitation field {field}"
        );
    }
    assert!(missing_limitation.get("limit").is_some());
    assert!(
        summary["result"]["session_coverage"]["next_step_groups"]
            .as_array()
            .unwrap()
            .iter()
            .any(|group| group["agents"]
                .as_array()
                .unwrap()
                .contains(&json!("github-copilot"))
                && group["steps"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("verify_session_root_before_rescan")))
    );
    assert!(
        summary_output.stdout.len() * 100 <= full_output.stdout.len() * 40,
        "summary={} full={}",
        summary_output.stdout.len(),
        full_output.stdout.len()
    );

    let full_report = json_output(&run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            full_state.to_str().unwrap(),
            "--json",
            "report",
        ],
        None,
    ));
    let summary_report = json_output(&run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            summary_state.to_str().unwrap(),
            "--json",
            "report",
        ],
        None,
    ));
    let full_payload: String = rusqlite::Connection::open(full_state.join("skillroster.db"))
        .unwrap()
        .query_row("SELECT payload_json FROM scan_payloads", [], |row| {
            row.get(0)
        })
        .unwrap();
    let summary_payload: String = rusqlite::Connection::open(summary_state.join("skillroster.db"))
        .unwrap()
        .query_row("SELECT payload_json FROM scan_payloads", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(summary_payload, full_payload);

    let mut full_report_result = full_report["result"].clone();
    let mut summary_report_result = summary_report["result"].clone();
    for result in [&mut full_report_result, &mut summary_report_result] {
        result.as_object_mut().unwrap().remove("report_id");
        result.as_object_mut().unwrap().remove("snapshot_id");
        for finding in result["findings"].as_array_mut().unwrap() {
            finding.as_object_mut().unwrap().remove("id");
            finding
                .as_object_mut()
                .unwrap()
                .remove("primary_evidence_id");
        }
    }
    assert_eq!(summary_report_result, full_report_result);
}

#[test]
fn report_help_names_the_safe_default_and_explicit_exhaustive_export() {
    let output = run(&["report", "--help"], None);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(
        help.contains("Defaults to the bounded Summary view"),
        "{help}"
    );
    assert!(help.contains("--full"), "{help}");
    assert!(help.contains("exhaustive report"), "{help}");
    assert!(
        help.contains("Defaults to 5 for compact detail and 20 otherwise"),
        "{help}"
    );
}

#[test]
fn semantic_overlap_detail_is_decision_complete_for_agent_comparison() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = temp.path().join("skills");
    let left_path = root.join("browser-tabs/SKILL.md");
    let right_path = root.join("browser-session/SKILL.md");
    let long_description = " authenticated browser context".repeat(80);
    let long_source = "source-system-".repeat(80);
    let triggers = (0..12)
        .map(|index| format!("inspect-page-{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let left = format!(
        "---\nname: 浏览器标签\ndescription: Control existing logged in browser tabs and inspect page state{long_description}\nsource: {long_source}\ntriggers: [{triggers}]\n---\nOperate visible tabs safely.\n"
    );
    let right = format!(
        "---\nname: 浏览器会话\ndescription: Inspect existing logged in browser session tabs and page state{long_description}\nsource: {long_source}\ntriggers: [{triggers}]\n---\nReview authenticated browser state.\n"
    );
    fs::create_dir_all(left_path.parent().unwrap()).unwrap();
    fs::create_dir_all(right_path.parent().unwrap()).unwrap();
    fs::write(&left_path, &left).unwrap();
    fs::write(&right_path, &right).unwrap();
    let mut explicit_roots = vec![format!("codex={}", root.display())];
    for (index, agent) in ["claude-code", "pi", "opencode", "hermes", "cursor"]
        .into_iter()
        .enumerate()
    {
        let extra_root = temp.path().join(format!("skills-{index}"));
        let extra_left = extra_root.join("browser-tabs/SKILL.md");
        let extra_right = extra_root.join("browser-session/SKILL.md");
        fs::create_dir_all(extra_left.parent().unwrap()).unwrap();
        fs::create_dir_all(extra_right.parent().unwrap()).unwrap();
        fs::write(extra_left, &left).unwrap();
        fs::write(extra_right, &right).unwrap();
        explicit_roots.push(format!("{agent}={}", extra_root.display()));
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let mut scan_args = common.to_vec();
    for explicit_root in &explicit_roots {
        scan_args.extend(["--root", explicit_root]);
    }
    scan_args.push("scan");
    json_output(&run(&scan_args, None));
    let findings = json_output(&run(
        &[
            &common[..],
            &[
                "report",
                "--findings",
                "--category",
                "overlap",
                "--severity",
                "low",
                "--limit",
                "100",
            ],
        ]
        .concat(),
        None,
    ));
    let finding_id = findings["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Semantic overlap candidate")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    {
        let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
        let encoded: String = database
            .query_row(
                "SELECT details_json FROM findings WHERE id = ?1",
                [finding_id],
                |row| row.get(0),
            )
            .unwrap();
        let mut details: Value = serde_json::from_str(&encoded).unwrap();
        details["title"] = json!("Copy-edited semantic comparison title");
        database
            .execute(
                "UPDATE findings SET title = ?1, details_json = ?2 WHERE id = ?3",
                (
                    "Copy-edited semantic comparison title",
                    details.to_string(),
                    finding_id,
                ),
            )
            .unwrap();
    }
    let compact = json_output(&run(
        &[
            &common[..],
            &["report", "--finding", finding_id, "--limit", "1"],
        ]
        .concat(),
        None,
    ));
    let comparison = &compact["result"]["comparison"];
    assert_eq!(comparison["decision"], "compare_skill_meaning");
    assert_eq!(comparison["automatic_change_supported"], false);
    assert_eq!(comparison["semantic_conclusion_owner"], "agent_or_user");
    assert_eq!(comparison["basis"]["metric"], "routing_vocabulary_jaccard");
    assert!(comparison["basis"]["score"].as_f64().unwrap() >= 0.45);
    assert!(comparison["basis"]["intersection_count"].as_u64().unwrap() >= 3);
    assert!(
        comparison["basis"]["union_count"].as_u64().unwrap()
            >= comparison["basis"]["intersection_count"].as_u64().unwrap()
    );
    assert!(
        comparison["basis"]["shared_terms"]
            .as_array()
            .unwrap()
            .len()
            <= 20
    );
    let skills = comparison["skills"].as_array().unwrap();
    assert_eq!(skills.len(), 2);
    let mut names = skills
        .iter()
        .map(|skill| skill["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, ["浏览器会话", "浏览器标签"]);
    assert!(skills.iter().all(|skill| {
        skill["description"].as_str().unwrap().chars().count() == 512
            && skill["description_truncated"] == true
            && skill["trigger_count"] == 12
            && skill["triggers"].as_array().unwrap().len() == 10
            && skill["triggers_truncated"] == true
            && skill["source"].as_str().unwrap().chars().count() == 256
            && skill["source_truncated"] == true
            && skill["summary"].as_str().is_some()
            && skill["agents"]
                == json!(["claude-code", "codex", "cursor", "hermes", "opencode", "pi"])
            && skill["agent_count"] == 6
            && skill["agents_truncated"] == false
            && skill["providers"] == json!([])
            && skill["provider_count"] == 0
            && skill["providers_truncated"] == false
            && skill["governable"] == true
            && skill["readable_path_count"] == 6
            && skill["readable_paths_truncated"] == true
            && skill["current_content_available"] == true
            && skill["readable_paths"].as_array().unwrap().len() == 5
            && skill["readable_placement_count"] == 6
            && skill["readable_placements_truncated"] == false
            && skill["readable_placements"].as_array().unwrap().len() == 6
            && skill["readable_placements"][0]["provider"].is_null()
            && skill["readable_placements"][0]["provider_truncated"] == false
            && skill["readable_placements"][0]["path"]
                .as_str()
                .is_some_and(|path| path.ends_with("SKILL.md"))
    }));
    assert!(
        compact["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );
    let compact_full_action = compact["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "show_full_finding")
        .unwrap();
    assert_eq!(
        compact_full_action["argv"],
        context_action_argv(
            &home,
            &state,
            &[
                "report",
                "--finding",
                finding_id,
                "--full",
                "--limit",
                "1",
                "--json",
            ]
        )
    );
    let full = json_output(&run(
        &[
            &common[..],
            &["report", "--finding", finding_id, "--full", "--limit", "1"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(
        full["result"]["affected_skill_ids"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        full["result"]["comparison"]["skills"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let complete_full = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id, "--full"]].concat(),
        None,
    ));
    let mut expected_skill_ids = complete_full["result"]["affected_skill_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    expected_skill_ids.sort_unstable();
    let mut actual_skill_ids = skills
        .iter()
        .map(|skill| skill["skill_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    actual_skill_ids.sort_unstable();
    assert_eq!(actual_skill_ids, expected_skill_ids);
    assert_eq!(
        full["result"]["comparison"],
        compact["result"]["comparison"]
    );
    assert_eq!(full["result"]["files_changed"], false);
    assert_eq!(fs::read_to_string(left_path).unwrap(), left);
    assert_eq!(fs::read_to_string(right_path).unwrap(), right);
}

#[test]
fn healthy_status_does_not_suggest_an_unconditional_rescan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/skills")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let missing_snapshot = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(
        missing_snapshot["suggested_actions"][0]["argv"],
        context_action_argv(&home, &state, &["scan", "--summary", "--json"])
    );
    assert_eq!(
        missing_snapshot["suggested_actions"][0]["reason_code"],
        "snapshot_required"
    );

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    json_output(&run(&[&common[..], &["report"]].concat(), None));
    let healthy = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert!(healthy["result"]["latest_snapshot_id"].is_string());
    assert_eq!(healthy["result"]["recovery_state"], "clear");
    assert_eq!(healthy["suggested_actions"], json!([]));
}

#[test]
fn home_and_status_share_the_missing_snapshot_scan_continuation() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home with spaces");
    let state = temp.path().join("state with spaces");
    fs::create_dir_all(&home).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let home_result = json_output(&run(&common, None));
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    let expected = context_action_argv(&home, &state, &["scan", "--summary", "--json"]);

    assert_eq!(home_result["result"]["state"], "no_snapshot");
    assert_eq!(home_result["result"]["snapshot_state"], "missing");
    assert_eq!(
        home_result["suggested_actions"],
        status["suggested_actions"]
    );
    assert_eq!(home_result["suggested_actions"][0]["argv"], expected);
    assert_eq!(home_result["suggested_actions"][0]["mutates"], false);
    assert_eq!(
        home_result["suggested_actions"][0]["requires_confirmation"],
        false
    );

    let human = run_with_columns(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
        ],
        60,
    );
    assert!(human.status.success());
    let human = String::from_utf8(human.stdout).unwrap();
    let expected_argv = expected
        .as_array()
        .unwrap()
        .iter()
        .map(|argument| argument.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(continuation_argv(&human), expected_argv);
    assert!(human.lines().all(|line| line.chars().count() <= 60));
    let human_status = run_with_columns(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "status",
        ],
        60,
    );
    assert!(human_status.status.success());
    let human_status = String::from_utf8(human_status.stdout).unwrap();
    assert_eq!(continuation_argv(&human_status), expected_argv);
    assert!(
        human_status.lines().all(|line| line.chars().count() <= 60),
        "line exceeded 60 columns:\n{human_status}"
    );
}

#[test]
fn home_and_status_resume_the_first_value_flow_until_the_current_report_exists() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home with spaces");
    let state = temp.path().join("state with spaces");
    fs::create_dir_all(home.join(".codex/skills/example")).unwrap();
    fs::write(
        home.join(".codex/skills/example/SKILL.md"),
        "---\nname: example\ndescription: Example Skill\n---\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let initial = json_output(&run(&common, None));
    assert_eq!(initial["result"]["state"], "no_snapshot");
    assert_eq!(
        initial["suggested_actions"][0]["argv"],
        context_action_argv(&home, &state, &["scan", "--summary", "--json"])
    );
    let scan = json_output(&run_suggested_action(&initial["suggested_actions"][0]));
    assert_eq!(scan["result"]["files_changed"], false);

    let resumed_home = json_output(&run(&common, None));
    let resumed_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    let expected_report = context_action_argv(&home, &state, &["report", "--json"]);
    assert_eq!(resumed_home["result"]["state"], "report_required");
    assert_eq!(resumed_status["result"]["state"], "report_required");
    assert_eq!(
        resumed_home["suggested_actions"],
        resumed_status["suggested_actions"]
    );
    assert_eq!(
        resumed_home["suggested_actions"][0]["argv"],
        expected_report
    );
    assert_eq!(resumed_home["suggested_actions"][0]["mutates"], false);
    assert_eq!(resumed_home["result"]["files_changed"], false);
    assert_eq!(resumed_status["result"]["files_changed"], false);

    let report = json_output(&run_suggested_action(&resumed_home["suggested_actions"][0]));
    assert_eq!(
        report["result"]["snapshot_id"],
        scan["result"]["snapshot_id"]
    );
    assert_eq!(report["result"]["files_changed"], false);
    let complete = json_output(&run(&common, None));
    assert_eq!(complete["result"]["state"], "ready");
    assert_eq!(complete["suggested_actions"], json!([]));

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    database
        .execute("UPDATE scan_payloads SET updated_at = updated_at + 1", [])
        .unwrap();
    drop(database);
    let payload_changed = json_output(&run(&common, None));
    assert_eq!(payload_changed["result"]["state"], "report_required");
    json_output(&run_suggested_action(
        &payload_changed["suggested_actions"][0],
    ));
    let rebuilt = json_output(&run(&common, None));
    assert_eq!(rebuilt["result"]["state"], "ready");
    assert_eq!(rebuilt["suggested_actions"], json!([]));
}

#[test]
fn home_routes_a_current_snapshot_only_when_a_ready_plan_exists() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report_required = json_output(&run(&common, None));
    assert_eq!(report_required["result"]["state"], "report_required");

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    let plan_ready = json_output(&run(&common, None));
    assert_eq!(plan_ready["result"]["state"], "plan_ready");
    assert_eq!(plan_ready["result"]["snapshot_state"], "current");
    assert_eq!(plan_ready["result"]["pending_plan_count"], 1);
    assert_eq!(plan_ready["suggested_actions"], status["suggested_actions"]);
    assert_eq!(
        plan_ready["suggested_actions"][0]["argv"],
        context_action_argv(
            &home,
            &state,
            &[
                "plan",
                "--show",
                setup["result"]["plan_id"].as_str().unwrap(),
                "--json"
            ]
        )
    );
}

#[test]
fn home_and_status_prioritize_recovery_over_an_invalidated_snapshot() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    let applied_receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let applied_journal = state
        .join("receipts")
        .join(format!("{applied_receipt_id}.json"));
    let mut orphan: Value = serde_json::from_slice(&fs::read(applied_journal).unwrap()).unwrap();
    let orphan_id = "receipt_00000000000000000000000000";
    orphan["id"] = json!(orphan_id);
    orphan["status"] = json!("recovery_required");
    fs::write(
        state.join("receipts").join(format!("{orphan_id}.json")),
        serde_json::to_vec_pretty(&orphan).unwrap(),
    )
    .unwrap();

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    let home_result = json_output(&run(&common, None));
    assert_eq!(home_result["result"]["state"], "recovery_required");
    assert_eq!(home_result["result"]["snapshot_state"], "rescan_required");
    assert_eq!(status["result"]["snapshot_state"], "rescan_required");
    assert_eq!(
        status["result"]["snapshot_invalidated_by_receipt_id"],
        applied_receipt_id
    );
    assert_eq!(home_result["result"]["recovery_state"], "required");
    assert_eq!(
        home_result["suggested_actions"],
        status["suggested_actions"]
    );
    assert_eq!(
        home_result["suggested_actions"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        home_result["result"]["next_action"],
        home_result["suggested_actions"][0]
    );
    assert_eq!(
        home_result["suggested_actions"][0]["argv"],
        context_action_argv(&home, &state, &["lifecycle", "recovery", "--json"])
    );
    assert_eq!(
        home_result["suggested_actions"][0]["reason_code"],
        "recovery_required"
    );
}

#[test]
fn public_cli_exits_quietly_when_the_output_consumer_closes() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    for index in 0..80 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\ndescription: fixture\n---\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"][0]["id"].as_str().unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .args([&common[..], &["report", "--finding", finding_id, "--full"]].concat())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_find_paths_are_readable(found: &Value) {
    let paths = found["result"]["matches"][0]["paths"]
        .as_array()
        .expect("find paths");
    assert!(!paths.is_empty());
    for path in paths {
        fs::File::open(path.as_str().unwrap()).expect("find must return a readable SKILL.md");
    }
}

#[test]
fn public_find_keeps_the_user_task_and_uses_agent_retrieval_hints() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/archify");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: archify\ndescription: Create interactive architecture workflow diagrams\n---\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let unhinted = json_output(&run(
        &[&common[..], &["find", "把架构流程画成可交互图"]].concat(),
        None,
    ));
    assert!(unhinted["result"]["matches"].as_array().unwrap().is_empty());
    assert_eq!(unhinted["result"]["warnings"].as_array().unwrap().len(), 1);
    assert!(
        unhinted["result"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("--hint"))
    );

    let found = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "把架构流程画成可交互图",
                "--hint",
                "interactive architecture workflow diagram",
                "--hint",
                "  interactive architecture workflow diagram  ",
            ],
        ]
        .concat(),
        None,
    ));

    assert_eq!(found["result"]["task"], "把架构流程画成可交互图");
    assert_eq!(
        found["result"]["retrieval_hints"],
        json!(["interactive architecture workflow diagram"])
    );
    assert_eq!(found["result"]["matches"][0]["name"], "archify");
    assert_eq!(
        found["result"]["ranking_strategy"],
        "task_hint_reciprocal_rank_fusion"
    );
}

#[test]
fn public_find_hints_do_not_erase_a_native_task_match() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    let native = skill_root.join("humanizer-zh");
    fs::create_dir_all(&native).unwrap();
    fs::write(
        native.join("SKILL.md"),
        "---\nname: humanizer-zh\ndescription: 把中文文章改得更自然，更像人类写的。\n---\n保留原意并去掉机器表达。\n",
    )
    .unwrap();
    let session_miner = skill_root.join("agent-session-miner");
    fs::create_dir_all(&session_miner).unwrap();
    fs::write(
        session_miner.join("SKILL.md"),
        "---\nname: agent-session-miner\ndescription: Mine local AI coding-agent session history into redacted reusable content seeds. Humanization and writing patterns may be analyzed.\n---\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let task = "把这篇中文文章改得像人写的";
    let unhinted = json_output(&run(
        &[&common[..], &["find", task, "--limit", "3"]].concat(),
        None,
    ));
    assert!(
        unhinted["result"]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|matched| matched["name"] == "humanizer-zh")
    );

    let hinted = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                task,
                "--hint",
                "humanize Chinese writing remove AI tone",
                "--limit",
                "3",
            ],
        ]
        .concat(),
        None,
    ));
    let native = &hinted["result"]["matches"][0];
    assert_eq!(native["name"], "humanizer-zh");
    assert_eq!(native["rank"], 1);
    assert_eq!(native["task_channel_rank"], 1);
    assert!(native["augmented_channel_rank"].is_number());
    assert!(native.get("ranking_adjustments").is_none());
    assert_eq!(
        hinted["result"]["ranking_strategy"],
        "task_hint_reciprocal_rank_fusion"
    );
}

#[test]
fn public_find_prefers_a_direct_single_token_name_hint_over_broad_native_overlap() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    for (directory, contents) in [
        (
            "pdf",
            "---\nname: pdf\ndescription: Read PDF files and inspect rendered layout on every page.\n---\n",
        ),
        (
            "yunxiao-smartcr",
            "---\nname: yunxiao-smartcr\ndescription: >\n  本地代码审查师（Code Reviewer），通过 AST 静态检查和 LLM 审查产出报告。\n  触发场景：审查代码变更、检查一下、看有没有问题、PR 检查。\n  不应触发：排障、性能优化建议、纯代码解释、单测生成。\n---\n读取变更并检查每一页报告的排版。\n",
        ),
    ] {
        let path = skill_root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), contents).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let found = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "读取这个 PDF 并检查每一页的排版有没有问题",
                "--hint",
                "read PDF inspect rendered layout on every page",
                "--limit",
                "3",
            ],
        ]
        .concat(),
        None,
    ));

    assert_eq!(found["result"]["matches"][0]["name"], "pdf");
    assert_eq!(found["result"]["matches"][0]["augmented_channel_rank"], 1);
}

#[test]
fn public_find_does_not_treat_part_of_a_multi_token_name_as_direct_hint_evidence() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    for (directory, contents) in [
        (
            "native-task",
            "---\nname: native-task\ndescription: 原始任务专用能力\n---\n",
        ),
        (
            "github-code-review",
            "---\nname: github-code-review\ndescription: Review documents, contracts, reports, plans, policies, requirements, evidence, and risks.\n---\n",
        ),
    ] {
        let path = skill_root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), contents).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let found = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "原始任务专用能力",
                "--hint",
                "review documents contracts reports plans policies requirements evidence risks",
                "--limit",
                "3",
            ],
        ]
        .concat(),
        None,
    ));

    assert_eq!(found["result"]["matches"][0]["name"], "native-task");
    assert_eq!(
        found["result"]["matches"][0]["ranking_adjustments"],
        json!(["protected_original_task_match"])
    );
}

#[test]
fn public_find_respects_a_chinese_do_not_use_clause() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    for (directory, contents) in [
        (
            "diagnose",
            "---\nname: diagnose\ndescription: Diagnose intermittent failures, reproduce them, and identify root causes.\n---\n",
        ),
        (
            "yunxiao-smartcr",
            "---\nname: yunxiao-smartcr\ndescription: >\n  本地代码审查师。触发场景：审查代码变更、检查一下、看有没有问题。\n  不应触发：偶现失败、复现并定位、排障、为什么会崩、报错是什么意思。\n---\n",
        ),
    ] {
        let path = skill_root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), contents).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let found = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "这个偶现失败到底是什么原因，帮我复现并定位",
                "--hint",
                "diagnose intermittent failure reproduce and identify root cause",
                "--limit",
                "3",
            ],
        ]
        .concat(),
        None,
    ));

    let names = found["result"]["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|matched| matched["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names[0], "diagnose");
    assert!(!names.contains(&"yunxiao-smartcr"));
}

#[test]
fn public_find_keeps_a_positive_cjk_clause_after_an_exclusion() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/mixed-routing");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: mixed-routing\ndescription: 适用于代码审查。不应触发：排障。也适用于事故复盘。\n---\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let found = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "事故复盘",
                "--hint",
                "incident retrospective",
                "--limit",
                "1",
            ],
        ]
        .concat(),
        None,
    ));

    assert_eq!(found["result"]["matches"][0]["name"], "mixed-routing");
}

#[test]
fn public_find_hinted_ranking_is_prefix_stable_across_limits() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    for (directory, contents) in [
        (
            "analyze-data-quality",
            "---\nname: analyze-data-quality\ndescription: Assess whether structured data and analytical evidence are trustworthy. Use when the task is to check data quality.\n---\n",
        ),
        (
            "spreadsheets",
            "---\nname: Spreadsheets\ndescription: Create, edit, analyze, and verify standalone spreadsheet files and workbooks, including Excel files.\n---\n",
        ),
        (
            "local-data-quality",
            "---\nname: local-data-quality\ndescription: 分析本地表格数据和数据质量。\n---\n",
        ),
        (
            "local-report-storage",
            "---\nname: local-report-storage\ndescription: 管理本地表格数据和数据质量报告文件。\n---\n",
        ),
    ] {
        let path = skill_root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), contents).unwrap();
    }
    for index in 0..24 {
        let path = skill_root.join(format!("spreadsheet-helper-{index:02}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!(
                "---\nname: spreadsheet-helper-{index:02}\ndescription: Generic spreadsheet helper number {index}.\n---\n"
            ),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let task = "分析一个本地表格的数据质量";
    let hint = "analyze standalone spreadsheet workbook data quality";
    let find_names = |limit: &str| {
        let found = json_output(&run(
            &[
                &common[..],
                &[
                    "find",
                    task,
                    "--hint",
                    hint,
                    "--hint",
                    "Spreadsheets",
                    "--limit",
                    limit,
                ],
            ]
            .concat(),
            None,
        ));
        found["result"]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|matched| matched["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    let complete_ranking = find_names("10");
    assert_eq!(complete_ranking[0], "Spreadsheets");
    assert!(
        complete_ranking[..3]
            .iter()
            .any(|name| name == "local-data-quality")
    );
    assert!(complete_ranking.iter().any(|name| name == "Spreadsheets"));
    assert!(
        complete_ranking
            .iter()
            .any(|name| name == "analyze-data-quality")
    );
    for limit in 1..=10 {
        let bounded = find_names(&limit.to_string());
        assert_eq!(bounded, complete_ranking[..bounded.len()]);
    }
}

#[test]
fn public_find_routes_natural_cjk_paraphrases_against_cjk_metadata() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    let humanizer = skill_root.join("humanizer-zh");
    let unrelated = skill_root.join("generic-writer");
    fs::create_dir_all(&humanizer).unwrap();
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(
        humanizer.join("SKILL.md"),
        "---\nname: humanizer-zh\ndescription: 去除文本中的 AI 生成痕迹，让中文表达更自然、更像人类书写。\n---\n编辑中文并保留原意。\n",
    )
    .unwrap();
    fs::write(
        unrelated.join("SKILL.md"),
        "---\nname: generic-writer\ndescription: Generic English writing helper\n---\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let found = json_output(&run(
        &[&common[..], &["find", "把中文改自然一点", "--limit", "3"]].concat(),
        None,
    ));

    assert_eq!(found["result"]["task"], "把中文改自然一点");
    assert_eq!(found["result"]["matches"][0]["name"], "humanizer-zh");
    assert!(
        found["result"]["matches"][0]["match_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason
                .as_str()
                .unwrap()
                .starts_with("cjk_description_bigrams:"))
    );
    assert_find_paths_are_readable(&found);
}

#[test]
fn public_find_expands_plural_candidates_and_bounds_incidental_single_token_matches() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    let blog = skill_root.join("blog");
    fs::create_dir_all(&blog).unwrap();
    fs::write(
        blog.join("SKILL.md"),
        "---\nname: blog\ndescription: Publish a technical article\n---\n",
    )
    .unwrap();
    for index in 0..8 {
        let skill = skill_root.join(format!("incidental-{index}"));
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!(
                "---\nname: incidental-{index}\ndescription: Generic helper\n---\nMentions archive incidentally.\n"
            ),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let plural = json_output(&run(
        &[&common[..], &["find", "blogs", "--limit", "10"]].concat(),
        None,
    ));
    assert_eq!(plural["result"]["matches"][0]["name"], "blog");

    let incidental = json_output(&run(
        &[&common[..], &["find", "archive", "--limit", "100"]].concat(),
        None,
    ));
    assert_eq!(incidental["result"]["matches"].as_array().unwrap().len(), 3);
}

#[test]
fn finding_drilldown_is_bounded_and_pageable() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    for index in 0..51 {
        let directory = skill_root.join(format!("skill-{index:02}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!(
                "---\nname: skill-{index:02}\ndescription: Dedicated task number {index}\n---\n"
            ),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let invalid_full = run(
        &[&common[..], &["report", "--full", "--summary"]].concat(),
        None,
    );
    assert!(!invalid_full.status.success());
    let invalid_full: Value = serde_json::from_slice(&invalid_full.stdout).unwrap();
    assert_eq!(invalid_full["error"]["code"], "invalid_cli_arguments");
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["category"] == "exposure")
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let first_output = run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    );
    assert!(first_output.stdout.len() < 20_000);
    let first = json_output(&first_output);
    assert_eq!(first["result"]["page"]["offset"], 0);
    assert_eq!(first["result"]["page"]["limit"], 5);
    assert_eq!(first["result"]["page"]["next_offset"], 5);
    assert_eq!(first["result"]["items"].as_array().unwrap().len(), 5);
    assert_eq!(first["result"]["detail"]["mode"], "compact");
    assert!(first["result"].get("placements").is_none());
    assert!(first["result"].get("affected_placement_ids").is_none());
    assert!(first["result"].get("evidence_ids").is_none());
    assert_eq!(
        first["suggested_actions"][0]["action"],
        "list_more_finding_detail"
    );
    assert_eq!(
        first["suggested_actions"][0]["argv"],
        context_action_argv(
            &home,
            &state,
            &[
                "report",
                "--finding",
                finding_id,
                "--limit",
                "5",
                "--offset",
                "5",
                "--json",
            ]
        )
    );

    let second = json_output(&run(
        &[
            &common[..],
            &[
                "report",
                "--finding",
                finding_id,
                "--offset",
                "20",
                "--limit",
                "20",
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(second["result"]["page"]["offset"], 20);
    assert_eq!(second["result"]["page"]["limit"], 20);
    assert_eq!(second["result"]["items"].as_array().unwrap().len(), 20);
    assert_ne!(
        first["result"]["items"][0]["evidence_id"],
        second["result"]["items"][0]["evidence_id"]
    );

    let full_action = first["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "show_full_finding")
        .unwrap();
    assert_eq!(
        full_action["argv"],
        context_action_argv(
            &home,
            &state,
            &["report", "--finding", finding_id, "--full", "--json"]
        )
    );
    let full_output = run_suggested_action(full_action);
    let full = json_output(&full_output);
    assert_eq!(full["result"]["detail"]["mode"], "full");
    assert_eq!(full["result"]["page"]["limit"], 20);
    assert!(full["result"]["placements"].is_array());
    assert!(full["result"]["evidence"].is_array());
    assert!(full["result"]["affected_placement_ids"].is_array());
    assert!(full_output.stdout.len() > first_output.stdout.len());
    assert_eq!(
        full["suggested_actions"][0]["action"],
        "list_more_finding_detail"
    );
    assert_eq!(
        full["suggested_actions"][0]["argv"],
        context_action_argv(
            &home,
            &state,
            &[
                "report",
                "--finding",
                finding_id,
                "--full",
                "--limit",
                "20",
                "--offset",
                "20",
                "--json",
            ]
        )
    );
}

#[test]
fn finding_list_is_paged_filterable_and_leads_to_evidence() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    for index in 0..30 {
        let content = format!(
            "---\nname: duplicate-{index:02}\ndescription: Dedicated duplicate task {index}\n---\n"
        );
        for root in [".codex/skills", ".claude/skills"] {
            let directory = home.join(root).join(format!("duplicate-{index:02}"));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("SKILL.md"), &content).unwrap();
        }
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let summary = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let default_report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    assert_eq!(
        default_report["result"]["findings"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        default_report["result"]["finding_count"],
        summary["result"]["finding_count"]
    );
    assert_eq!(
        default_report["result"]["finding_rollups"],
        summary["result"]["finding_rollups"]
    );
    assert_eq!(default_report["result"]["files_changed"], false);
    let exhaustive_report = json_output(&run(&[&common[..], &["report", "--full"]].concat(), None));
    assert_eq!(
        exhaustive_report["result"]["findings"]
            .as_array()
            .unwrap()
            .len(),
        60
    );
    assert_eq!(exhaustive_report["result"]["files_changed"], false);
    let duplicate_rollup = summary["result"]["finding_rollups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rollup| rollup["title"] == "Exact duplicate Skill placements")
        .unwrap();
    assert_eq!(duplicate_rollup["finding_count"], 30);
    assert_eq!(duplicate_rollup["affected_skill_count"], 30);
    assert_eq!(duplicate_rollup["affected_placement_count"], 60);
    assert_eq!(summary["suggested_actions"][0]["action"], "list_findings");
    assert_eq!(
        summary["suggested_actions"][0]["argv"],
        context_action_argv(
            &home,
            &state,
            &[
                "report",
                "--findings",
                "--limit",
                "20",
                "--offset",
                "0",
                "--json",
            ]
        )
    );
    assert_eq!(summary["suggested_actions"].as_array().unwrap().len(), 4);
    for (finding, suggested_action) in summary["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .zip(
            summary["suggested_actions"]
                .as_array()
                .unwrap()
                .iter()
                .skip(1),
        )
    {
        let finding_id = finding["id"].as_str().unwrap();
        assert_eq!(suggested_action["action"], "view_finding");
        assert_eq!(suggested_action["mutates"], false);
        assert_eq!(suggested_action["requires_confirmation"], false);
        assert_eq!(suggested_action["reason_code"], "top_finding_selected");
        assert_eq!(
            suggested_action["argv"],
            context_action_argv(
                &home,
                &state,
                &["report", "--finding", finding_id, "--json"]
            )
        );
        let detail = json_output(&run_suggested_action(suggested_action));
        assert_eq!(detail["result"]["id"], finding_id);
        assert_eq!(detail["result"]["files_changed"], false);
    }

    let first_output = run(
        &[
            &common[..],
            &[
                "report",
                "--findings",
                "--category",
                "overlap",
                "--severity",
                "medium",
                "--limit",
                "10",
            ],
        ]
        .concat(),
        None,
    );
    assert!(first_output.stdout.len() < 20_000);
    let first = json_output(&first_output);
    assert_eq!(first["result"]["view"], "findings");
    assert_eq!(first["result"]["matched_finding_count"], 30);
    assert_eq!(first["result"]["page"]["returned"], 10);
    assert_eq!(first["result"]["page"]["next_offset"], 10);
    assert_eq!(
        first["result"]["finding_rollups"],
        summary["result"]["finding_rollups"]
    );
    assert_eq!(first["result"]["items"].as_array().unwrap().len(), 10);
    assert!(
        first["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["category"] == "overlap"
                && finding["severity"] == "medium"
                && finding.get("evidence_ids").is_none())
    );
    assert_eq!(
        first["suggested_actions"][0]["argv"],
        context_action_argv(
            &home,
            &state,
            &[
                "report",
                "--findings",
                "--category",
                "overlap",
                "--severity",
                "medium",
                "--limit",
                "10",
                "--offset",
                "10",
                "--json",
            ]
        )
    );

    let second = json_output(&run(
        &[
            &common[..],
            &[
                "report",
                "--findings",
                "--category",
                "overlap",
                "--severity",
                "medium",
                "--limit",
                "10",
                "--offset",
                "10",
            ],
        ]
        .concat(),
        None,
    ));
    assert_ne!(
        first["result"]["items"][0]["id"],
        second["result"]["items"][0]["id"]
    );

    let finding_id = first["result"]["items"][0]["id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert!(!detail["result"]["items"].as_array().unwrap().is_empty());
    assert_eq!(detail["result"]["coverage"]["basis"], "skill_root_scan");
    assert_eq!(detail["result"]["coverage"]["denominator_reliable"], true);
    assert!(
        detail["result"]["coverage"]["missing_root_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    let connection = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let stored_details: String = connection
        .query_row(
            "SELECT details_json FROM findings WHERE id = ?1",
            [finding_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut stored_details: Value = serde_json::from_str(&stored_details).unwrap();
    stored_details["coverage"] = json!({
        "denominator_reliable": false,
        "limited_agents": ["codex", "claude-code"]
    });
    connection
        .execute(
            "UPDATE findings SET details_json = ?1 WHERE id = ?2",
            (serde_json::to_string(&stored_details).unwrap(), finding_id),
        )
        .unwrap();
    drop(connection);
    let migrated_detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(
        migrated_detail["result"]["coverage"]["basis"],
        "skill_root_scan"
    );
    assert_eq!(
        migrated_detail["result"]["coverage"]["denominator_reliable"],
        true
    );

    let usage_findings = json_output(&run(
        &[
            &common[..],
            &[
                "report",
                "--findings",
                "--category",
                "usage",
                "--limit",
                "20",
            ],
        ]
        .concat(),
        None,
    ));
    let usage_finding_id = usage_findings["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Usage coverage is incomplete")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let connection = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let stored_usage_details: String = connection
        .query_row(
            "SELECT details_json FROM findings WHERE id = ?1",
            [usage_finding_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut stored_usage_details: Value = serde_json::from_str(&stored_usage_details).unwrap();
    stored_usage_details["coverage"] = json!({"denominator_reliable": true});
    connection
        .execute(
            "UPDATE findings SET details_json = ?1 WHERE id = ?2",
            (
                serde_json::to_string(&stored_usage_details).unwrap(),
                usage_finding_id,
            ),
        )
        .unwrap();
    drop(connection);
    let usage_detail = json_output(&run(
        &[&common[..], &["report", "--finding", usage_finding_id]].concat(),
        None,
    ));
    assert_eq!(usage_detail["result"]["coverage"]["basis"], "session_usage");
    assert_eq!(
        usage_detail["result"]["coverage"]["denominator_reliable"],
        false
    );
    assert!(
        !usage_detail["result"]["coverage"]["missing_agents"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    for invalid in [
        vec!["report", "--category", "overlap"],
        vec!["report", "--summary", "--findings"],
    ] {
        let output = run(&[&common[..], &invalid].concat(), None);
        assert!(!output.status.success());
        let output: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output["error"]["code"], "invalid_cli_arguments");
    }
}

#[test]
fn legacy_snapshot_requires_typed_rescan_before_find_or_report() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/legacy-identity");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: legacy-identity\ndescription: Legacy identity fixture\n---\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let encoded: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();

    let mut legacy: Value = serde_json::from_str(&encoded).unwrap();
    legacy["content_identity_algorithm"] = json!("sha256-content-v1");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("identity_path_coverage");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("non_unicode_identity_paths_skipped");
    for skill in legacy["skills"].as_array_mut().unwrap() {
        skill
            .as_object_mut()
            .unwrap()
            .remove("content_identity_digest");
    }
    database
        .execute(
            "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
            rusqlite::params![legacy.to_string(), snapshot],
        )
        .unwrap();

    for tail in [
        vec!["find", "legacy identity fixture"],
        vec!["report", "--summary"],
    ] {
        let rejected = run(&[&common[..], &tail].concat(), None);
        assert!(!rejected.status.success());
        let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert_eq!(
            rejected["error"]["code"],
            "content_identity_rescan_required"
        );
        assert_eq!(
            rejected["error"]["details"]["required_algorithm"],
            "sha256-content-unicode-v2"
        );
        assert_eq!(rejected["error"]["details"]["files_changed"], false);
        assert_eq!(
            rejected["suggested_actions"][0]["argv"],
            context_action_argv(&home, &state, &["scan", "--summary", "--json"])
        );
    }

    let rejected_plan = run(&[&common[..], &["plan", "--stdin"]].concat(), Some("{}"));
    assert!(!rejected_plan.status.success());
    let rejected_plan: Value = serde_json::from_slice(&rejected_plan.stdout).unwrap();
    assert_eq!(
        rejected_plan["error"]["code"],
        "content_identity_rescan_required"
    );
}

#[test]
fn gitignore_only_copies_share_routing_identity_but_keep_integrity_drift_checks() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex = home.join(".codex/skills/shared-capability");
    let claude = home.join(".claude/skills/shared-capability");
    for directory in [&codex, &claude] {
        fs::create_dir_all(directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            "---\nname: shared-capability\ndescription: Exact route fixture\n---\nshared body\n",
        )
        .unwrap();
    }
    fs::write(claude.join(".gitignore"), "local-only\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let scanned = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(scanned["result"]["skill_count"], 1);
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    assert!(
        report["result"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| {
                finding["title"] != "Same-name Skills have different content"
                    && finding["title"] != "Declared identity has divergent local content"
            })
    );

    let found = json_output(&run(
        &[
            &common[..],
            &["find", "exact route fixture", "--load", "--limit", "1"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(found["result"]["matches"][0]["variant_count"], 1);
    assert_eq!(
        found["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(found["result"]["loaded_skill"]["content"]["complete"], true);

    let loaded_entrypoint = Path::new(
        found["result"]["loaded_skill"]["content"]["path"]
            .as_str()
            .unwrap(),
    );
    fs::write(
        loaded_entrypoint.parent().unwrap().join(".gitignore"),
        "changed after scan\n",
    )
    .unwrap();
    let drifted = run(
        &[
            &common[..],
            &["find", "exact route fixture", "--load", "--limit", "1"],
        ]
        .concat(),
        None,
    );
    assert!(!drifted.status.success());
    let drifted: Value = serde_json::from_slice(&drifted.stdout).unwrap();
    assert_eq!(
        drifted["error"]["details"]["reason"],
        "package_identity_drift"
    );
    assert_eq!(drifted["error"]["details"]["files_changed"], false);
}

#[test]
fn same_name_divergent_finding_keeps_variant_paths_and_requires_a_choice() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex = home.join(".codex/skills/shared-capability");
    let claude = home.join(".claude/skills/shared-capability");
    fs::create_dir_all(&codex).unwrap();
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        codex.join("SKILL.md"),
        "---\nname: shared-capability\ndescription: First implementation\n---\nalpha\n",
    )
    .unwrap();
    fs::write(
        claude.join("SKILL.md"),
        "---\nname: shared-capability\ndescription: Second implementation\n---\nbeta\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let before_report = json_output(&run(
        &[&common[..], &["find", "first implementation"]].concat(),
        None,
    ));
    assert_eq!(
        before_report["result"]["matches"][0]["variant_finding"]["state"],
        "report_required"
    );
    assert_eq!(
        before_report["result"]["matches"][0]["variant_finding"]["reason_code"],
        "current_snapshot_report_missing"
    );
    assert!(before_report["result"]["matches"][0]["variant_finding"]["finding_id"].is_null());
    assert_eq!(
        before_report["suggested_actions"][0]["argv"],
        context_action_argv(&home, &state, &["report", "--summary", "--json"])
    );
    assert_eq!(before_report["suggested_actions"][0]["mutates"], false);
    assert_eq!(
        before_report["suggested_actions"][0]["requires_confirmation"],
        false
    );

    json_output(&run(&[&common[..], &["report"]].concat(), None));
    let report = json_output(&run(
        &[
            &common[..],
            &["report", "--findings", "--category", "layout"],
        ]
        .concat(),
        None,
    ));
    let finding = report["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["title"] == "Same-name Skills have different content"
                && finding["summary"]
                    .as_str()
                    .unwrap()
                    .contains("shared-capability")
        })
        .unwrap();
    assert_eq!(finding["affected_skill_count"], 2);
    assert_eq!(finding["affected_placement_count"], 2);
    let finding_id = finding["id"].as_str().unwrap();

    let linked_find = json_output(&run(
        &[&common[..], &["find", "first implementation"]].concat(),
        None,
    ));
    let variant_finding = &linked_find["result"]["matches"][0]["variant_finding"];
    assert_eq!(variant_finding["state"], "available");
    assert_eq!(variant_finding["finding_id"], finding_id);
    assert_eq!(variant_finding["report_id"], report["result"]["report_id"]);
    assert_eq!(
        variant_finding["snapshot_id"],
        report["result"]["snapshot_id"]
    );
    assert_eq!(variant_finding["resolution"], "choose_same_name_variant");
    assert_eq!(
        variant_finding["argv"],
        context_action_argv(
            &home,
            &state,
            &["report", "--finding", finding_id, "--json"],
        )
    );
    assert_eq!(
        linked_find["suggested_actions"][0]["argv"],
        variant_finding["argv"]
    );
    assert_eq!(linked_find["suggested_actions"][0]["mutates"], false);
    assert_eq!(
        linked_find["suggested_actions"][0]["requires_confirmation"],
        false
    );

    fs::write(
        codex.join("SKILL.md"),
        "---\nname: shared-capability\ndescription: First implementation\n---\nchanged after report\n",
    )
    .unwrap();
    let drifted_find = json_output(&run(
        &[&common[..], &["find", "first implementation"]].concat(),
        None,
    ));
    assert_eq!(drifted_find["result"]["rescan_required"], true);
    assert_eq!(
        drifted_find["result"]["matches"][0]["variant_finding"]["state"],
        "rescan_required"
    );
    assert_eq!(
        drifted_find["result"]["matches"][0]["variant_finding"]["reason_code"],
        "routable_variant_drift_detected"
    );
    assert!(drifted_find["result"]["matches"][0]["variant_finding"]["finding_id"].is_null());
    assert_eq!(
        drifted_find["suggested_actions"][0]["argv"],
        context_action_argv(&home, &state, &["scan", "--summary", "--json"])
    );

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let after_rescan = json_output(&run(
        &[&common[..], &["find", "first implementation"]].concat(),
        None,
    ));
    assert_eq!(
        after_rescan["result"]["matches"][0]["variant_finding"]["state"],
        "report_required"
    );
    assert!(after_rescan["result"]["matches"][0]["variant_finding"]["finding_id"].is_null());

    let compact = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(
        compact["result"]["resolution"]["decision"],
        "choose_same_name_variant"
    );
    assert_eq!(
        compact["result"]["resolution"]["automatic_change_supported"],
        false
    );
    let variants = compact["result"]["resolution"]["variants"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 2);
    for variant in variants {
        assert!(variant["skill_id"].as_str().unwrap().starts_with("skill_"));
        assert_eq!(variant["content_digests"].as_array().unwrap().len(), 1);
        assert_eq!(variant["paths"].as_array().unwrap().len(), 1);
        assert!(Path::new(variant["paths"][0].as_str().unwrap()).is_file());
        assert_eq!(variant["agents"].as_array().unwrap().len(), 1);
    }
    assert!(
        compact["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );

    let full = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id, "--full"]].concat(),
        None,
    ));
    assert_eq!(full["result"]["placements"].as_array().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn untrusted_same_name_variants_route_to_source_confirmation_without_a_rescan_loop() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let sources = temp.path().join("sources");
    let first_source = sources.join("first");
    let second_source = sources.join("second");
    for (source, body) in [(&first_source, "alpha"), (&second_source, "beta")] {
        fs::create_dir_all(source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            format!(
                "---\nname: shared-external\ndescription: External comparison route\n---\n{body}\n"
            ),
        )
        .unwrap();
    }
    let first_source = fs::canonicalize(first_source).unwrap();
    let second_source = fs::canonicalize(second_source).unwrap();
    let codex_root = home.join(".codex/skills");
    let claude_root = home.join(".claude/skills");
    fs::create_dir_all(&codex_root).unwrap();
    fs::create_dir_all(&claude_root).unwrap();
    let codex_link = codex_root.join("shared-external");
    let claude_link = claude_root.join("shared-external");
    std::os::unix::fs::symlink(&first_source, &codex_link).unwrap();
    std::os::unix::fs::symlink(&second_source, &claude_link).unwrap();

    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["status"]].concat(), None));
    let retained = [
        (&codex_link, "content:retained-strong-alpha"),
        (&claude_link, "content:retained-strong-beta"),
    ]
    .map(|(link, identity_key)| {
        let entrypoint = link.join("SKILL.md");
        let skill_id = format!(
            "skill_{:x}",
            Sha256::digest(format!("unreadable-link:{}", entrypoint.display()).as_bytes())
        );
        let connection = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
        connection
            .execute(
                "INSERT INTO skills
                    (id, identity_key, name, description, declared_source, declared_revision,
                     content_digest, digest_version, governance_state, canonical_path)
                 VALUES (?1, ?2, 'shared-external', 'External comparison route', NULL, NULL,
                         ?3, 1, 'managed', ?4)",
                rusqlite::params![
                    skill_id,
                    identity_key,
                    format!("retained-package-{skill_id}"),
                    link.to_string_lossy(),
                ],
            )
            .unwrap();
        (skill_id, identity_key)
    });

    let untrusted_scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(untrusted_scan["result"]["skill_count"], 2);
    let mut report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let original_report_id = report["result"]["report_id"].as_str().unwrap().to_owned();
    let original_finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let connection = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let summary: String = connection
        .query_row(
            "SELECT summary_json FROM reports WHERE id = ?1",
            [&original_report_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut legacy_summary: Value = serde_json::from_str(&summary).unwrap();
    let legacy_finding = legacy_summary["findings"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()
        .as_object_mut()
        .unwrap();
    legacy_finding.remove("kind");
    let details: String = connection
        .query_row(
            "SELECT details_json FROM findings WHERE id = ?1",
            [&original_finding_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut legacy_details: Value = serde_json::from_str(&details).unwrap();
    legacy_details.as_object_mut().unwrap().remove("kind");
    connection
        .execute(
            "UPDATE reports SET summary_json = ?1 WHERE id = ?2",
            rusqlite::params![legacy_summary.to_string(), original_report_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE findings SET details_json = ?1 WHERE id = ?2",
            rusqlite::params![legacy_details.to_string(), original_finding_id],
        )
        .unwrap();
    drop(connection);

    let legacy_detail = json_output(&run(
        &[&common[..], &["report", "--finding", &original_finding_id]].concat(),
        None,
    ));
    assert_eq!(
        legacy_detail["result"]["kind"],
        "escaping_link_source_confirmation"
    );
    assert_eq!(
        legacy_detail["result"]["resolution"]["decision"],
        "confirm_trusted_source_roots"
    );

    report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    assert_ne!(report["result"]["report_id"], original_report_id);
    let escaping_finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap();
    assert_eq!(
        escaping_finding["kind"],
        "escaping_link_source_confirmation"
    );
    let finding_id = escaping_finding["id"].as_str().unwrap();

    let found = json_output(&run(
        &[&common[..], &["find", "shared-external"]].concat(),
        None,
    ));
    let reference = &found["result"]["matches"][0]["variant_finding"];
    assert_eq!(found["result"]["rescan_required"], false);
    assert_eq!(reference["state"], "source_confirmation_required");
    assert_eq!(
        reference["reason_code"],
        "untrusted_variants_require_source_confirmation"
    );
    assert_eq!(reference["finding_id"], finding_id);
    assert_eq!(reference["report_id"], report["result"]["report_id"]);
    assert_eq!(reference["resolution"], "confirm_trusted_source_roots");
    assert_eq!(
        reference["argv"],
        context_action_argv(
            &home,
            &state,
            &["report", "--finding", finding_id, "--json"],
        )
    );
    assert!(
        found["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| {
                action["action"] != "load_exact_variant_for_comparison"
                    && action["action"] != "plan"
                    && action["action"] != "scan"
                    && action["mutates"] == false
                    && action["requires_confirmation"] == false
            })
    );
    let blocked_load = run(
        &[
            &common[..],
            &["find", "shared-external", "--load", "--limit", "1"],
        ]
        .concat(),
        None,
    );
    assert!(!blocked_load.status.success());
    let blocked_load: Value = serde_json::from_slice(&blocked_load.stdout).unwrap();
    assert_eq!(
        blocked_load["error"]["details"]["reason"],
        "same_name_variants_ambiguous"
    );
    assert!(blocked_load["error"]["details"].get("content").is_none());

    let repeated_scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let repeated_report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    assert_eq!(
        repeated_report["result"]["snapshot_id"],
        repeated_scan["result"]["snapshot_id"]
    );
    let repeated_finding_id = repeated_report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let repeated = json_output(&run(
        &[&common[..], &["find", "shared-external"]].concat(),
        None,
    ));
    assert_eq!(repeated["result"]["rescan_required"], false);
    assert_eq!(
        repeated["result"]["matches"][0]["variant_finding"]["state"],
        "source_confirmation_required"
    );
    assert_eq!(
        repeated["result"]["matches"][0]["variant_finding"]["finding_id"],
        repeated_finding_id
    );
    let connection = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    for (skill_id, identity_key) in &retained {
        let stored: (String, String) = connection
            .query_row(
                "SELECT identity_key, governance_state FROM skills WHERE id = ?1",
                [skill_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(&stored.0, identity_key);
        assert_eq!(stored.1, "managed");
    }
    drop(connection);

    let replacement = temp.path().join("replacement");
    fs::create_dir(&replacement).unwrap();
    fs::write(
        replacement.join("SKILL.md"),
        "---\nname: shared-external\ndescription: External comparison route\n---\nreplacement\n",
    )
    .unwrap();
    fs::remove_file(&codex_link).unwrap();
    std::os::unix::fs::symlink(&replacement, &codex_link).unwrap();
    let drifted = json_output(&run(
        &[&common[..], &["find", "shared-external"]].concat(),
        None,
    ));
    assert_eq!(drifted["result"]["rescan_required"], true);
    assert_eq!(
        drifted["result"]["matches"][0]["variant_finding"]["state"],
        "rescan_required"
    );
    fs::remove_file(&codex_link).unwrap();
    std::os::unix::fs::symlink(&first_source, &codex_link).unwrap();
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let recovered_report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let recovered_finding_id = recovered_report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let confirm_source = |finding_id: &str, source: &Path| {
        json_output(&run(
            &[
                &common[..],
                &[
                    "source-root",
                    "confirm",
                    "--finding",
                    finding_id,
                    "--path",
                    source.to_str().unwrap(),
                ],
            ]
            .concat(),
            None,
        ))
    };
    let first_confirmation = confirm_source(recovered_finding_id, &first_source);
    assert_eq!(
        first_confirmation["result"]["permission_scope"],
        "exact_local_read_only"
    );
    assert_eq!(first_confirmation["result"]["content_endorsed"], false);
    assert_eq!(first_confirmation["result"]["plan_apply_authorized"], false);
    assert_eq!(first_confirmation["result"]["files_changed"], false);

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let mixed_report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let mixed = json_output(&run(
        &[&common[..], &["find", "shared-external"]].concat(),
        None,
    ));
    assert_eq!(mixed["result"]["rescan_required"], false);
    assert_ne!(
        mixed["result"]["matches"][0]["variant_finding"]["state"],
        "source_confirmation_required"
    );
    let load_actions = mixed["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["action"] == "load_exact_variant_for_comparison")
        .collect::<Vec<_>>();
    assert_eq!(load_actions.len(), 1);
    let readable_variant_id = mixed["result"]["matches"][0]["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variant| !variant["paths"].as_array().unwrap().is_empty())
        .unwrap()["skill_id"]
        .as_str()
        .unwrap();
    assert!(
        load_actions[0]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|argument| argument == readable_variant_id)
    );
    let mixed_finding_id = mixed_report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let second_confirmation = confirm_source(mixed_finding_id, &second_source);
    assert_eq!(
        second_confirmation["result"]["permission_scope"],
        "exact_local_read_only"
    );
    assert_eq!(second_confirmation["result"]["content_endorsed"], false);
    assert_eq!(
        second_confirmation["result"]["plan_apply_authorized"],
        false
    );
    assert_eq!(second_confirmation["result"]["files_changed"], false);

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    json_output(&run(&[&common[..], &["report"]].concat(), None));
    let readable = json_output(&run(
        &[&common[..], &["find", "external comparison route"]].concat(),
        None,
    ));
    assert_eq!(readable["result"]["matches"][0]["variant_count"], 2);
    assert_eq!(
        readable["result"]["matches"][0]["variant_finding"]["state"],
        "available"
    );
    let connection = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    for (skill_id, identity_key) in &retained {
        let stored: (String, String) = connection
            .query_row(
                "SELECT identity_key, governance_state FROM skills WHERE id = ?1",
                [skill_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(&stored.0, identity_key);
        assert_eq!(stored.1, "managed");
    }
}

#[test]
fn variant_finding_rechecks_drift_beyond_the_displayed_variant_limit() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    for index in 0..11 {
        let skill = root.join(format!("variant-{index:02}"));
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!(
                "---\nname: shared-many\ndescription: Many variant route\n---\nvariant {index}\n"
            ),
        )
        .unwrap();
    }
    for index in 0..2 {
        let skill = root.join(format!("primary-{index:02}"));
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!(
                "---\nname: primary-clean\ndescription: Primary clean exact route\n---\nprimary {index}\n"
            ),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot_id = scan["result"]["snapshot_id"].as_str().unwrap();
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["kind"] == "same_name_divergent_content"
                && finding["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("shared-many"))
        })
        .unwrap();
    let found = json_output(&run(
        &[&common[..], &["find", "many variant route"]].concat(),
        None,
    ));
    assert_eq!(found["result"]["matches"][0]["variant_count"], 11);
    assert_eq!(
        found["result"]["matches"][0]["variants"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let displayed_ids = found["result"]["matches"][0]["variant_skill_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let finding_detail = json_output(&run(
        &[
            &common[..],
            &[
                "report",
                "--finding",
                finding["id"].as_str().unwrap(),
                "--full",
            ],
        ]
        .concat(),
        None,
    ));
    let omitted_id = finding_detail["result"]["affected_skill_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|skill_id| !displayed_ids.contains(skill_id))
        .unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot_id],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let omitted_entrypoint = payload["placements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|placement| placement["skill_id"] == omitted_id)
        .and_then(|placement| placement["entrypoint"].as_str())
        .unwrap();
    fs::write(
        omitted_entrypoint,
        "---\nname: shared-many\ndescription: Many variant route\n---\ndrifted\n",
    )
    .unwrap();

    let drifted = json_output(&run(
        &[&common[..], &["find", "many variant route"]].concat(),
        None,
    ));
    assert_eq!(drifted["result"]["rescan_required"], true);
    assert_eq!(
        drifted["result"]["matches"][0]["variant_finding"]["state"],
        "rescan_required"
    );
    assert!(drifted["result"]["matches"][0]["variant_finding"]["finding_id"].is_null());
    assert!(
        drifted["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "load_exact_variant_for_comparison")
    );

    let clean_top = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "primary clean exact route many variant",
                "--limit",
                "2",
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(clean_top["result"]["matches"][0]["name"], "primary-clean");
    assert_eq!(clean_top["result"]["matches"][0]["variant_count"], 2);
    assert_eq!(clean_top["result"]["rescan_required"], true);
    assert_eq!(
        clean_top["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| action["action"] == "load_exact_variant_for_comparison")
            .count(),
        2
    );
}

#[test]
fn raw_roster_plan_requires_evidence_for_the_changed_skill() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let target = home.join(".codex/skills/target-skill");
    let unrelated_codex = home.join(".codex/skills/unrelated-skill");
    let unrelated_claude = home.join(".claude/skills/unrelated-skill");
    for directory in [&target, &unrelated_codex, &unrelated_claude] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::write(
        target.join("SKILL.md"),
        "---\nname: target-skill\ndescription: Target capability\n---\n",
    )
    .unwrap();
    let unrelated_content = "---\nname: unrelated-skill\ndescription: Unrelated capability\n---\n";
    fs::write(unrelated_codex.join("SKILL.md"), unrelated_content).unwrap();
    fs::write(unrelated_claude.join("SKILL.md"), unrelated_content).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot_id = scan["result"]["snapshot_id"].as_str().unwrap();
    let findings = json_output(&run(
        &[&common[..], &["report", "--findings", "--limit", "20"]].concat(),
        None,
    ));
    let unrelated_evidence_id = findings["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["kind"] == "exact_duplicate_placements")
        .unwrap()["primary_evidence_id"]
        .as_str()
        .unwrap();
    let find = json_output(&run(
        &[&common[..], &["find", "target capability", "--limit", "1"]].concat(),
        None,
    ));
    let target_skill_id = find["result"]["matches"][0]["skill_id"].as_str().unwrap();
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot_id,
        "evidence_ids": [unrelated_evidence_id],
        "roster_changes": [{
            "agent": "codex",
            "skill_id": target_skill_id,
            "state": "on_demand"
        }]
    });

    let output = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );

    assert!(!output.status.success());
    let rejected: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rejected["error"]["code"], "plan_evidence_scope_mismatch");
    assert_eq!(
        rejected["error"]["details"]["reason"],
        "no_cited_evidence_covers_roster_change"
    );
    assert_eq!(rejected["error"]["details"]["files_changed"], false);
    assert_eq!(
        rejected["error"]["details"]["unsupported_changes"],
        json!([{"agent": "codex", "skill_id": target_skill_id}])
    );
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 0);
    assert!(target.join("SKILL.md").is_file());

    let target_claude = home.join(".claude/skills/target-skill");
    fs::create_dir_all(&target_claude).unwrap();
    fs::copy(target.join("SKILL.md"), target_claude.join("SKILL.md")).unwrap();
    let rescan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let current_snapshot_id = rescan["result"]["snapshot_id"].as_str().unwrap();
    let current_findings = json_output(&run(
        &[&common[..], &["report", "--findings", "--limit", "20"]].concat(),
        None,
    ));
    let mut target_placement_evidence_id = None;
    for finding in current_findings["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["kind"] == "exact_duplicate_placements")
    {
        let finding_id = finding["id"].as_str().unwrap();
        let detail = json_output(&run(
            &[&common[..], &["report", "--finding", finding_id, "--full"]].concat(),
            None,
        ));
        if !detail["result"]["affected_skill_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|skill_id| skill_id.as_str() == Some(target_skill_id))
        {
            continue;
        }
        target_placement_evidence_id = detail["result"]["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .find(|evidence| evidence["subject_type"] == "placement")
            .and_then(|evidence| evidence["id"].as_str())
            .map(str::to_owned);
        break;
    }
    let target_placement_evidence_id = target_placement_evidence_id
        .expect("the target duplicate Finding must expose Placement Evidence");
    let relevant_request = json!({
        "schema_version": 1,
        "scan_id": current_snapshot_id,
        "evidence_ids": [target_placement_evidence_id],
        "roster_changes": [{
            "agent": "codex",
            "skill_id": target_skill_id,
            "state": "on_demand"
        }]
    });
    let accepted = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&relevant_request.to_string()),
    ));
    assert_eq!(accepted["ok"], true);
    assert_eq!(
        accepted["result"]["evidence"]["ids"],
        json!([target_placement_evidence_id])
    );
}

#[test]
fn public_cli_scans_reports_plans_applies_and_undoes() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_root = home.join(".codex/skills");
    let skill = skill_root.join("example");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: example\ndescription: Example task helper\ntriggers: [example]\n---\n",
    )
    .unwrap();
    let second_skill = home.join(".claude/skills/example");
    fs::create_dir_all(&second_skill).unwrap();
    fs::write(
        second_skill.join("SKILL.md"),
        "---\nname: example\ndescription: Example task helper\ntriggers: [example]\n---\n",
    )
    .unwrap();
    let session_root = home.join(".codex/sessions");
    fs::create_dir_all(&session_root).unwrap();
    fs::write(
        session_root.join("session.jsonl"),
        "{\"type\":\"invoke_skill\",\"invoked_skill\":\"example\"}\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(scan["ok"], true);
    assert_eq!(scan["result"]["skill_count"], 1);
    assert_eq!(scan["result"]["files_changed"], false);
    assert_eq!(
        scan["suggested_actions"][0]["argv"],
        context_action_argv(&home, &state, &["report", "--json"])
    );
    let codex_coverage = scan["result"]["coverage"]
        .as_array()
        .unwrap()
        .iter()
        .find(|coverage| coverage["agent"] == "codex")
        .unwrap();
    assert_eq!(codex_coverage["files_discovered"], 1);
    assert_eq!(codex_coverage["files_observed"], 1);
    assert_eq!(codex_coverage["files_partially_observed"], 0);
    assert_eq!(codex_coverage["denominator_reliable"], true);
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    for (table, minimum) in [
        ("roots", 16_i64),
        ("placements", 1),
        ("evidence", 1),
        ("usage_events", 1),
    ] {
        let count: i64 = database
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(count >= minimum, "expected normalized rows in {table}");
    }

    let initial_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(initial_status["result"]["retention"]["raw_usage_days"], 180);
    assert_eq!(initial_status["result"]["pending_plan_count"], 0);

    let report_output = run(&[&common[..], &["report"]].concat(), None);
    let report_output_len = report_output.stdout.len();
    let report = json_output(&report_output);
    assert_eq!(report["ok"], true);
    assert!(report["result"]["findings"].is_array());
    assert!(report["result"]["primary_metrics"].is_object());
    assert_eq!(report["result"]["observed_use_agent_count"], 1);
    assert_eq!(report["result"]["coverage_root_agent_count"], 1);
    assert_eq!(report["result"]["coverage_sampled_agent_count"], 1);
    assert_eq!(report["result"]["coverage_reliable_agent_count"], 1);
    assert_eq!(report["result"]["coverage_limited_agent_count"], 0);
    assert_eq!(report["result"]["coverage_missing_agent_count"], 7);
    assert_eq!(report["result"]["coverage_inaccessible_agent_count"], 0);
    assert_eq!(
        report["result"]["session_coverage"],
        json!({
            "supported_agents": 8,
            "roots_present_agents": 1,
            "sampled_agents": 1,
            "complete_agents": 1,
            "limited_agents": 0,
            "missing_root_agents": 7,
            "inaccessible_agents": 0
        })
    );
    let repeated_report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    assert_eq!(
        repeated_report["result"]["report_id"],
        report["result"]["report_id"]
    );
    let summary_report_output = run(&[&common[..], &["report", "--summary"]].concat(), None);
    let summary_report = json_output(&summary_report_output);
    assert_eq!(
        summary_report["result"]["finding_count"],
        report["result"]["finding_count"]
    );
    assert!(
        summary_report["result"]["findings"]
            .as_array()
            .unwrap()
            .len()
            <= 3
    );
    assert_eq!(
        summary_report["result"]["findings"],
        report["result"]["findings"]
    );
    let full_report_output = run(&[&common[..], &["report", "--full"]].concat(), None);
    let full_report = json_output(&full_report_output);
    assert_eq!(
        full_report["result"]["findings"].as_array().unwrap().len(),
        report["result"]["finding_count"].as_u64().unwrap() as usize
    );
    assert!(full_report_output.stdout.len() > report_output_len);
    assert_eq!(
        summary_report["result"]["session_coverage"],
        report["result"]["session_coverage"]
    );
    for finding in summary_report["result"]["findings"].as_array().unwrap() {
        assert!(finding.get("affected_skill_ids").is_none());
        assert!(finding.get("affected_placement_ids").is_none());
        assert!(finding.get("evidence_ids").is_none());
        assert!(finding.get("primary_evidence_id").is_some());
        assert!(finding["affected_skill_count"].is_number());
        assert!(finding["affected_placement_count"].is_number());
    }
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["affected_placement_count"]
                .as_u64()
                .unwrap_or_default()
                > 0
        })
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let finding = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(finding["result"]["id"], finding_id);
    assert!(finding["result"].get("affected_skill_ids").is_none());
    assert!(finding["result"].get("affected_placement_ids").is_none());
    assert!(finding["result"]["impact"].is_object());
    assert!(finding["result"]["coverage"].is_object());
    assert!(
        finding["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["path"].as_str() == Some(skill.join("SKILL.md").to_str().unwrap()))
    );
    assert!(!finding["result"]["items"].as_array().unwrap().is_empty());
    for item in finding["result"]["items"].as_array().unwrap() {
        let exists: i64 = database
            .query_row(
                "SELECT COUNT(*) FROM evidence WHERE id = ?1",
                [item["evidence_id"].as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "Finding Evidence must be traceable");
    }

    let find = json_output(&run(
        &[&common[..], &["find", "example task"]].concat(),
        None,
    ));
    assert_eq!(find["result"]["matches"][0]["name"], "example");
    let skill_id = find["result"]["matches"][0]["skill_id"].as_str().unwrap();
    let evidence_id = skill_evidence_id(&database, snapshot, skill_id);

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let setup_plan = setup["result"]["plan_id"].as_str().unwrap();
    assert!(setup["result"]["operation_count"].as_u64().unwrap() > 0);
    assert!(setup["result"].get("operations").is_none());
    assert!(!setup["result"].to_string().contains("## Safety boundaries"));
    let setup_applied = json_output(&run(&[&common[..], &["apply", setup_plan]].concat(), None));
    assert!(skill_root.join("skillroster/SKILL.md").is_file());
    let setup_receipt = setup_applied["result"]["receipt_id"].as_str().unwrap();
    let setup_undone = json_output(&run(
        &[&common[..], &["undo", setup_receipt]].concat(),
        None,
    ));
    assert_eq!(setup_undone["result"]["verification"], "passed");
    assert!(!skill_root.join("skillroster").exists());

    let plan_input = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id.clone()],
        "roster_changes": [{
            "agent": "codex",
            "skill_id": skill_id,
            "state": "on_demand"
        }]
    })
    .to_string();
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&plan_input),
    ));
    assert_eq!(plan["result"]["files_changed"], false);
    assert_eq!(plan["result"]["evidence"]["ids"][0], evidence_id);
    assert_eq!(plan["result"]["change_summary"]["roster_change_count"], 1);
    assert_eq!(
        plan["result"]["impact"]["roster"]["after_state_counts"]["on_demand"],
        1
    );
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();

    let agent_id: String = database
        .query_row("SELECT id FROM agents WHERE kind = 'codex'", [], |row| {
            row.get(0)
        })
        .unwrap();
    database
        .execute(
            "INSERT INTO roster_entries (agent_id, skill_id, state, updated_at)
             VALUES (?1, ?2, 'core', 0)",
            rusqlite::params![agent_id, skill_id],
        )
        .unwrap();
    let drifted = run(&[&common[..], &["apply", plan_id]].concat(), None);
    assert!(!drifted.status.success(), "Apply must reject Roster drift");
    database.execute("DELETE FROM roster_entries", []).unwrap();

    let pending_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(pending_status["result"]["pending_plan_count"], 1);
    assert_eq!(
        pending_status["suggested_actions"][0]["argv"],
        context_action_argv(&home, &state, &["plan", "--show", plan_id, "--json"])
    );
    assert_eq!(
        pending_status["suggested_actions"][0]["reason_code"],
        "pending_plan_requires_review"
    );
    assert_eq!(pending_status["suggested_actions"][0]["mutates"], false);

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(applied["result"]["verification"], "passed");
    let roster_state: String = database
        .query_row("SELECT state FROM roster_entries", [], |row| row.get(0))
        .unwrap();
    assert_eq!(roster_state, "on_demand");
    let receipt = applied["result"]["receipt_id"].as_str().unwrap();
    let receipt_verification: String = database
        .query_row(
            "SELECT verification_json FROM receipts WHERE id = ?1",
            [receipt],
            |row| row.get(0),
        )
        .unwrap();
    let receipt_verification: Value = serde_json::from_str(&receipt_verification).unwrap();
    assert_eq!(receipt_verification["evidence_ids"][0], evidence_id);
    let applied_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(
        applied_status["result"]["last_receipt"]["receipt_id"],
        receipt
    );

    let export_path = temp.path().join("lifecycle.json");
    let exported = json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                export_path.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(exported["result"]["operation"], "export");
    let export: Value = serde_json::from_slice(&fs::read(&export_path).unwrap()).unwrap();
    assert_eq!(export["retention"]["raw_usage_days"], 180);
    let overwrite = run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                export_path.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    );
    assert!(!overwrite.status.success());

    let purged = json_output(&run(
        &[&common[..], &["lifecycle", "purge", "--raw-days", "180"]].concat(),
        None,
    ));
    assert_eq!(purged["result"]["plans_or_receipts_changed"], false);
    assert_eq!(purged["result"]["agent_files_changed"], false);

    let shared_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("write.lock"))
        .unwrap();
    FileExt::try_lock_shared(&shared_lock).unwrap();
    let concurrent_status = run(&[&common[..], &["status"]].concat(), None);
    assert!(concurrent_status.status.success());
    let concurrent_plan_detail = run(&[&common[..], &["plan", "--show", plan_id]].concat(), None);
    assert!(concurrent_plan_detail.status.success());
    for (args, input) in [
        (&["scan"][..], None),
        (&["report"][..], None),
        (&["plan"][..], Some("{}")),
        (&["lifecycle", "exclude", "codex"][..], None),
        (&["lifecycle", "purge", "--raw-days", "180"][..], None),
        (&["lifecycle", "recovery"][..], None),
    ] {
        let blocked = run(&[&common[..], args].concat(), input);
        assert!(
            !blocked.status.success(),
            "command was not locked: {args:?}"
        );
        let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
        assert_eq!(blocked["error"]["code"], "write_locked");
        assert_eq!(blocked["error"]["retryable"], true);
    }
    FileExt::unlock(&shared_lock).unwrap();
    drop(shared_lock);

    let clean_recovery = json_output(&run(
        &[&common[..], &["lifecycle", "recovery"]].concat(),
        None,
    ));
    assert_eq!(clean_recovery["result"]["recovery_state"], "clear");
    assert_eq!(clean_recovery["result"]["imported_receipt_ids"], json!([]));
    assert_eq!(clean_recovery["result"]["import_errors"], json!([]));
    assert_eq!(clean_recovery["result"]["state_changed"], false);

    let write_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("write.lock"))
        .unwrap();
    write_lock.try_lock_exclusive().unwrap();
    let locked_purge = run(
        &[&common[..], &["lifecycle", "purge", "--raw-days", "180"]].concat(),
        None,
    );
    assert!(!locked_purge.status.success());
    let locked_purge: Value = serde_json::from_slice(&locked_purge.stdout).unwrap();
    assert_eq!(locked_purge["error"]["code"], "write_locked");
    let locked_status = run(&[&common[..], &["status"]].concat(), None);
    assert!(!locked_status.status.success());
    let locked_status: Value = serde_json::from_slice(&locked_status.stdout).unwrap();
    assert_eq!(locked_status["error"]["code"], "write_locked");
    let locked_delete = run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    );
    assert!(!locked_delete.status.success());
    let locked_delete: Value = serde_json::from_slice(&locked_delete.stdout).unwrap();
    assert_eq!(locked_delete["error"]["code"], "write_locked");
    FileExt::unlock(&write_lock).unwrap();
    drop(write_lock);

    let undone = json_output(&run(&[&common[..], &["undo", receipt]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    let original_status: String = database
        .query_row(
            "SELECT status FROM receipts WHERE id = ?1",
            [receipt],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original_status, "undone");
    let roster_count: i64 = database
        .query_row("SELECT COUNT(*) FROM roster_entries", [], |row| row.get(0))
        .unwrap();
    assert_eq!(roster_count, 0);

    let repeated = run(&[&common[..], &["undo", receipt]].concat(), None);
    assert!(!repeated.status.success());
    let repeated: Value = serde_json::from_slice(&repeated.stdout).unwrap();
    assert_eq!(repeated["ok"], false);
    let recovery_count: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM receipts WHERE status = 'recovery_required'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recovery_count, 0);

    let unconfirmed_history_purge = run(
        &[&common[..], &["lifecycle", "purge", "--plans-receipts"]].concat(),
        None,
    );
    assert!(!unconfirmed_history_purge.status.success());
    let history = json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "purge",
                "--plans-receipts",
                "--confirm",
                "PURGE-PLANS-RECEIPTS",
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(history["result"]["plans_or_receipts_changed"], true);
    assert_eq!(history["result"]["files_changed"], true);
    assert_eq!(history["result"]["agent_files_changed"], false);
    assert_eq!(history["result"]["library_files_changed"], false);
    assert!(
        history["result"]["plan_receipt_result"]["recovery_directories"]
            .as_u64()
            .unwrap()
            > 0,
        "purging Receipt history must also remove its recovery content"
    );
    for table in ["plans", "plan_operations", "receipts", "receipt_operations"] {
        let count: i64 = database
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "explicit history purge must clear {table}");
    }
    assert!(skill.join("SKILL.md").is_file());
}

#[test]
fn source_confirmation_details_follow_public_lifecycle() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let details = state.join("source-confirmation");
    fs::create_dir(&details).unwrap();
    let artifact = details.join(format!("{}.json", ulid::Ulid::new()));
    let reviewed = temp.path().join("reviewed");
    let artifact_payload = json!({
        "schema_version": 1,
        "reason": "trusted_canonical_sources_required",
        "decision": "confirm_trusted_source_roots",
        "requested_core_budget": 10,
        "blocked_change_count": 1,
        "blocked_changes": [{
            "agent": "codex",
            "skill_id": "skill_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "name": "fixture",
            "reason": "no_owned_exact_content_to_preserve",
            "state": "unchanged",
            "observed_source_target": reviewed
        }],
        "skill_ids": ["skill_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        "source_root_count": 1,
        "source_roots": [reviewed],
        "after_confirmation": {
            "repeatable_option": "--source-root",
            "source_roots": [reviewed],
            "argv": ["skillroster", "--source-root", reviewed, "scan", "--json"]
        }
    });
    fs::write(&artifact, serde_json::to_vec(&artifact_payload).unwrap()).unwrap();

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(
        status["result"]["retention"]["source_confirmation_details"]["count"],
        1
    );
    let inspect = json_output(&run(
        &[&common[..], &["lifecycle", "inspect"]].concat(),
        None,
    ));
    assert_eq!(
        inspect["result"]["counts"]["source_confirmation_details"],
        1
    );
    assert_eq!(
        inspect["result"]["source_confirmation_details"]["retention"],
        "until_explicit_purge_or_delete"
    );

    let export_path = temp.path().join("lifecycle-details.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                export_path.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let exported: Value = serde_json::from_slice(&fs::read(export_path).unwrap()).unwrap();
    assert_eq!(
        exported["source_confirmation_details"][0]["schema_version"],
        1
    );

    let purged = json_output(&run(
        &[
            &common[..],
            &["lifecycle", "purge", "--source-confirmation"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(purged["result"]["removed_source_confirmation_details"], 1);
    assert!(!details.exists());

    fs::create_dir(&details).unwrap();
    let user_json = details.join(format!("{}.json", ulid::Ulid::new()));
    let mut lookalike_payload = artifact_payload.clone();
    lookalike_payload["after_confirmation"]["argv"] = json!(["skillroster", "lifecycle", "delete"]);
    fs::write(&user_json, serde_json::to_vec(&lookalike_payload).unwrap()).unwrap();
    let refused = run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    );
    assert!(!refused.status.success());
    assert!(user_json.is_file());
    assert!(state.join("skillroster.db").is_file());
    fs::remove_file(&user_json).unwrap();
    let non_minimal_json = details.join(format!("{}.json", ulid::Ulid::new()));
    let mut non_minimal_payload = artifact_payload.clone();
    let child = reviewed.join("child");
    non_minimal_payload["source_root_count"] = json!(2);
    non_minimal_payload["source_roots"] = json!([child, reviewed]);
    non_minimal_payload["after_confirmation"]["source_roots"] = json!([child, reviewed]);
    non_minimal_payload["after_confirmation"]["argv"] = json!([
        "skillroster",
        "--source-root",
        child,
        "--source-root",
        reviewed,
        "scan",
        "--json"
    ]);
    fs::write(
        &non_minimal_json,
        serde_json::to_vec(&non_minimal_payload).unwrap(),
    )
    .unwrap();
    let refused = run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    );
    assert!(!refused.status.success());
    assert!(non_minimal_json.is_file());
    assert!(state.join("skillroster.db").is_file());
    fs::remove_file(&non_minimal_json).unwrap();
    fs::write(&artifact, serde_json::to_vec(&artifact_payload).unwrap()).unwrap();
    let deleted = json_output(&run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(deleted["result"]["removed_source_confirmation_details"], 1);
    assert!(!details.exists());
}

#[test]
fn setup_requires_a_choice_before_replacing_a_modified_bootstrap_skill() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let ordinary = root.join("ordinary");
    let bootstrap = root.join("skillroster/SKILL.md");
    fs::create_dir_all(&ordinary).unwrap();
    fs::write(
        ordinary.join("SKILL.md"),
        "---\nname: ordinary\ndescription: fixture\n---\n",
    )
    .unwrap();
    fs::create_dir_all(bootstrap.parent().unwrap()).unwrap();
    let modified =
        "---\nname: skillroster\ndescription: locally customized\n---\ncustom instructions\n";
    fs::write(&bootstrap, modified).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let blocked = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(blocked["result"]["state"], "modified_choice_required");
    assert_setup_versions(&blocked);
    assert_eq!(blocked["result"]["modified_count"], 1);
    assert!(blocked["result"]["plan_id"].is_null());
    assert_eq!(blocked["result"]["targets"][0]["status"], "modified");
    assert!(blocked["result"]["targets"][0]["installed_version"].is_null());
    assert_eq!(blocked["suggested_actions"].as_array().unwrap().len(), 2);
    assert_eq!(
        blocked["suggested_actions"][0]["argv"],
        context_action_argv(
            &home,
            &state,
            &["setup", "--modified-choice", "retain-local", "--json",]
        )
    );
    assert_eq!(blocked["suggested_actions"][0]["mutates"], false);
    assert_eq!(
        blocked["suggested_actions"][0]["requires_confirmation"],
        false
    );
    assert_eq!(
        blocked["suggested_actions"][1]["argv"],
        context_action_argv(
            &home,
            &state,
            &["setup", "--modified-choice", "adopt-current", "--json",]
        )
    );
    assert_eq!(fs::read_to_string(&bootstrap).unwrap(), modified);

    let retained = json_output(&run(
        &[&common[..], &["setup", "--modified-choice", "retain-local"]].concat(),
        None,
    ));
    assert_eq!(retained["result"]["state"], "local_modifications_retained");
    assert!(retained["result"]["plan_id"].is_null());
    assert_eq!(fs::read_to_string(&bootstrap).unwrap(), modified);

    let upgrade = json_output(&run(
        &[
            &common[..],
            &["setup", "--modified-choice", "adopt-current"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(upgrade["result"]["state"], "preview_ready");
    assert_setup_versions(&upgrade);
    assert_eq!(upgrade["result"]["replace_count"], 1);
    assert_eq!(upgrade["result"]["modified_count"], 1);
    assert_eq!(upgrade["suggested_actions"].as_array().unwrap().len(), 1);
    assert_eq!(upgrade["suggested_actions"][0]["action"], "apply");
    assert_eq!(upgrade["suggested_actions"][0]["mutates"], true);
    assert_eq!(
        upgrade["suggested_actions"][0]["requires_confirmation"],
        true
    );
    assert_eq!(fs::read_to_string(&bootstrap).unwrap(), modified);
    let plan_id = upgrade["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    assert!(
        detail["result"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| operation["kind"] == "replace_file")
    );

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(
        fs::read_to_string(&bootstrap).unwrap(),
        include_str!("../skill/skillroster/SKILL.md").replace("\r\n", "\n")
    );
    for reference in ["routing.md", "governance.md", "mutation.md"] {
        assert!(
            bootstrap
                .parent()
                .unwrap()
                .join("references")
                .join(reference)
                .is_file()
        );
    }
    let routing =
        fs::read_to_string(bootstrap.parent().unwrap().join("references/routing.md")).unwrap();
    assert!(routing.contains("inspect_same_name_variants"));
    assert!(routing.contains("find_snapshot_changed"));
    assert!(routing.contains("rerun_find_on_latest_snapshot"));
    assert!(routing.contains("Do not reconstruct or rerun Find"));
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let current = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(current["result"]["state"], "up_to_date");
    assert!(current["result"]["plan_id"].is_null());
    assert_setup_versions(&current);
    assert_eq!(
        current["result"]["targets"][0]["installed_version"],
        "1.8.29"
    );

    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(fs::read_to_string(&bootstrap).unwrap(), modified);
}

#[test]
fn setup_upgrades_an_exact_official_legacy_bootstrap_and_undo_restores_it() {
    for (legacy_version, legacy) in [
        ("1.4.0", include_str!("fixtures/bootstrap-v1.4.0.md")),
        ("1.5.0", include_str!("fixtures/bootstrap-v1.5.0.md")),
        ("1.5.1", include_str!("fixtures/bootstrap-v1.5.1.md")),
        ("1.6.0", include_str!("fixtures/bootstrap-v1.6.0.md")),
        ("1.7.0", include_str!("fixtures/bootstrap-v1.7.0.md")),
        ("1.7.1", include_str!("fixtures/bootstrap-v1.7.1.md")),
        ("1.8.0", include_str!("fixtures/bootstrap-v1.8.0.md")),
        ("1.8.1", include_str!("fixtures/bootstrap-v1.8.1.md")),
        ("1.8.2", include_str!("fixtures/bootstrap-v1.8.2.md")),
        ("1.8.3", include_str!("fixtures/bootstrap-v1.8.3.md")),
        ("1.8.4", include_str!("fixtures/bootstrap-v1.8.4.md")),
        ("1.8.18", include_str!("fixtures/bootstrap-v1.8.18.md")),
        ("1.8.19", include_str!("fixtures/bootstrap-v1.8.19.md")),
    ] {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        let root = home.join(".codex/skills");
        let ordinary = root.join("ordinary");
        let bootstrap = root.join("skillroster/SKILL.md");
        fs::create_dir_all(&ordinary).unwrap();
        fs::write(
            ordinary.join("SKILL.md"),
            "---\nname: ordinary\ndescription: fixture\n---\n",
        )
        .unwrap();
        fs::create_dir_all(bootstrap.parent().unwrap()).unwrap();
        fs::write(&bootstrap, legacy).unwrap();
        let common = [
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
        ];
        json_output(&run(&[&common[..], &["scan"]].concat(), None));

        let upgrade = json_output(&run(&[&common[..], &["setup"]].concat(), None));
        assert_eq!(upgrade["result"]["state"], "preview_ready");
        assert_eq!(upgrade["result"]["outdated_count"], 1);
        assert_eq!(upgrade["result"]["modified_count"], 0);
        assert_eq!(upgrade["result"]["replace_count"], 1);
        assert_eq!(
            upgrade["result"]["targets"][0]["status"],
            "official_outdated"
        );
        assert_eq!(
            upgrade["result"]["targets"][0]["installed_version"],
            legacy_version
        );
        assert_eq!(fs::read_to_string(&bootstrap).unwrap(), legacy);

        let plan_id = upgrade["result"]["plan_id"].as_str().unwrap();
        let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
        assert_eq!(applied["result"]["verification"], "passed");
        assert_eq!(
            fs::read_to_string(&bootstrap).unwrap(),
            include_str!("../skill/skillroster/SKILL.md").replace("\r\n", "\n")
        );
        json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
        let current = json_output(&run(&[&common[..], &["setup"]].concat(), None));
        assert_eq!(current["result"]["state"], "up_to_date");

        let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
        let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
        assert_eq!(undone["result"]["verification"], "passed");
        assert_eq!(fs::read_to_string(&bootstrap).unwrap(), legacy);
        assert!(!bootstrap.parent().unwrap().join("references").exists());
    }
}

#[test]
fn setup_upgrades_the_public_v1_8_23_package_and_undo_restores_every_file() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let package = home.join(".codex/skills/skillroster");
    let legacy_files = [
        (
            "SKILL.md",
            include_str!("fixtures/bootstrap-v1.8.23.md").to_owned(),
        ),
        (
            "references/routing.md",
            include_str!("fixtures/bootstrap-routing-v1.8.23.md").to_owned(),
        ),
        (
            "references/governance.md",
            include_str!("fixtures/bootstrap-governance-v1.8.23.md").to_owned(),
        ),
        (
            "references/mutation.md",
            include_str!("fixtures/bootstrap-mutation-v1.8.23.md").to_owned(),
        ),
    ];
    for (relative_path, content) in &legacy_files {
        let target = package.join(relative_path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, content).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let preview = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(preview["result"]["state"], "preview_ready");
    assert_eq!(preview["result"]["outdated_count"], 1);
    assert_eq!(preview["result"]["modified_count"], 0);
    assert_eq!(
        preview["result"]["targets"][0]["status"],
        "official_outdated"
    );
    assert_eq!(
        preview["result"]["targets"][0]["installed_version"],
        "1.8.23"
    );
    assert_eq!(preview["result"]["operation_groups"]["replace_file"], 4);
    assert_eq!(preview["result"]["files_changed"], false);

    let plan_id = preview["result"]["plan_id"].as_str().unwrap();
    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(applied["result"]["verification"], "passed");
    assert_eq!(applied["result"]["changed_path_count"], 4);
    assert_eq!(
        applied["result"]["changed_paths"].as_array().unwrap().len(),
        4
    );
    assert_eq!(applied["result"]["changed_paths_truncated"], false);
    for (relative_path, expected) in [
        ("SKILL.md", include_str!("../skill/skillroster/SKILL.md")),
        (
            "references/routing.md",
            include_str!("../skill/skillroster/references/routing.md"),
        ),
        (
            "references/governance.md",
            include_str!("../skill/skillroster/references/governance.md"),
        ),
        (
            "references/mutation.md",
            include_str!("../skill/skillroster/references/mutation.md"),
        ),
    ] {
        assert_eq!(
            fs::read_to_string(package.join(relative_path))
                .unwrap()
                .replace("\r\n", "\n"),
            expected.replace("\r\n", "\n")
        );
    }
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let current = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(current["result"]["state"], "up_to_date");
    assert_eq!(
        current["result"]["targets"][0]["installed_version"],
        "1.8.29"
    );

    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(undone["result"]["changed_path_count"], 4);
    assert_eq!(
        undone["result"]["changed_paths"].as_array().unwrap().len(),
        4
    );
    assert_eq!(undone["result"]["changed_paths_truncated"], false);
    for (relative_path, expected) in legacy_files {
        assert_eq!(
            fs::read_to_string(package.join(relative_path)).unwrap(),
            expected
        );
    }
}

#[test]
fn setup_manages_the_fixed_bootstrap_package_and_preserves_unmanaged_files() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    json_output(&run(
        &[
            &common[..],
            &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let package = root.join("skillroster");
    let routing = package.join("references/routing.md");
    let unmanaged = package.join("notes.local.md");
    fs::write(&unmanaged, "keep me\n").unwrap();
    fs::write(&routing, "local routing instructions\n").unwrap();

    let blocked = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(blocked["result"]["state"], "modified_choice_required");
    assert_eq!(blocked["result"]["modified_count"], 1);
    assert!(blocked["result"]["plan_id"].is_null());

    let retained = json_output(&run(
        &[&common[..], &["setup", "--modified-choice", "retain-local"]].concat(),
        None,
    ));
    assert_eq!(retained["result"]["state"], "local_modifications_retained");
    assert_eq!(
        fs::read_to_string(&routing).unwrap(),
        "local routing instructions\n"
    );

    let adopt = json_output(&run(
        &[
            &common[..],
            &["setup", "--modified-choice", "adopt-current"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(adopt["result"]["operation_count"], 1);
    let adopted = json_output(&run(
        &[
            &common[..],
            &["apply", adopt["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(
        fs::read_to_string(&routing).unwrap(),
        include_str!("../skill/skillroster/references/routing.md").replace("\r\n", "\n")
    );
    assert_eq!(fs::read_to_string(&unmanaged).unwrap(), "keep me\n");
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let healthy = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(healthy["result"]["state"], "up_to_date");
    assert_eq!(healthy["result"]["targets"][0]["managed_file_count"], 4);

    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", adopted["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(
        fs::read_to_string(&routing).unwrap(),
        "local routing instructions\n"
    );
    assert_eq!(fs::read_to_string(&unmanaged).unwrap(), "keep me\n");
}

#[test]
fn setup_does_not_report_a_package_with_a_missing_reference_as_current() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    json_output(&run(
        &[
            &common[..],
            &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    fs::remove_file(root.join("skillroster/references/governance.md")).unwrap();

    let result = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(result["result"]["state"], "modified_choice_required");
    assert_eq!(result["result"]["current_count"], 0);
    assert_eq!(result["result"]["modified_count"], 1);
    assert!(result["result"]["plan_id"].is_null());
    assert!(
        result["result"]["targets"][0]["managed_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| {
                file["relative_path"] == "references/governance.md" && file["status"] == "missing"
            })
    );

    let adopt = json_output(&run(
        &[
            &common[..],
            &["setup", "--modified-choice", "adopt-current"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(adopt["result"]["operation_count"], 1);
    let plan_id = adopt["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    assert_eq!(detail["result"]["operations"].as_array().unwrap().len(), 1);
    assert_eq!(detail["result"]["operations"][0]["kind"], "write_file");
    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert!(root.join("skillroster/references/governance.md").is_file());
    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert!(!root.join("skillroster/references/governance.md").exists());
}

#[test]
fn setup_treats_a_package_with_no_managed_files_as_missing_and_preserves_extra_files() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let package = root.join("skillroster");
    fs::create_dir_all(&package).unwrap();
    let extra = package.join("notes.local.md");
    fs::write(&extra, "keep me\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(setup["result"]["state"], "preview_ready");
    assert_eq!(setup["result"]["missing_count"], 1);
    assert_eq!(setup["result"]["modified_count"], 0);
    assert_eq!(setup["result"]["targets"][0]["status"], "missing");
    assert_eq!(setup["result"]["operation_count"], 5);
    assert_eq!(setup["result"]["affected"]["agent_count"], 1);
    assert_eq!(setup["result"]["affected"]["agents"], json!(["codex"]));
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(fs::read_to_string(&extra).unwrap(), "keep me\n");
    assert!(package.join("SKILL.md").is_file());
    assert!(package.join("references/mutation.md").is_file());

    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(fs::read_to_string(&extra).unwrap(), "keep me\n");
    assert!(!package.join("SKILL.md").exists());
    assert!(!package.join("references").exists());
}

#[cfg(unix)]
#[test]
fn setup_rejects_a_symlinked_managed_reference() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    json_output(&run(
        &[
            &common[..],
            &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let routing = root.join("skillroster/references/routing.md");
    fs::remove_file(&routing).unwrap();
    symlink(root.join("skillroster/SKILL.md"), &routing).unwrap();

    let result = json_output(&run(
        &[
            &common[..],
            &["setup", "--modified-choice", "adopt-current"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(result["result"]["state"], "unsupported_targets");
    assert_setup_versions(&result);
    assert_eq!(result["result"]["unsupported_count"], 1);
    assert!(result["result"]["plan_id"].is_null());
    assert!(
        fs::symlink_metadata(routing)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn setup_deduplicates_shared_agent_roots_and_undo_restores_each_physical_root() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let shared = home.join(".agents_skills");
    let opencode = home.join(".config/opencode/skills");
    let hermes = home.join(".hermes/skills");
    let cursor = home.join(".cursor/skills");
    let gemini = home.join(".gemini/skills");
    let copilot = home.join(".copilot/skills");
    let physical_roots = [&shared, &opencode, &hermes, &cursor, &gemini, &copilot];
    for root in physical_roots {
        fs::create_dir_all(root).unwrap();
    }
    for (parent, link) in [
        (home.join(".codex"), home.join(".codex/skills")),
        (home.join(".claude"), home.join(".claude/skills")),
        (home.join(".pi/agent"), home.join(".pi/agent/skills")),
    ] {
        fs::create_dir_all(parent).unwrap();
        symlink(&shared, link).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(setup["result"]["state"], "preview_ready");
    assert_eq!(
        setup["result"]["detected_agents"].as_array().unwrap().len(),
        8
    );
    assert_eq!(setup["result"]["targets"].as_array().unwrap().len(), 8);
    assert_eq!(setup["result"]["missing_count"], 8);
    assert_eq!(setup["result"]["physical_target_count"], 6);
    assert_eq!(setup["result"]["operation_count"], 36);
    assert_eq!(setup["result"]["affected"]["agent_count"], 8);
    assert_eq!(
        setup["result"]["affected"]["agents"],
        json!([
            "claude-code",
            "codex",
            "cursor",
            "gemini-cli",
            "github-copilot",
            "hermes",
            "opencode",
            "pi"
        ])
    );

    let plan_id = setup["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    for field in [
        "snapshot_id",
        "digest",
        "change_summary",
        "operation_groups",
        "affected",
        "diff_summary",
        "impact",
        "risk",
        "reversible",
        "canonical_deletion_count",
        "confirmation_required",
        "files_changed",
    ] {
        assert_eq!(setup["result"][field], detail["result"][field], "{field}");
    }
    assert_eq!(setup["result"]["detail"]["available"], true);
    assert_eq!(
        setup["result"]["detail"]["command"],
        json!(["plan", "--show", plan_id, "--json"])
    );
    assert_eq!(setup["result"]["change_summary"]["operation_count"], 36);
    assert_eq!(setup["result"]["operation_groups"]["create_directory"], 12);
    assert_eq!(setup["result"]["operation_groups"]["write_file"], 24);
    assert_eq!(setup["result"]["risk"], "filesystem_change");
    assert_eq!(setup["result"]["reversible"], true);
    assert_eq!(setup["result"]["confirmation_required"], true);
    assert_eq!(setup["result"]["files_changed"], false);
    assert!(setup["result"].get("operations").is_none());
    let setup_json = serde_json::to_string(&setup).unwrap();
    assert!(!setup_json.contains("\"content\":"));
    assert!(!setup_json.contains("expected_fingerprint"));
    assert!(serde_json::to_vec(&setup).unwrap().len() < serde_json::to_vec(&detail).unwrap().len());
    let operations = detail["result"]["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 36);
    let unique_targets = operations
        .iter()
        .map(|operation| operation["target"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique_targets.len(), 36);

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 1);
    assert!(status["result"]["last_receipt"].is_null());
    assert_eq!(status["result"]["recovery_state"], "clear");

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(applied["result"]["changed_path_count"], 36);
    assert_eq!(
        applied["result"]["changed_paths"].as_array().unwrap().len(),
        10
    );
    assert_eq!(applied["result"]["changed_paths_truncated"], true);
    let applied_paths = applied["result"]["changed_paths"].as_array().unwrap();
    assert!(
        applied_paths
            .windows(2)
            .all(|pair| { pair[0].as_str().unwrap() <= pair[1].as_str().unwrap() })
    );
    for root in physical_roots {
        assert!(root.join("skillroster/SKILL.md").is_file());
        assert!(root.join("skillroster/references/routing.md").is_file());
        assert!(root.join("skillroster/references/governance.md").is_file());
        assert!(root.join("skillroster/references/mutation.md").is_file());
    }
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let persisted_receipt: Value = serde_json::from_slice(
        &fs::read(state.join("receipts").join(format!("{receipt_id}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        persisted_receipt["changed_paths"].as_array().unwrap().len(),
        36
    );
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(undone["result"]["changed_path_count"], 36);
    assert_eq!(
        undone["result"]["changed_paths"].as_array().unwrap().len(),
        10
    );
    assert_eq!(undone["result"]["changed_paths_truncated"], true);
    for root in physical_roots {
        assert!(!root.join("skillroster").exists());
    }
}

#[test]
fn setup_reuse_and_status_actionability_follow_snapshot_lifecycle() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/skills")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let first = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let retry = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(first["result"]["state"], "preview_ready");
    assert_eq!(retry["result"]["state"], "preview_ready");
    assert_eq!(retry["result"]["plan_id"], first["result"]["plan_id"]);
    assert_eq!(first["result"]["files_changed"], false);
    assert_eq!(retry["result"]["files_changed"], false);

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 1);
    assert_eq!(
        status["result"]["pending_plans"][0]["plan_id"],
        first["result"]["plan_id"]
    );

    let first_plan_id = first["result"]["plan_id"].as_str().unwrap();
    let applied = json_output(&run(
        &[&common[..], &["apply", first_plan_id]].concat(),
        None,
    ));
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");

    let after_terminal_state = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(after_terminal_state["result"]["state"], "preview_ready");
    assert_ne!(
        after_terminal_state["result"]["plan_id"],
        first["result"]["plan_id"]
    );
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 1);
    assert_eq!(
        status["result"]["pending_plans"][0]["plan_id"],
        after_terminal_state["result"]["plan_id"]
    );

    let explicit_choice = json_output(&run(
        &[&common[..], &["setup", "--modified-choice", "retain-local"]].concat(),
        None,
    ));
    assert_ne!(
        explicit_choice["result"]["plan_id"],
        after_terminal_state["result"]["plan_id"]
    );
    let explicit_retry = json_output(&run(
        &[&common[..], &["setup", "--modified-choice", "retain-local"]].concat(),
        None,
    ));
    assert_eq!(
        explicit_retry["result"]["plan_id"],
        explicit_choice["result"]["plan_id"]
    );
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 2);
    assert_eq!(
        status["suggested_actions"][0]["argv"],
        context_action_argv(
            &home,
            &state,
            &[
                "plan",
                "--show",
                explicit_choice["result"]["plan_id"].as_str().unwrap(),
                "--json"
            ]
        )
    );

    let default_after_explicit = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(
        default_after_explicit["result"]["plan_id"],
        after_terminal_state["result"]["plan_id"]
    );
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 2);

    let newer_scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let after_new_snapshot = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_ne!(
        after_new_snapshot["result"]["plan_id"],
        default_after_explicit["result"]["plan_id"]
    );
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 1);
    assert_eq!(
        status["result"]["pending_plans"][0]["plan_id"],
        after_new_snapshot["result"]["plan_id"]
    );
    assert_eq!(
        status["result"]["pending_plans"][0]["snapshot_id"],
        newer_scan["result"]["snapshot_id"]
    );
    assert_eq!(status["result"]["files_changed"], false);

    let old_plan_id = default_after_explicit["result"]["plan_id"]
        .as_str()
        .unwrap();
    let retained = json_output(&run(
        &[&common[..], &["plan", "--show", old_plan_id]].concat(),
        None,
    ));
    assert_eq!(retained["result"]["plan_id"], old_plan_id);
    let stale_apply = run(&[&common[..], &["apply", old_plan_id]].concat(), None);
    assert!(!stale_apply.status.success());
    let stale_apply: Value = serde_json::from_slice(&stale_apply.stdout).unwrap();
    assert_eq!(stale_apply["error"]["code"], "state_drift");
    assert_eq!(
        stale_apply["error"]["details"]["reason"],
        "plan_snapshot_stale"
    );
    assert_eq!(stale_apply["error"]["details"]["plan_id"], old_plan_id);
    assert_eq!(
        stale_apply["error"]["details"]["current_snapshot_id"],
        newer_scan["result"]["snapshot_id"]
    );
    assert_ne!(
        stale_apply["error"]["details"]["expected_snapshot_id"],
        stale_apply["error"]["details"]["current_snapshot_id"]
    );
    assert_eq!(stale_apply["error"]["details"]["files_changed"], false);
    assert!(
        stale_apply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("newer Snapshot exists")
    );

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    for lifecycle_status in ["applying", "recovery_required"] {
        database
            .execute(
                "UPDATE plans SET status = ?1 WHERE id = ?2",
                rusqlite::params![lifecycle_status, old_plan_id],
            )
            .unwrap();
        let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
        assert_eq!(status["result"]["pending_plan_count"], 2);
        let retained_lifecycle = status["result"]["pending_plans"]
            .as_array()
            .unwrap()
            .iter()
            .find(|plan| plan["plan_id"] == old_plan_id)
            .unwrap();
        assert_eq!(retained_lifecycle["status"], lifecycle_status);
    }
}

#[test]
fn setup_does_not_reuse_an_incomplete_legacy_plan_summary() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let first = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let first_plan_id = first["result"]["plan_id"].as_str().unwrap();

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let immutable: String = database
        .query_row(
            "SELECT immutable_json FROM plans WHERE id = ?1",
            [first_plan_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut legacy: Value = serde_json::from_str(&immutable).unwrap();
    let summary = legacy["input"]["summary"].as_object_mut().unwrap();
    summary.remove("operation_groups");
    summary.remove("affected");
    database
        .execute(
            "UPDATE plans SET immutable_json = ?1 WHERE id = ?2",
            rusqlite::params![legacy.to_string(), first_plan_id],
        )
        .unwrap();
    drop(database);

    let retry = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(retry["result"]["state"], "preview_ready");
    assert_ne!(retry["result"]["plan_id"], first["result"]["plan_id"]);
    assert_eq!(retry["result"]["operation_groups"]["create_directory"], 2);
    assert_eq!(retry["result"]["operation_groups"]["write_file"], 4);
    assert!(retry["result"]["affected"].is_object());
    assert_eq!(retry["result"]["confirmation_required"], true);
    assert_eq!(retry["result"]["files_changed"], false);
    assert!(!root.join("skillroster").exists());

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["pending_plan_count"], 2);
    assert!(status["result"]["last_receipt"].is_null());
}

#[test]
fn setup_does_not_reuse_a_legacy_zero_affected_agent_summary() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let first = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let first_plan_id = first["result"]["plan_id"].as_str().unwrap();

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let immutable: String = database
        .query_row(
            "SELECT immutable_json FROM plans WHERE id = ?1",
            [first_plan_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut legacy: Value = serde_json::from_str(&immutable).unwrap();
    legacy["input"]["summary"]["affected"]["agent_count"] = json!(0);
    legacy["input"]["summary"]["affected"]["agents"] = json!([]);
    legacy["input"]["reuse_identity"]
        .as_object_mut()
        .unwrap()
        .remove("affected_agents");
    database
        .execute(
            "UPDATE plans SET immutable_json = ?1 WHERE id = ?2",
            rusqlite::params![legacy.to_string(), first_plan_id],
        )
        .unwrap();
    drop(database);

    let retry = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(retry["result"]["state"], "preview_ready");
    assert_ne!(retry["result"]["plan_id"], first["result"]["plan_id"]);
    assert_eq!(retry["result"]["affected"]["agent_count"], 1);
    assert_eq!(retry["result"]["affected"]["agents"], json!(["codex"]));
    assert!(!root.join("skillroster").exists());
}

#[test]
fn setup_without_a_snapshot_returns_a_typed_scan_action() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let output = json_output(&run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "setup",
        ],
        None,
    ));

    assert_eq!(output["result"]["state"], "scan_required");
    assert_setup_versions(&output);
    assert_eq!(output["suggested_actions"].as_array().unwrap().len(), 1);
    assert_eq!(
        output["suggested_actions"][0]["argv"],
        context_action_argv(&home, &state, &["scan", "--summary", "--json"])
    );
}

#[test]
fn setup_bootstraps_a_detected_agent_before_its_first_skill_root_exists() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let sessions = home.join(".codex/sessions");
    let skill_root = home.join(".codex/skills");
    fs::create_dir_all(&sessions).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let scan = json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    assert_eq!(scan["result"]["placement_count"], 0);
    assert_eq!(
        scan["result"]["root_counts"],
        json!([
            {"count": 8, "kind": "skills", "status": "missing"},
            {"count": 1, "kind": "sessions", "status": "included"},
            {"count": 7, "kind": "sessions", "status": "missing"}
        ])
    );
    assert!(!skill_root.exists());

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(setup["result"]["state"], "preview_ready");
    assert_setup_versions(&setup);
    assert_eq!(
        setup["result"]["detected_agents"],
        json!([{
            "agent": "codex",
            "detection_basis": "included_session_root",
            "target": skill_root.join("skillroster").join("SKILL.md")
        }])
    );
    assert_eq!(setup["result"]["missing_count"], 1);
    assert_eq!(setup["result"]["physical_target_count"], 1);
    assert_eq!(setup["result"]["operation_count"], 7);
    assert_eq!(setup["result"]["operation_groups"]["create_directory"], 3);
    assert_eq!(setup["result"]["operation_groups"]["write_file"], 4);
    assert_eq!(setup["result"]["canonical_deletion_count"], 0);
    assert_eq!(setup["result"]["files_changed"], false);
    assert_eq!(setup["result"]["confirmation_required"], true);
    assert!(!skill_root.exists());

    let plan_id = setup["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    let physical_skill_root = fs::canonicalize(skill_root.parent().unwrap())
        .unwrap()
        .join("skills");
    assert!(
        detail["result"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| {
                operation["kind"] == "create_directory"
                    && operation["target"] == json!(physical_skill_root)
            })
    );

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(applied["result"]["verification"], "passed");
    assert!(skill_root.join("skillroster/SKILL.md").is_file());
    assert!(
        skill_root
            .join("skillroster/references/routing.md")
            .is_file()
    );
    assert!(
        skill_root
            .join("skillroster/references/governance.md")
            .is_file()
    );
    assert!(
        skill_root
            .join("skillroster/references/mutation.md")
            .is_file()
    );

    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    assert!(!skill_root.exists());
    assert!(sessions.is_dir());
}

#[test]
fn verified_apply_invalidates_inventory_until_scan_or_exact_undo() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let original_scan = json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    assert_eq!(original_scan["result"]["placement_count"], 0);
    let pre_apply_report = json_output(&run(&[&common[..], &["report", "--full"]].concat(), None));
    let finding_id = pre_apply_report["result"]["findings"]
        .as_array()
        .and_then(|findings| findings.first())
        .and_then(|finding| finding["id"].as_str())
        .expect("the known-missing-root Snapshot exposes a Finding")
        .to_owned();
    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let plan_id = setup["result"]["plan_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let immutable: String = database
        .query_row(
            "SELECT immutable_json FROM plans WHERE id = ?1",
            [plan_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut alternative: Value = serde_json::from_str(&immutable).unwrap();
    alternative["id"] = json!("plan_alternative_ready");
    database
        .execute(
            "INSERT INTO plans
                (id, scan_id, report_id, created_at, status, input_json, fingerprint, immutable_json)
             SELECT 'plan_alternative_ready', scan_id, report_id, created_at + 1,
                    status, input_json, fingerprint, ?1
             FROM plans WHERE id = ?2",
            rusqlite::params![alternative.to_string(), plan_id],
        )
        .unwrap();
    let ready_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(ready_status["result"]["pending_plan_count"], 2);
    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    let bootstrap = home.join(".codex/skills/skillroster");

    assert_eq!(applied["result"]["verification"], "passed");
    assert!(bootstrap.is_dir());
    assert_eq!(applied["result"]["rescan_required"], true);
    assert_eq!(applied["suggested_actions"].as_array().unwrap().len(), 2);
    assert_eq!(applied["suggested_actions"][0]["action"], "scan");
    assert_eq!(
        applied["suggested_actions"][0]["argv"],
        context_action_argv(&home, &state, &["scan", "--summary", "--json"])
    );
    assert_eq!(applied["suggested_actions"][0]["mutates"], false);
    assert_eq!(
        applied["suggested_actions"][0]["requires_confirmation"],
        false
    );
    assert_eq!(applied["suggested_actions"][1]["action"], "undo");

    for command in [
        vec!["report", "--summary"],
        vec!["find", "help me govern skills", "--load", "--limit", "1"],
        vec!["setup"],
    ] {
        let blocked = run(&[&common[..], &command].concat(), None);
        assert!(!blocked.status.success());
        let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
        assert_eq!(blocked["error"]["code"], "snapshot_rescan_required");
        assert_eq!(blocked["error"]["retryable"], true);
        assert_eq!(
            blocked["error"]["details"]["reason"],
            "verified_mutation_after_snapshot"
        );
        assert_eq!(
            blocked["error"]["details"]["snapshot_id"],
            original_scan["result"]["snapshot_id"]
        );
        assert_eq!(blocked["error"]["details"]["files_changed"], false);
        assert_eq!(blocked["suggested_actions"].as_array().unwrap().len(), 1);
        assert_eq!(blocked["suggested_actions"][0]["action"], "scan");
        assert_eq!(
            blocked["suggested_actions"][0]["argv"],
            context_action_argv(&home, &state, &["scan", "--summary", "--json"])
        );
    }
    let blocked_plan = run(&[&common[..], &["plan", "--stdin"]].concat(), Some("{}"));
    assert!(!blocked_plan.status.success());
    let blocked_plan: Value = serde_json::from_slice(&blocked_plan.stdout).unwrap();
    assert_eq!(blocked_plan["error"]["code"], "snapshot_rescan_required");
    assert_eq!(blocked_plan["suggested_actions"][0]["action"], "scan");
    let blocked_finding = run(
        &[&common[..], &["report", "--finding", &finding_id]].concat(),
        None,
    );
    assert!(!blocked_finding.status.success());
    let blocked_finding: Value = serde_json::from_slice(&blocked_finding.stdout).unwrap();
    assert_eq!(blocked_finding["error"]["code"], "snapshot_rescan_required");

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["recovery_state"], "clear");
    assert_eq!(status["result"]["snapshot_state"], "rescan_required");
    assert_eq!(
        status["result"]["snapshot_invalidated_by_receipt_id"],
        applied["result"]["receipt_id"]
    );
    assert_eq!(status["result"]["pending_plan_count"], 0);
    assert_eq!(status["result"]["pending_plans"], json!([]));
    assert_eq!(status["suggested_actions"].as_array().unwrap().len(), 2);
    assert_eq!(status["suggested_actions"][0]["action"], "scan");
    assert_eq!(
        status["result"]["next_action"],
        status["suggested_actions"][0]
    );
    assert_eq!(status["suggested_actions"][1]["action"], "undo");
    assert_eq!(
        status["suggested_actions"][1]["argv"],
        context_action_argv(
            &home,
            &state,
            &[
                "undo",
                applied["result"]["receipt_id"].as_str().unwrap(),
                "--json",
            ],
        )
    );
    assert_eq!(status["suggested_actions"][1]["mutates"], true);
    assert_eq!(
        status["suggested_actions"][1]["reason_code"],
        "receipt_undoable"
    );
    assert_eq!(
        status["suggested_actions"][1]["requires_confirmation"],
        true
    );
    let home_result = json_output(&run(&common, None));
    assert_eq!(home_result["result"]["state"], "rescan_required");
    assert_eq!(home_result["result"]["snapshot_state"], "rescan_required");
    assert_eq!(
        home_result["result"]["snapshot_invalidated_by_receipt_id"],
        applied["result"]["receipt_id"]
    );
    assert_eq!(
        home_result["suggested_actions"],
        status["suggested_actions"]
    );
    let undone = json_output(&run_suggested_action(&home_result["suggested_actions"][1]));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(
        undone["result"]["reverses_receipt_id"],
        applied["result"]["receipt_id"]
    );
    assert!(!bootstrap.exists());
    let restored = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    assert_eq!(
        restored["result"]["snapshot_id"],
        original_scan["result"]["snapshot_id"]
    );
    assert_eq!(restored["result"]["placement_count"], 0);
}

#[test]
fn suggested_scan_refreshes_inventory_after_verified_apply() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    let original_scan = json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));

    let refreshed = json_output(&run_suggested_action(&applied["suggested_actions"][0]));
    assert_ne!(
        refreshed["result"]["snapshot_id"],
        original_scan["result"]["snapshot_id"]
    );
    assert_eq!(refreshed["result"]["placement_count"], 1);
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    assert_eq!(
        report["result"]["snapshot_id"],
        refreshed["result"]["snapshot_id"]
    );
    assert_eq!(report["result"]["placement_count"], 1);
    let found = json_output(&run(
        &[
            &common[..],
            &["find", "help me govern skills", "--load", "--limit", "1"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(found["result"]["loaded_skill"]["selection"]["rank"], 1);
    assert_eq!(found["result"]["loaded_skill"]["content"]["complete"], true);
}

#[test]
fn exact_undo_invalidates_a_newer_post_apply_snapshot() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", setup["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    let post_apply_scan = json_output(&run_suggested_action(&applied["suggested_actions"][0]));
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));

    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(undone["result"]["rescan_required"], true);
    assert_eq!(undone["suggested_actions"].as_array().unwrap().len(), 1);
    assert_eq!(undone["suggested_actions"][0]["action"], "scan");
    let blocked = run(&[&common[..], &["report", "--summary"]].concat(), None);
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(blocked["error"]["code"], "snapshot_rescan_required");
    assert_eq!(
        blocked["error"]["details"]["snapshot_id"],
        post_apply_scan["result"]["snapshot_id"]
    );
    let resumed_home = json_output(&run(&common, None));
    let resumed_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(
        resumed_home["suggested_actions"],
        resumed_status["suggested_actions"]
    );
    assert_eq!(
        resumed_home["suggested_actions"].as_array().unwrap().len(),
        1
    );
    assert_eq!(resumed_home["suggested_actions"][0]["action"], "scan");

    let refreshed = json_output(&run_suggested_action(&undone["suggested_actions"][0]));
    assert_eq!(refreshed["result"]["placement_count"], 0);
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    assert_eq!(report["result"]["placement_count"], 0);
}

#[test]
fn setup_builds_only_the_fixed_missing_skill_root_chain_for_a_detected_agent() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let sessions = home.join(".local/share/opencode/storage/session");
    let config = home.join(".config");
    let skill_root = config.join("opencode/skills");
    fs::create_dir_all(&sessions).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(setup["result"]["state"], "preview_ready");
    assert_eq!(
        setup["result"]["detected_agents"],
        json!([{
            "agent": "opencode",
            "detection_basis": "included_session_root",
            "target": skill_root.join("skillroster").join("SKILL.md")
        }])
    );
    assert_eq!(setup["result"]["operation_count"], 9);
    assert_eq!(setup["result"]["operation_groups"]["create_directory"], 5);
    assert_eq!(setup["result"]["operation_groups"]["write_file"], 4);
    assert!(!config.exists());

    let plan_id = setup["result"]["plan_id"].as_str().unwrap();
    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(applied["result"]["verification"], "passed");
    assert!(skill_root.join("skillroster/SKILL.md").is_file());

    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    assert!(!config.exists());
    assert!(sessions.is_dir());
}

#[test]
fn setup_does_not_invent_agent_presence_from_known_missing_roots() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(&home).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));

    let setup = json_output(&run(&[&common[..], &["setup"]].concat(), None));
    assert_eq!(setup["result"]["state"], "no_supported_agent");
    assert_eq!(setup["result"]["detected_agents"], json!([]));
    assert!(setup["result"]["plan_id"].is_null());
    assert_eq!(setup["result"]["files_changed"], false);
    assert_eq!(setup["suggested_actions"], json!([]));
    assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
}

#[test]
fn setup_preserves_unsupported_bootstrap_targets_without_creating_a_plan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let ordinary = root.join("ordinary");
    fs::create_dir_all(&ordinary).unwrap();
    fs::write(
        ordinary.join("SKILL.md"),
        "---\nname: ordinary\ndescription: fixture\n---\n",
    )
    .unwrap();
    let blocked_parent = root.join("skillroster");
    fs::write(&blocked_parent, "user-owned non-directory target\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    for setup_args in [
        vec!["setup"],
        vec!["setup", "--modified-choice", "adopt-current"],
    ] {
        let output = json_output(&run(&[&common[..], &setup_args].concat(), None));
        assert_eq!(output["result"]["state"], "unsupported_targets");
        assert_eq!(output["result"]["unsupported_count"], 1);
        assert!(output["result"]["plan_id"].is_null());
        assert_eq!(output["result"]["targets"][0]["status"], "unsupported");
        assert_eq!(
            fs::read_to_string(&blocked_parent).unwrap(),
            "user-owned non-directory target\n"
        );
    }
}

#[test]
fn agent_plan_refuses_arbitrary_skill_root_write_file() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let skill = root.join("example");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: example\n---\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let input = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "operations": [{
            "kind": "write_file",
            "target": root.join("arbitrary.txt"),
            "content": "not governance\n",
            "expected_fingerprint": "missing"
        }]
    })
    .to_string();
    let rejected = run(&[&common[..], &["plan", "--stdin"]].concat(), Some(&input));
    assert!(!rejected.status.success());
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected["ok"], false);
    assert!(!root.join("arbitrary.txt").exists());

    let missing_evidence = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "roster_changes": [{
            "agent": "codex",
            "skill_id": "skill_missing",
            "state": "core"
        }]
    })
    .to_string();
    let missing_evidence = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&missing_evidence),
    );
    assert!(!missing_evidence.status.success());
    let missing_evidence: Value = serde_json::from_slice(&missing_evidence.stdout).unwrap();
    assert!(
        missing_evidence["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing_evidence")
    );

    let missing_skill = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "roster_changes": [{
            "agent": "codex",
            "skill_id": "skill_missing",
            "state": "core"
        }]
    })
    .to_string();
    let rejected_skill = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&missing_skill),
    );
    assert!(!rejected_skill.status.success());

    let stale_snapshot = json!({
        "schema_version": 1,
        "scan_id": "scan_missing",
        "roster_changes": [{
            "agent": "codex",
            "skill_id": scan["result"]["snapshot_id"],
            "state": "core"
        }]
    })
    .to_string();
    let rejected_snapshot = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&stale_snapshot),
    );
    assert!(!rejected_snapshot.status.success());

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let plan_count: i64 = database
        .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get(0))
        .unwrap();
    assert_eq!(plan_count, 0, "an invalid Plan must never become Ready");
}

#[test]
fn json_failure_is_one_parseable_document() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    let state = temp.path().join("state");
    let output = run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "report",
        ],
        None,
    );
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "report");
    assert_eq!(value["error"]["code"], "snapshot_required");
    assert_eq!(value["error"]["retryable"], true);
    assert_eq!(
        value["suggested_actions"][0]["argv"],
        context_action_argv(home, &state, &["scan", "--summary", "--json"])
    );
    assert_eq!(
        value["suggested_actions"][0]["reason_code"],
        "snapshot_required"
    );
    assert!(output.stderr.is_empty());

    let invalid = run(&["--json", "not-a-command"], None);
    assert_eq!(invalid.status.code(), Some(2));
    let invalid_value: Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert_eq!(invalid_value["command"], "cli");
    assert_eq!(invalid_value["error"]["code"], "invalid_cli_arguments");
    assert!(invalid.stderr.is_empty());
}

#[test]
fn explicit_roots_preserve_all_eight_agent_identities() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("empty-home");
    let state = temp.path().join("state");
    let agents = [
        "codex",
        "claude-code",
        "pi",
        "opencode",
        "hermes",
        "cursor",
        "gemini-cli",
        "github-copilot",
    ];
    let shared_root = temp.path().join("shared-skills");
    let shared_skill = shared_root.join("shared-fixture");
    fs::create_dir_all(&shared_skill).unwrap();
    fs::write(
        shared_skill.join("SKILL.md"),
        "---\nname: shared-fixture\n---\nshared fixture\n",
    )
    .unwrap();
    let mut args = vec![
        "--home".to_owned(),
        home.display().to_string(),
        "--state-dir".to_owned(),
        state.display().to_string(),
        "--json".to_owned(),
    ];
    for agent in agents {
        args.push("--root".to_owned());
        args.push(format!("{agent}={}", shared_root.display()));
    }
    args.push("scan".to_owned());
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let scan = json_output(&run(&refs, None));
    let explicit = scan["result"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|root| root["explicit"] == true)
        .collect::<Vec<_>>();
    assert_eq!(explicit.len(), 8);
    for agent in agents {
        assert!(explicit.iter().any(|root| root["agent"] == agent));
    }
    assert!(explicit.iter().all(|root| root["status"] == "included"));
    let coverage_agents = scan["result"]["coverage"]
        .as_array()
        .unwrap()
        .iter()
        .map(|coverage| coverage["agent"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(coverage_agents, agents);
    assert_eq!(scan["result"]["placement_count"], 8);
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let assigned_agents: i64 = database
        .query_row(
            "SELECT COUNT(DISTINCT p.agent_id)
             FROM placements p JOIN roots r ON r.id = p.root_id
             WHERE r.explicit = 1 AND p.exposed = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(assigned_agents, 8);
}

#[test]
fn scan_keeps_deep_package_support_out_of_root_discovery_coverage() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("empty-home");
    let state = temp.path().join("state");
    let root = temp.path().join("skills");
    let parent = root.join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::write(
        parent.join("SKILL.md"),
        "---\nname: parent\ndescription: Parent skill\n---\n",
    )
    .unwrap();
    fs::write(
        child.join("SKILL.md"),
        "---\nname: child\ndescription: Nested child skill\n---\n",
    )
    .unwrap();
    fs::create_dir_all(parent.join("references/one/two/three/four/five")).unwrap();

    let explicit_root = format!("codex={}", root.display());
    let scan = json_output(&run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--root",
            &explicit_root,
            "--json",
            "scan",
        ],
        None,
    ));

    assert_eq!(scan["result"]["skill_count"], 2);
    assert_eq!(scan["result"]["placement_count"], 2);
    let observed = scan["result"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|observed| observed["path"] == root.display().to_string())
        .unwrap();
    assert_eq!(observed["status"], "included");
    assert_eq!(observed["discovery_complete"], true);
    assert!(observed["detail"].is_null());
}

#[test]
fn scan_keeps_repository_metadata_out_of_root_discovery_coverage() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("empty-home");
    let state = temp.path().join("state");
    let root = temp.path().join("skills");
    let skill = root.join("example");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: example\ndescription: Example skill\n---\n",
    )
    .unwrap();
    fs::create_dir_all(root.join(".git/logs/refs/remotes/origin/archive")).unwrap();

    let explicit_root = format!("codex={}", root.display());
    let scan = json_output(&run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--root",
            &explicit_root,
            "--json",
            "scan",
        ],
        None,
    ));

    assert_eq!(scan["result"]["skill_count"], 1);
    let observed = scan["result"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|observed| observed["path"] == root.display().to_string())
        .unwrap();
    assert_eq!(observed["discovery_complete"], true);
    assert!(observed["detail"].is_null());
}

#[cfg(unix)]
#[test]
fn scan_excludes_linked_repository_metadata_from_entrypoint_discovery() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("empty-home");
    let state = temp.path().join("state");
    let root = temp.path().join("skills");
    let skill = root.join("example");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&skill).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: example\n---\n").unwrap();
    fs::write(outside.join("SKILL.md"), "---\nname: hidden\n---\n").unwrap();
    symlink(&outside, root.join(".git")).unwrap();

    let explicit_root = format!("codex={}", root.display());
    let scan = json_output(&run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--root",
            &explicit_root,
            "--json",
            "scan",
        ],
        None,
    ));

    assert_eq!(scan["result"]["skill_count"], 1);
    assert_eq!(scan["result"]["placement_count"], 1);
}

#[cfg(unix)]
#[test]
fn scan_ignores_linked_support_without_a_nested_skill_entrypoint() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("empty-home");
    let state = temp.path().join("state");
    let root = temp.path().join("skills");
    let parent = root.join("parent");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&parent).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(parent.join("SKILL.md"), "---\nname: parent\n---\n").unwrap();
    symlink(&outside, parent.join("references")).unwrap();

    let explicit_root = format!("codex={}", root.display());
    let scan = json_output(&run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "--root",
            &explicit_root,
            "--json",
            "scan",
        ],
        None,
    ));

    assert_eq!(scan["result"]["skill_count"], 1);
    assert_eq!(scan["result"]["placement_count"], 1);
    let observed = scan["result"]["roots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|observed| observed["path"] == root.display().to_string())
        .unwrap();
    assert_eq!(observed["discovery_complete"], true);
}

#[test]
fn source_confirmation_crash_temp_does_not_block_lifecycle_cleanup() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let details = state.join("source-confirmation");
    fs::create_dir_all(&details).unwrap();
    let interrupted = details.join(format!(".{}.tmp", ulid::Ulid::new()));
    fs::write(&interrupted, b"incomplete").unwrap();

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(
        status["result"]["retention"]["source_confirmation_details"]["count"],
        0
    );
    assert!(interrupted.is_file());

    let purged = json_output(&run(
        &[
            &common[..],
            &["lifecycle", "purge", "--source-confirmation"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(purged["result"]["removed_source_confirmation_details"], 1);
    assert!(!details.exists());
}

#[test]
fn lifecycle_exclusion_and_database_delete_preserve_user_files() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/example");
    let session = home.join(".codex/sessions/session.jsonl");
    let library = state.join("library/example");
    fs::create_dir_all(&skill).unwrap();
    fs::create_dir_all(session.parent().unwrap()).unwrap();
    fs::create_dir_all(&library).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: example\n---\nbody\n").unwrap();
    fs::write(
        &session,
        "{\"type\":\"invoke_skill\",\"invoked_skill\":\"example\",\"prompt\":\"PRIVATE-CONVERSATION-TEXT\"}\n",
    )
    .unwrap();
    fs::write(library.join("SKILL.md"), "library sentinel\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let excluded = json_output(&run(
        &[&common[..], &["lifecycle", "exclude", "codex"]].concat(),
        None,
    ));
    assert_eq!(excluded["result"]["raw_conversations_copied"], false);
    let rescanned = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert!(
        rescanned["result"]["roots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|root| root["agent"] == "codex"
                && root["kind"] == "sessions"
                && root["status"] == "excluded")
    );
    let summary = json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    assert_eq!(summary["result"]["session_coverage"]["supported_agents"], 8);
    assert_eq!(summary["result"]["session_coverage"]["excluded_agents"], 1);
    assert_eq!(
        summary["result"]["session_coverage"]["agents"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert!(
        summary["result"]["session_coverage"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|agent| agent["agent"] == "codex" && agent["state"] == "excluded")
    );
    assert!(
        summary["result"]["session_coverage"]["next_step_groups"]
            .as_array()
            .unwrap()
            .iter()
            .any(|group| group["agents"]
                .as_array()
                .unwrap()
                .contains(&json!("codex"))
                && group["steps"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("do_not_infer_usage_for_excluded_agent")))
    );
    let inspect = json_output(&run(
        &[&common[..], &["lifecycle", "inspect"]].concat(),
        None,
    ));
    assert_eq!(inspect["result"]["evidence_exclusions"], json!(["codex"]));
    let export_path = temp.path().join("lifecycle-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                export_path.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    assert!(
        !fs::read_to_string(&export_path)
            .unwrap()
            .contains("PRIVATE-CONVERSATION-TEXT")
    );

    let wrong_confirmation = run(
        &[&common[..], &["lifecycle", "delete", "--confirm", "wrong"]].concat(),
        None,
    );
    assert!(!wrong_confirmation.status.success());
    assert!(state.join("skillroster.db").is_file());
    let deleted = json_output(&run(
        &[
            &common[..],
            &["lifecycle", "delete", "--confirm", "DELETE-LOCAL-STATE"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(deleted["result"]["agent_files_changed"], false);
    assert_eq!(deleted["result"]["library_files_changed"], false);
    assert!(!state.join("skillroster.db").exists());
    assert!(skill.join("SKILL.md").is_file());
    assert!(library.join("SKILL.md").is_file());

    let rebuilt = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(rebuilt["result"]["skill_count"], 2);
    assert!(
        rebuilt["result"]["roots"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |root| root["path"] == state.join("library").to_str().unwrap()
                    || fs::canonicalize(root["path"].as_str().unwrap()).ok()
                        == fs::canonicalize(state.join("library")).ok()
            )
    );
    assert!(state.join("skillroster.db").is_file());
}

#[test]
fn unchanged_rescans_do_not_multiply_exported_usage_observations() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/example");
    let session = home.join(".codex/sessions/session.jsonl");
    fs::create_dir_all(&skill).unwrap();
    fs::create_dir_all(session.parent().unwrap()).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: example\n---\nbody\n").unwrap();
    fs::write(
        &session,
        "{\"timestamp\":\"1970-01-01T00:00:10Z\",\"type\":\"invoke_skill\",\"invoked_skill\":\"example\"}\n",
    )
    .unwrap();
    fs::File::options()
        .write(true)
        .open(&session)
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(10)))
        .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let first_export = temp.path().join("first-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                first_export.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let first: Value = serde_json::from_slice(&fs::read(&first_export).unwrap()).unwrap();
    assert_eq!(first["data"]["usage_events"].as_array().unwrap().len(), 1);
    assert_eq!(first["data"]["usage_events"][0]["occurred_at"], 10);

    fs::File::options()
        .write(true)
        .open(&session)
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(15)))
        .unwrap();
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let second_export = temp.path().join("second-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                second_export.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let second: Value = serde_json::from_slice(&fs::read(&second_export).unwrap()).unwrap();
    assert_eq!(second["data"]["usage_events"].as_array().unwrap().len(), 1);

    let mut session_file = OpenOptions::new().append(true).open(&session).unwrap();
    writeln!(
        session_file,
        "{{\"timestamp\":\"1970-01-01T00:00:20Z\",\"type\":\"invoke_skill\",\"invoked_skill\":\"example\"}}"
    )
    .unwrap();
    session_file
        .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(20)))
        .unwrap();
    drop(session_file);
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let third_export = temp.path().join("third-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                third_export.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let third: Value = serde_json::from_slice(&fs::read(&third_export).unwrap()).unwrap();
    assert_eq!(third["data"]["usage_events"].as_array().unwrap().len(), 2);

    json_output(&run(
        &[&common[..], &["lifecycle", "purge", "--raw-days", "1"]].concat(),
        None,
    ));
    let retained_export = temp.path().join("retained-export.json");
    json_output(&run(
        &[
            &common[..],
            &[
                "lifecycle",
                "export",
                "--output",
                retained_export.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let retained: Value = serde_json::from_slice(&fs::read(&retained_export).unwrap()).unwrap();
    assert_eq!(retained["schema_version"], 2);
    assert_eq!(
        retained["usage_history"]["raw_value_field"],
        "observed_event_count"
    );
    assert_eq!(retained["usage_history"]["observations_additive"], false);
    assert_eq!(retained["data"]["usage_events"], json!([]));
    assert!(
        retained["data"]["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .all(|evidence| evidence["kind"] != "usage")
    );
    assert_eq!(
        retained["data"]["usage_monthly_sources"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        retained["data"]["usage_monthly_sources"][0]["max_observed_event_count"],
        2
    );
    assert_eq!(retained["data"]["usage_monthly_legacy"], json!([]));
}

#[test]
fn source_update_requires_conflict_choice_and_round_trips_adoption() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill_dir = home.join(".codex/skills/source-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    let entrypoint = skill_dir.join("SKILL.md");
    let original =
        "---\nname: source-skill\nsource: github:example/source-skill\nversion: v1\n---\nold\n";
    fs::write(&entrypoint, original).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = payload["skills"][0]["id"].as_str().unwrap();
    let placement_id = payload["placements"][0]["id"].as_str().unwrap();
    let current_fingerprint = payload["placements"][0]["content_digest"].as_str().unwrap();
    let evidence_id = skill_evidence_id(&database, snapshot, skill_id);
    let upstream =
        "---\nname: source-skill\nsource: github:example/source-skill\nversion: v2\n---\nnew\n";
    let digest = |content: &str| format!("{:x}", Sha256::digest(content.as_bytes()));
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id.clone()],
        "source_updates": [{
            "skill_id": skill_id,
            "placement_id": placement_id,
            "source": "github:example/source-skill",
            "current_revision": "v1",
            "current_fingerprint": current_fingerprint,
            "base_digest": digest(original),
            "upstream_revision": "v2",
            "upstream_content": upstream,
            "upstream_digest": digest(upstream)
        }]
    });
    let untrusted_default = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!untrusted_default.status.success());
    let untrusted_error: Value = serde_json::from_slice(&untrusted_default.stdout).unwrap();
    assert!(
        untrusted_error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("first-observed and untrusted")
    );
    let mut explicit_adopt = request.clone();
    explicit_adopt["source_updates"][0]["choice"] = json!("adopt_upstream");
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&explicit_adopt.to_string()),
    ));
    assert_eq!(plan["result"]["risk"], "source_update");
    assert_eq!(
        plan["result"]["diff_summary"]["items"][0]["choice_reason"],
        "first_observed_baseline_untrusted"
    );
    assert_eq!(
        plan["result"]["diff_summary"]["items"][0]["choice"],
        "adopt_upstream"
    );
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(fs::read_to_string(&entrypoint).unwrap(), upstream);
    let receipt = applied["result"]["receipt_id"].as_str().unwrap();

    let trusted_digest: String = database
        .query_row(
            "SELECT trusted_digest FROM source_baselines
             WHERE source = 'github:example/source-skill' AND revision = 'v2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(trusted_digest, digest(upstream));
    let trusted_scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let trusted_snapshot = trusted_scan["result"]["snapshot_id"].as_str().unwrap();
    let trusted_payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [trusted_snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let trusted_payload: Value = serde_json::from_str(&trusted_payload).unwrap();
    let upstream_v3 =
        "---\nname: source-skill\nsource: github:example/source-skill\nversion: v3\n---\nnewer\n";
    let trusted_request = json!({
        "schema_version": 1,
        "scan_id": trusted_snapshot,
        "evidence_ids": [database.query_row::<String, _, _>(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [trusted_snapshot],
            |row| row.get(0),
        ).unwrap()],
        "source_updates": [{
            "skill_id": trusted_payload["skills"][0]["id"],
            "placement_id": trusted_payload["placements"][0]["id"],
            "source": "github:example/source-skill",
            "current_revision": "v2",
            "current_fingerprint": trusted_payload["placements"][0]["content_digest"],
            "upstream_revision": "v3",
            "upstream_content": upstream_v3,
            "upstream_digest": digest(upstream_v3)
        }]
    });
    let trusted_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&trusted_request.to_string()),
    ));
    assert_eq!(
        trusted_plan["result"]["diff_summary"]["items"][0]["choice_reason"],
        "trusted_baseline_clean"
    );
    let undone = json_output(&run(&[&common[..], &["undo", receipt]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(fs::read_to_string(&entrypoint).unwrap(), original);

    fs::write(&entrypoint, original.replace("old", "local edit")).unwrap();
    let rescanned = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let new_snapshot = rescanned["result"]["snapshot_id"].as_str().unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [new_snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let modified_request = json!({
        "schema_version": 1,
        "scan_id": new_snapshot,
        "evidence_ids": [database.query_row::<String, _, _>(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [new_snapshot],
            |row| row.get(0),
        ).unwrap()],
        "source_updates": [{
            "skill_id": payload["skills"][0]["id"],
            "placement_id": payload["placements"][0]["id"],
            "source": "github:example/source-skill",
            "current_revision": "v1",
            "current_fingerprint": payload["placements"][0]["content_digest"],
            "base_digest": digest(original),
            "upstream_revision": "v2",
            "upstream_content": upstream,
            "upstream_digest": digest(upstream)
        }]
    });
    let conflict = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&modified_request.to_string()),
    );
    assert!(!conflict.status.success());

    let mut forged_baseline_request = modified_request.clone();
    forged_baseline_request["source_updates"][0]["base_digest"] =
        json!(digest(&original.replace("old", "local edit")));
    let forged_baseline = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&forged_baseline_request.to_string()),
    );
    assert!(
        !forged_baseline.status.success(),
        "a caller-supplied hash of locally edited content must not replace the stored baseline"
    );
    let forged_error: Value = serde_json::from_slice(&forged_baseline.stdout).unwrap();
    assert!(
        forged_error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("immutable source baseline"),
        "unexpected forged-baseline rejection: {forged_error}"
    );

    let mut stale_evidence_request = modified_request.clone();
    stale_evidence_request["evidence_ids"] = json!([evidence_id]);
    stale_evidence_request["source_updates"][0]["choice"] = json!("retain_local");
    let stale_evidence = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&stale_evidence_request.to_string()),
    );
    assert!(!stale_evidence.status.success());

    let mut preserve_request = modified_request.clone();
    preserve_request["source_updates"][0]["choice"] = json!("preserve_both");
    let preserve_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&preserve_request.to_string()),
    ));
    let preserved = json_output(&run(
        &[
            &common[..],
            &[
                "apply",
                preserve_plan["result"]["plan_id"].as_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    ));
    let sibling = skill_dir.join("SKILL.upstream-v2.md");
    assert_eq!(fs::read_to_string(&sibling).unwrap(), upstream);
    assert!(
        fs::read_to_string(&entrypoint)
            .unwrap()
            .contains("local edit")
    );
    let preserve_undo = json_output(&run(
        &[
            &common[..],
            &["undo", preserved["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(preserve_undo["result"]["verification"], "passed");
    assert!(!sibling.exists());

    let mut retain_request = modified_request;
    retain_request["source_updates"][0]["choice"] = json!("retain_local");
    let retain_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&retain_request.to_string()),
    ));
    let retained = json_output(&run(
        &[
            &common[..],
            &["apply", retain_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(retained["result"]["changed_path_count"], 0);
    assert_eq!(retained["result"]["files_changed"], false);
    let retained_undo = json_output(&run(
        &[
            &common[..],
            &["undo", retained["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(retained_undo["result"]["verification"], "passed");

    database
        .execute("DELETE FROM source_baselines", [])
        .unwrap();
    let mut missing_baseline_request = retain_request.clone();
    missing_baseline_request["source_updates"][0]
        .as_object_mut()
        .unwrap()
        .remove("base_digest");
    missing_baseline_request["source_updates"][0]
        .as_object_mut()
        .unwrap()
        .remove("choice");
    let missing_baseline = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&missing_baseline_request.to_string()),
    );
    assert!(
        !missing_baseline.status.success(),
        "a missing baseline must never silently adopt upstream content"
    );
    let missing_error: Value = serde_json::from_slice(&missing_baseline.stdout).unwrap();
    assert!(
        missing_error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("explicit choice is required")
    );
    missing_baseline_request["source_updates"][0]["choice"] = json!("retain_local");
    let explicit_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&missing_baseline_request.to_string()),
    ));
    assert_eq!(explicit_plan["result"]["risk"], "source_update");
}

#[test]
fn library_governance_managed_and_hosted_round_trip_and_refuse_drift() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_skill = home.join(".codex/skills/shared");
    let claude_skill = home.join(".claude/skills/shared");
    let content = "---\nname: shared\ndescription: shared fixture\n---\nbody\n";
    for directory in [&codex_skill, &claude_skill] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = payload["skills"][0]["id"].as_str().unwrap();
    let placements = payload["placements"].as_array().unwrap();
    assert_eq!(placements.len(), 2);
    let canonical = placements
        .iter()
        .find(|placement| placement["agent"] == "codex")
        .unwrap();
    let placement_ids = placements
        .iter()
        .map(|placement| placement["id"].clone())
        .collect::<Vec<_>>();
    let evidence_id = skill_evidence_id(&database, snapshot, skill_id);
    let request = |state_name: &str| {
        json!({
            "schema_version": 1,
            "scan_id": snapshot,
            "evidence_ids": [evidence_id],
            "library_changes": [{
                "skill_id": skill_id,
                "canonical_placement_id": canonical["id"],
                "placement_ids": placement_ids,
                "requested_state": state_name
            }]
        })
    };

    let managed_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request("managed").to_string()),
    ));
    assert_eq!(managed_plan["result"]["risk"], "library_governance");
    let managed_impact = &managed_plan["result"]["impact"]["items"][0];
    assert_eq!(managed_impact["before"]["physical_source_count"], 2);
    assert_eq!(managed_impact["after"]["physical_source_count"], 1);
    assert_eq!(managed_impact["delta"]["physical_source_count"], -1);
    assert_eq!(managed_impact["before"]["placement_count"], 2);
    assert_eq!(managed_impact["after"]["placement_count"], 2);
    assert_eq!(managed_impact["delta"]["placement_count"], 0);
    assert_eq!(
        managed_impact["before"]["default_exposed_placement_count"],
        2
    );
    assert_eq!(
        managed_impact["after"]["default_exposed_placement_count"],
        2
    );
    assert_eq!(
        managed_impact["delta"]["default_exposed_placement_count"],
        0
    );
    assert_eq!(managed_impact["after"]["relinked_placement_count"], 1);
    let managed = json_output(&run(
        &[
            &common[..],
            &["apply", managed_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert!(
        fs::symlink_metadata(&claude_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&claude_skill).unwrap(),
        fs::canonicalize(&codex_skill).unwrap()
    );
    let governance_state: String = database
        .query_row(
            "SELECT governance_state FROM skills WHERE id = ?1",
            [skill_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(governance_state, "managed");
    let managed_undo = json_output(&run(
        &[
            &common[..],
            &["undo", managed["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(
        managed_undo["result"]["verification"], "passed",
        "{managed_undo}"
    );
    assert!(claude_skill.is_dir());
    assert!(
        !fs::symlink_metadata(&claude_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let governance_state: String = database
        .query_row(
            "SELECT governance_state FROM skills WHERE id = ?1",
            [skill_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(governance_state, "observed");

    let hosted_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request("hosted").to_string()),
    ));
    let hosted_impact = &hosted_plan["result"]["impact"]["items"][0];
    assert_eq!(hosted_impact["before"]["physical_source_count"], 2);
    assert_eq!(hosted_impact["after"]["physical_source_count"], 1);
    assert_eq!(hosted_impact["delta"]["physical_source_count"], -1);
    assert_eq!(hosted_impact["before"]["placement_count"], 2);
    assert_eq!(hosted_impact["after"]["placement_count"], 3);
    assert_eq!(hosted_impact["delta"]["placement_count"], 1);
    assert_eq!(
        hosted_impact["before"]["default_exposed_placement_count"],
        2
    );
    assert_eq!(hosted_impact["after"]["default_exposed_placement_count"], 2);
    assert_eq!(hosted_impact["delta"]["default_exposed_placement_count"], 0);
    assert_eq!(hosted_impact["after"]["relinked_placement_count"], 2);
    let hosted = json_output(&run(
        &[
            &common[..],
            &["apply", hosted_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    let library_skill = state.join("library/shared");
    assert!(library_skill.join("SKILL.md").is_file());
    assert_eq!(
        fs::canonicalize(&codex_skill).unwrap(),
        fs::canonicalize(&library_skill).unwrap()
    );
    assert_eq!(
        fs::canonicalize(&claude_skill).unwrap(),
        fs::canonicalize(&library_skill).unwrap()
    );
    let governance_state: String = database
        .query_row(
            "SELECT governance_state FROM skills WHERE id = ?1",
            [skill_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(governance_state, "hosted");
    let hosted_rescan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let hosted_find = json_output(&run(&[&common[..], &["find", "shared"]].concat(), None));
    assert_eq!(
        hosted_find["result"]["matches"][0]["roster_state"],
        "unassigned"
    );
    assert_find_paths_are_readable(&hosted_find);
    let expected_library_entry = fs::canonicalize(library_skill.join("SKILL.md")).unwrap();
    assert!(
        hosted_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| fs::canonicalize(path.as_str().unwrap())
                .is_ok_and(|actual| actual == expected_library_entry))
    );
    let hosted_snapshot = hosted_rescan["result"]["snapshot_id"].as_str().unwrap();
    let hosted_payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [hosted_snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let hosted_payload: Value = serde_json::from_str(&hosted_payload).unwrap();
    let hosted_placements = hosted_payload["placements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|placement| placement["skill_id"] == skill_id)
        .collect::<Vec<_>>();
    assert_eq!(hosted_placements.len(), 3);
    assert!(hosted_placements.iter().any(|placement| {
        placement["directory"]
            .as_str()
            .and_then(|path| fs::canonicalize(path).ok())
            .is_some_and(|path| path == fs::canonicalize(&library_skill).unwrap())
    }));
    assert!(
        hosted_payload["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|warning| !warning.as_str().unwrap().contains("escapes approved roots"))
    );
    json_output(&run(
        &[
            &common[..],
            &["undo", hosted["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert!(!state.join("library").exists());
    assert!(!state.join("plan-backups").exists());
    assert!(codex_skill.join("SKILL.md").is_file());
    assert!(claude_skill.join("SKILL.md").is_file());
    let restored_scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let restored_find = json_output(&run(&[&common[..], &["find", "shared"]].concat(), None));
    assert_find_paths_are_readable(&restored_find);
    assert!(
        restored_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| !path.as_str().unwrap().contains("/library/"))
    );

    let restored_snapshot = restored_scan["result"]["snapshot_id"].as_str().unwrap();
    let restored_evidence_id: String = database
        .query_row(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [restored_snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let mut drift_request = request("managed");
    drift_request["scan_id"] = json!(restored_snapshot);
    drift_request["evidence_ids"] = json!([restored_evidence_id]);
    let drift_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&drift_request.to_string()),
    ));
    fs::write(claude_skill.join("SKILL.md"), "user drift\n").unwrap();
    let drifted = run(
        &[
            &common[..],
            &["apply", drift_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    );
    assert!(!drifted.status.success());
    assert!(
        !fs::symlink_metadata(&claude_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn hosted_plan_refuses_a_library_nested_under_an_agent_skill_root() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let codex_root = home.join(".codex/skills");
    let state = codex_root.join(".skillroster-state");
    let codex_skill = codex_root.join("shared");
    let claude_skill = home.join(".claude/skills/shared");
    let content = "---\nname: shared\ndescription: shared fixture\n---\nbody\n";
    for directory in [&codex_skill, &claude_skill] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    let mut common_owned = vec![
        "--home".to_owned(),
        home.display().to_string(),
        "--state-dir".to_owned(),
        state.display().to_string(),
        "--json".to_owned(),
    ];
    for index in 0..12 {
        common_owned.push("--root".to_owned());
        common_owned.push(format!(
            "cursor={}",
            state
                .join("library")
                .join(format!("root-{index:02}"))
                .display()
        ));
    }
    let common = common_owned.iter().map(String::as_str).collect::<Vec<_>>();
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let placements = payload["placements"].as_array().unwrap();
    let canonical = placements
        .iter()
        .find(|placement| placement["agent"] == "codex")
        .unwrap();
    let evidence_id: String = database
        .query_row(
            "SELECT id FROM evidence WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "library_changes": [{
            "skill_id": payload["skills"][0]["id"],
            "canonical_placement_id": canonical["id"],
            "placement_ids": placements
                .iter()
                .map(|placement| placement["id"].clone())
                .collect::<Vec<_>>(),
            "requested_state": "hosted"
        }]
    });

    let rejected = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!rejected.status.success());
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(
        rejected["error"]["code"],
        "library_root_conflicts_with_agent_root"
    );
    assert_eq!(
        rejected["error"]["details"]["reason"],
        "library_root_overlaps_agent_skill_root"
    );
    assert_eq!(rejected["error"]["details"]["files_changed"], false);
    assert_eq!(rejected["error"]["details"]["agent_root_count"], 13);
    assert_eq!(
        rejected["error"]["details"]["agent_roots"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(rejected["error"]["details"]["agent_roots_truncated"], true);
    assert_eq!(
        rejected["error"]["details"]["next_action"],
        "choose_state_dir_outside_agent_skill_roots"
    );
    assert!(
        rejected["error"]["details"]["agent_roots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|root| fs::canonicalize(root.as_str().unwrap()).ok()
                == fs::canonicalize(&codex_root).ok())
    );
    let plan_count: i64 = database
        .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get(0))
        .unwrap();
    assert_eq!(plan_count, 0);
    assert!(!state.join("library").exists());
}

#[test]
fn multi_skill_library_plan_preserves_totals_beyond_the_item_preview() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    for index in 0..12 {
        let name = format!("shared-{index:02}");
        let content =
            format!("---\nname: {name}\ndescription: shared fixture {index}\n---\nbody {index}\n");
        for root in [home.join(".codex/skills"), home.join(".claude/skills")] {
            let directory = root.join(&name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("SKILL.md"), &content).unwrap();
        }
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let mut placements_by_skill = std::collections::BTreeMap::<String, Vec<&Value>>::new();
    for placement in payload["placements"].as_array().unwrap() {
        placements_by_skill
            .entry(placement["skill_id"].as_str().unwrap().to_owned())
            .or_default()
            .push(placement);
    }
    assert_eq!(placements_by_skill.len(), 12);
    assert!(
        placements_by_skill
            .values()
            .all(|placements| placements.len() == 2)
    );
    let evidence_ids = payload["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill_evidence_id(&database, snapshot, skill["id"].as_str().unwrap()))
        .collect::<Vec<_>>();
    let library_changes = placements_by_skill
        .into_iter()
        .map(|(skill_id, placements)| {
            let canonical = placements
                .iter()
                .find(|placement| placement["agent"] == "codex")
                .unwrap();
            json!({
                "skill_id": skill_id,
                "canonical_placement_id": canonical["id"],
                "placement_ids": placements
                    .iter()
                    .map(|placement| placement["id"].clone())
                    .collect::<Vec<_>>(),
                "requested_state": "managed"
            })
        })
        .collect::<Vec<_>>();
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": evidence_ids,
        "library_changes": library_changes
    });

    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    let impact = &plan["result"]["impact"];
    assert_eq!(impact["item_count"], 12);
    assert_eq!(impact["items"].as_array().unwrap().len(), 10);
    assert_eq!(impact["items_truncated"], true);
    assert_eq!(impact["totals"]["before"]["physical_source_count"], 24);
    assert_eq!(impact["totals"]["after"]["physical_source_count"], 12);
    assert_eq!(impact["totals"]["delta"]["physical_source_count"], -12);
    assert_eq!(impact["totals"]["before"]["placement_count"], 24);
    assert_eq!(impact["totals"]["after"]["placement_count"], 24);
    assert_eq!(impact["totals"]["delta"]["placement_count"], 0);
    assert_eq!(
        impact["totals"]["delta"]["default_exposed_placement_count"],
        0
    );
    assert_eq!(impact["totals"]["relinked_placement_count"], 12);

    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let full = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    assert_eq!(full["result"]["impact"], plan["result"]["impact"]);
    assert_eq!(full["result"]["operations"].as_array().unwrap().len(), 25);
    assert_eq!(plan["result"]["files_changed"], false);
}

#[test]
#[cfg(unix)]
fn exact_duplicate_plan_deduplicates_shared_agent_roots_and_undo_restores_sources() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let shared_root = home.join(".agents_skills");
    let shared_skill = shared_root.join("shared");
    let opencode_skill = home.join(".config/opencode/skills/shared");
    let hermes_skill = home.join(".hermes/skills/shared");
    let content = "---\nname: shared\ndescription: shared fixture\n---\nbody\n";
    for directory in [&shared_skill, &opencode_skill, &hermes_skill] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    for (parent, logical_root) in [
        (home.join(".codex"), home.join(".codex/skills")),
        (home.join(".claude"), home.join(".claude/skills")),
        (home.join(".pi/agent"), home.join(".pi/agent/skills")),
    ] {
        fs::create_dir_all(parent).unwrap();
        symlink(&shared_root, logical_root).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Exact duplicate Skill placements")
        .unwrap();
    assert!(
        finding["summary"]
            .as_str()
            .unwrap()
            .contains("6 placements across 3 distinct physical sources")
    );
    let finding_id = finding["id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["canonical_candidate_count"], 3);
    assert_eq!(planning["canonical_candidates_truncated"], false);
    assert_eq!(
        planning["canonical_candidates"][0]["path"],
        fs::canonicalize(shared_skill.join("SKILL.md"))
            .unwrap()
            .to_str()
            .unwrap()
    );
    let canonical_placement_id = planning["canonical_candidates"][0]["placement_id"]
        .as_str()
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": finding_id,
            "canonical_placement_id": canonical_placement_id,
            "requested_state": "managed"
        }]
    });

    let codex_root = home.join(".codex/skills");
    let drifted_root = home.join("drifted-shared-root");
    fs::create_dir_all(drifted_root.join("shared")).unwrap();
    fs::write(drifted_root.join("shared/SKILL.md"), content).unwrap();
    fs::remove_file(&codex_root).unwrap();
    symlink(&drifted_root, &codex_root).unwrap();
    let drifted_output = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!drifted_output.status.success());
    let drifted: Value = serde_json::from_slice(&drifted_output.stdout).unwrap();
    assert_eq!(drifted["ok"], false);
    assert_eq!(drifted["error"]["code"], "state_drift");
    fs::remove_file(&codex_root).unwrap();
    symlink(&shared_root, &codex_root).unwrap();

    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert_eq!(plan["result"]["change_summary"]["operation_count"], 5);
    assert_eq!(plan["result"]["affected"]["placement_count"], 6);
    let impact = &plan["result"]["impact"]["items"][0];
    assert_eq!(impact["before"]["physical_source_count"], 3);
    assert_eq!(impact["after"]["physical_source_count"], 1);
    assert_eq!(impact["delta"]["physical_source_count"], -2);
    assert_eq!(impact["before"]["placement_count"], 6);
    assert_eq!(impact["after"]["placement_count"], 6);
    assert_eq!(impact["delta"]["placement_count"], 0);
    assert_eq!(impact["before"]["default_exposed_placement_count"], 5);
    assert_eq!(impact["after"]["default_exposed_placement_count"], 5);
    assert_eq!(impact["delta"]["default_exposed_placement_count"], 0);
    assert_eq!(impact["after"]["relinked_placement_count"], 2);
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let full = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    assert_eq!(full["result"]["impact"], plan["result"]["impact"]);
    let targets = full["result"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| operation["target"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(targets.len(), 5);

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    for directory in [&opencode_skill, &hermes_skill] {
        assert!(
            fs::symlink_metadata(directory)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::canonicalize(directory).unwrap(),
            fs::canonicalize(&shared_skill).unwrap()
        );
    }
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    for directory in [&opencode_skill, &hermes_skill] {
        assert!(directory.join("SKILL.md").is_file());
        assert!(
            !fs::symlink_metadata(directory)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(directory.join("SKILL.md")).unwrap(),
            content
        );
    }
    assert_eq!(
        fs::read_to_string(shared_skill.join("SKILL.md")).unwrap(),
        content
    );
}

#[test]
#[cfg(unix)]
fn exact_duplicate_plan_moves_the_real_source_instead_of_its_alias() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_skill = home.join(".codex/skills/shared");
    let opencode_real = home.join(".config/opencode/skills/z-real");
    let opencode_alias = home.join(".config/opencode/skills/a-alias");
    let hermes_skill = home.join(".hermes/skills/shared");
    let content = "---\nname: shared\ndescription: shared fixture\n---\nbody\n";
    for directory in [&codex_skill, &opencode_real, &hermes_skill] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    symlink(&opencode_real, &opencode_alias).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Exact duplicate Skill placements")
        .unwrap();
    let finding_id = finding["id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["canonical_candidate_count"], 3);
    assert!(
        planning["canonical_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| candidate["path"] != opencode_alias.join("SKILL.md").to_str().unwrap())
    );
    let canonical_placement_id = planning["canonical_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["path"] == codex_skill.join("SKILL.md").to_str().unwrap())
        .unwrap()["placement_id"]
        .as_str()
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": finding_id,
            "canonical_placement_id": canonical_placement_id,
            "requested_state": "managed"
        }]
    });

    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert_eq!(plan["result"]["change_summary"]["operation_count"], 5);
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let full = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    let moved_sources = full["result"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|operation| operation["kind"] == "move_recoverable")
        .map(|operation| operation["source"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        moved_sources
            .iter()
            .any(|source| Path::new(source).file_name().unwrap() == "z-real")
    );
    assert!(
        moved_sources
            .iter()
            .all(|source| Path::new(source).file_name().unwrap() != "a-alias")
    );

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert!(
        fs::symlink_metadata(&opencode_real)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&opencode_alias).unwrap(),
        fs::canonicalize(&codex_skill).unwrap()
    );
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    assert!(opencode_real.join("SKILL.md").is_file());
    assert_eq!(
        fs::read_to_string(opencode_alias.join("SKILL.md")).unwrap(),
        content
    );
}

#[test]
fn exact_duplicate_finding_prepares_library_plan_from_semantic_choices() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let source_root = temp.path().join("sources");
    let source_skill = source_root.join("shared");
    let codex_skill = home.join(".codex/skills/shared");
    let claude_skill = home.join(".claude/skills/shared");
    let content = "---\nname: shared\ndescription: shared fixture\n---\nbody\n";
    for directory in [&source_skill, &codex_skill, &claude_skill] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    for index in 0..48 {
        let directory = home.join(format!(".codex/skills/shared-copy-{index:02}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--source-root",
        source_root.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Exact duplicate Skill placements")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], true);
    assert!(
        detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["action"] == "plan")
    );
    assert_eq!(planning["snapshot_id"], report["result"]["snapshot_id"]);
    assert_eq!(planning["request_field"], "finding_library_changes");
    let candidates = planning["canonical_candidates"].as_array().unwrap();
    assert_eq!(planning["canonical_candidate_count"], 51);
    assert_eq!(planning["canonical_candidates_truncated"], true);
    assert_eq!(
        candidates[0]["path"],
        fs::canonicalize(source_skill.join("SKILL.md"))
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(candidates[0]["reason"], "non_exposed_source");
    let canonical_placement_id = candidates[0]["placement_id"].as_str().unwrap();

    let request = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": finding_id,
            "canonical_placement_id": canonical_placement_id,
            "requested_state": "managed"
        }]
    });
    let plan_output = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(
        plan_output.stdout.len() < 8_000,
        "summary response was {} bytes",
        plan_output.stdout.len()
    );
    let plan = json_output(&plan_output);
    assert_eq!(plan["result"]["risk"], "library_governance");
    assert_eq!(plan["result"]["findings"]["ids"], json!([finding_id]));
    assert_eq!(plan["result"]["detail_level"], "summary");
    assert!(plan["result"].get("operations").is_none());
    assert!(plan["result"].get("library_changes").is_none());
    assert_eq!(plan["result"]["change_summary"]["operation_count"], 101);
    assert_eq!(plan["result"]["affected"]["placement_count"], 51);
    assert_eq!(plan["result"]["affected"]["agent_count"], 2);
    assert_eq!(
        plan["result"]["affected"]["agents"],
        json!(["claude-code", "codex"])
    );
    assert_eq!(plan["result"]["evidence"]["count"], 1);
    assert_eq!(plan["result"]["files_changed"], false);
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    assert_eq!(
        plan["result"]["detail"]["command"],
        json!(["plan", "--show", plan_id, "--json"])
    );
    assert!(
        !plan["result"]["detail"]["contains"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_core_selections"))
    );
    let full_output = run(&[&common[..], &["plan", "--show", plan_id]].concat(), None);
    assert!(full_output.stdout.len() > plan_output.stdout.len());
    let full = json_output(&full_output);
    assert_eq!(full["result"]["detail_level"], "full");
    assert_eq!(full["result"]["operations"].as_array().unwrap().len(), 101);
    assert_eq!(
        full["result"]["library_changes"][0]["placement_ids"]
            .as_array()
            .unwrap()
            .len(),
        51
    );
    assert_eq!(full["result"]["finding_ids"], json!([finding_id]));
    assert_eq!(full["result"]["files_changed"], false);
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let stored_report_id: String = database
        .query_row(
            "SELECT report_id FROM plans WHERE id = ?1",
            [plan_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_report_id, report["result"]["report_id"]);
    assert!(
        !fs::symlink_metadata(&codex_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        !fs::symlink_metadata(&claude_skill)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    let invalid_canonical = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": finding_id,
            "canonical_placement_id": "placement_not_in_finding",
            "requested_state": "managed"
        }]
    });
    let rejected = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&invalid_canonical.to_string()),
    );
    assert!(!rejected.status.success());
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("canonical placement is not part of Finding")
    );

    let non_overlap_finding = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["category"] != "overlap")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let wrong_finding_type = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": non_overlap_finding,
            "canonical_placement_id": canonical_placement_id,
            "requested_state": "managed"
        }]
    });
    let rejected = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&wrong_finding_type.to_string()),
    );
    assert!(!rejected.status.success());
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is not an exact-duplicate Finding")
    );

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let stale = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!stale.status.success());
    let stale: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert!(
        stale["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not belong to the latest Snapshot")
    );
    let stale_detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(stale_detail["result"]["planning"]["supported"], false);
    assert_eq!(
        stale_detail["result"]["planning"]["reason"],
        "stale_finding"
    );
    assert!(
        stale_detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );
}

#[test]
#[cfg(unix)]
fn large_roster_finding_blocks_partial_plan_until_source_is_confirmed() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let outside = temp.path().join("unconfirmed-source");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(
        outside.join("SKILL.md"),
        "---\nname: external\n---\nfixture\n",
    )
    .unwrap();
    let outside = fs::canonicalize(outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("zzz-external")).unwrap();
    for index in 0..51 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(detail["result"]["kind"], "large_default_roster");
    assert_eq!(detail["result"]["files_changed"], false);
    assert_eq!(detail["result"]["planning"]["supported"], false);
    assert_eq!(
        detail["result"]["planning"]["reason"],
        "trusted_canonical_sources_required"
    );
    assert_eq!(detail["result"]["planning"]["blocked_change_count"], 1);
    assert_eq!(
        detail["result"]["planning"]["observed_link_targets"],
        json!([outside])
    );
    let prerequisite = &detail["result"]["planning"]["source_confirmation_finding"];
    assert_eq!(prerequisite["state"], "available");
    assert_eq!(prerequisite["kind"], "escaping_link_source_confirmation");
    assert_eq!(prerequisite["snapshot_id"], report["result"]["snapshot_id"]);
    let source_finding_id = prerequisite["finding_id"].as_str().unwrap();
    assert_ne!(source_finding_id, finding_id);
    let source_action = detail["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "view_source_confirmation_finding")
        .expect("blocked Roster exposes its source Finding continuation");
    let source_argv = source_action["argv"].as_array().unwrap();
    assert_eq!(source_argv[0], env!("CARGO_BIN_EXE_skillroster"));
    assert_eq!(source_argv[1], "--state-dir");
    assert_eq!(source_argv[2], state.to_str().unwrap());
    assert_eq!(source_argv[3], "--home");
    assert_eq!(source_argv[4], home.to_str().unwrap());
    assert_eq!(
        &source_argv[5..],
        &json!(["report", "--finding", source_finding_id, "--json"])
            .as_array()
            .unwrap()[..]
    );
    assert_eq!(source_action["mutates"], false);
    assert_eq!(source_action["requires_confirmation"], false);
    let source_detail = json_output(&run_suggested_action(source_action));
    assert_eq!(
        source_detail["result"]["kind"],
        "escaping_link_source_confirmation"
    );
    let confirm_actions = source_detail["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["action"] == "confirm_source_root_read_permission")
        .collect::<Vec<_>>();
    assert!(!confirm_actions.is_empty());
    assert!(
        confirm_actions
            .iter()
            .all(|action| { action["mutates"] == true && action["requires_confirmation"] == true })
    );
    assert!(
        detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );

    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let blocked = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert!(
        blocked["error"]["message"]
            .as_str()
            .unwrap()
            .contains("confirm the reported source roots")
    );

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let report_id = prerequisite["report_id"].as_str().unwrap();
    let duplicate_finding_id = "finding_197_cli_duplicate";
    database
        .execute(
            "INSERT INTO findings (id, report_id, category, severity, title, summary, details_json) SELECT ?1, report_id, category, severity, title, summary, details_json FROM findings WHERE id = ?2",
            rusqlite::params![duplicate_finding_id, source_finding_id],
        )
        .unwrap();
    let raw_summary: String = database
        .query_row(
            "SELECT summary_json FROM reports WHERE id = ?1",
            [report_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut summary: Value = serde_json::from_str(&raw_summary).unwrap();
    let mut duplicate = summary["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == source_finding_id)
        .unwrap()
        .clone();
    duplicate["id"] = json!(duplicate_finding_id);
    summary["findings"].as_array_mut().unwrap().push(duplicate);
    database
        .execute(
            "UPDATE reports SET summary_json = ?1 WHERE id = ?2",
            rusqlite::params![summary.to_string(), report_id],
        )
        .unwrap();

    let ambiguous = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(
        ambiguous["result"]["planning"]["source_confirmation_finding"]["reason_code"],
        "matching_escaping_link_finding_ambiguous"
    );
    assert_eq!(
        ambiguous["result"]["planning"]["source_confirmation_finding"]["candidate_count"],
        2
    );
    assert!(
        ambiguous["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "view_source_confirmation_finding")
    );
}

#[test]
#[cfg(unix)]
fn custom_budget_plan_reports_actionable_source_blockers() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let reviewed = temp.path().join("reviewed");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&reviewed).unwrap();
    let reviewed = fs::canonicalize(reviewed).unwrap();
    for index in 0..10 {
        let directory = root.join(format!("aaa-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: aaa-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    for name in ["mid-a", "mid-b"] {
        let canonical = reviewed.join(name);
        fs::create_dir(&canonical).unwrap();
        fs::write(
            canonical.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture\n"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&canonical, root.join(name)).unwrap();
    }
    for index in 0..49 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", &finding_id]].concat(),
        None,
    ));
    assert_eq!(detail["result"]["planning"]["supported"], true);

    let default_request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let default_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&default_request.to_string()),
    ));
    assert_eq!(default_plan["ok"], true);
    assert_eq!(default_plan["result"]["files_changed"], false);

    let custom_request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 10,
            "protected_skill_ids": []
        }]
    });
    let blocked = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&custom_request.to_string()),
    );
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(blocked["ok"], false);
    assert_eq!(
        blocked["error"]["code"],
        "trusted_canonical_sources_required"
    );
    assert_eq!(blocked["error"]["details"]["requested_core_budget"], 10);
    assert_eq!(blocked["error"]["details"]["blocked_change_count"], 2);
    assert_eq!(
        blocked["error"]["details"]["blocked_changes_truncated"],
        false
    );
    let exact_sources = [reviewed.join("mid-a"), reviewed.join("mid-b")];
    assert_eq!(
        blocked["error"]["details"]["source_roots"],
        json!(exact_sources)
    );
    assert_eq!(blocked["error"]["paths"], json!(exact_sources));
    let names = blocked["error"]["details"]["blocked_changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["mid-a", "mid-b"]);
    assert!(
        blocked["error"]["details"]["blocked_changes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| {
                item["agent"] == "codex"
                    && item["reason"] == "untrusted_external_placement_blocks_mutation"
                    && item["owned_by_agent"] == true
                    && item["mutation_scope"] == "untrusted_external"
                    && item["mutation_scopes"] == json!(["untrusted_external"])
                    && item["observed_source_target"]
                        .as_str()
                        .is_some_and(|path| path.starts_with(reviewed.to_str().unwrap()))
            })
    );
    assert_eq!(
        blocked["error"]["details"]["after_confirmation"]["repeatable_option"],
        "--source-root"
    );
    assert_eq!(blocked["error"]["details"]["files_changed"], false);
    assert_eq!(blocked["error"]["details"]["state_files_changed"], false);
    assert_eq!(
        blocked["error"]["details"]["detail_artifact_created"],
        false
    );
    assert_eq!(
        blocked["suggested_actions"][0]["action"],
        "scan_with_confirmed_source_roots"
    );
    assert_eq!(
        blocked["suggested_actions"][0]["requires_confirmation"],
        true
    );
    assert_eq!(blocked["suggested_actions"][0]["mutates"], false);
    let suggested_argv = blocked["suggested_actions"][0]["argv"].as_array().unwrap();
    assert_eq!(
        &suggested_argv[..5],
        &json!([
            env!("CARGO_BIN_EXE_skillroster"),
            "--state-dir",
            state,
            "--home",
            home
        ])
        .as_array()
        .unwrap()[..]
    );
    assert_eq!(
        &blocked["error"]["details"]["after_confirmation"]["argv_template"]
            .as_array()
            .unwrap()[..5],
        &suggested_argv[..5]
    );
    for exact_source in &exact_sources {
        assert!(suggested_argv.windows(2).any(|pair| {
            pair[0] == "--source-root" && pair[1] == exact_source.to_str().unwrap()
        }));
    }
    assert!(
        !suggested_argv
            .windows(2)
            .any(|pair| { pair[0] == "--source-root" && pair[1] == reviewed.to_str().unwrap() })
    );
    let suggested_roots = suggested_argv
        .windows(2)
        .filter(|pair| pair[0] == "--source-root")
        .map(|pair| {
            pair[1]
                .as_str()
                .expect("source-root argument must be a string")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        suggested_roots,
        exact_sources
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect::<Vec<_>>()
    );
    let retry_source_args = suggested_roots
        .iter()
        .flat_map(|root| ["--source-root", *root])
        .collect::<Vec<_>>();
    assert!(root.join("mid-a").exists());
    assert!(root.join("aaa-000").exists());

    for width in ["60", "80", "120"] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_skillroster"));
        command
            .args([
                "--home",
                home.to_str().unwrap(),
                "--state-dir",
                state.to_str().unwrap(),
                "plan",
                "--stdin",
            ])
            .env("COLUMNS", width)
            .env("NO_COLOR", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(custom_request.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let human = String::from_utf8(output.stderr).unwrap();
        assert!(human.contains("mid-a"), "{human}");
        assert!(human.contains("--source-root"), "{human}");
        assert!(
            human.contains("no automatic change is supported"),
            "{human}"
        );
        let max_width: usize = width.parse().unwrap();
        assert!(
            human.lines().all(|line| line.chars().count() <= max_width),
            "line exceeded {width} columns:\n{human}"
        );
    }

    json_output(&run(
        &[&common[..], &retry_source_args, &["scan"]].concat(),
        None,
    ));
    let report = json_output(&run(
        &[&common[..], &retry_source_args, &["report"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let retry = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 10,
            "protected_skill_ids": []
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &retry_source_args, &["plan", "--stdin"]].concat(),
        Some(&retry.to_string()),
    ));
    assert_eq!(plan["ok"], true);
    assert_eq!(plan["result"]["files_changed"], false);
    assert!(plan["result"]["plan_id"].as_str().is_some());
    assert_eq!(plan["result"]["impact"]["after_default_exposure"], 10);
    assert!(root.join("mid-a").exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 61);
}

#[test]
#[cfg(unix)]
fn full_roster_finding_can_protect_read_only_skills_and_prepare_a_plan() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let sources = temp.path().join("sources");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&sources).unwrap();
    for index in 0..51 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let shared = root.join("zzz-shared");
    fs::create_dir(&shared).unwrap();
    fs::write(
        shared.join("SKILL.md"),
        "---\nname: zzz-shared\n---\nfixture\n",
    )
    .unwrap();
    for agent_root in [home.join(".claude/skills"), home.join(".pi/agent/skills")] {
        fs::create_dir_all(&agent_root).unwrap();
        symlink(&shared, agent_root.join("zzz-shared")).unwrap();
    }
    let source_paths = (0..6)
        .map(|index| {
            let name = format!("zzz-read-only-{index}");
            let source = sources.join(&name);
            fs::create_dir(&source).unwrap();
            fs::write(
                source.join("SKILL.md"),
                format!("---\nname: {name}\n---\nfixture\n"),
            )
            .unwrap();
            let source = fs::canonicalize(source).unwrap();
            symlink(&source, root.join(&name)).unwrap();
            source
        })
        .collect::<Vec<_>>();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let finding_id = |report: &Value, kind: &str| {
        report["result"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["kind"] == kind)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report", "--full"]].concat(), None));
    let source_finding_id = finding_id(&report, "escaping_link_source_confirmation");
    for source in &source_paths {
        json_output(&run(
            &[
                &common[..],
                &[
                    "source-root",
                    "confirm",
                    "--finding",
                    &source_finding_id,
                    "--path",
                    source.to_str().unwrap(),
                ],
            ]
            .concat(),
            None,
        ));
    }

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report", "--full"]].concat(), None));
    let roster_finding_id = finding_id(&report, "large_default_roster");
    let compact = json_output(&run(
        &[&common[..], &["report", "--finding", &roster_finding_id]].concat(),
        None,
    ));
    let compact_choice = &compact["result"]["planning"]["resolution_choices"][0];
    assert_eq!(compact_choice["choice"], "protect_blocked_skills_as_core");
    assert_eq!(compact_choice["available"], false);
    assert_eq!(
        compact_choice["unavailable_reason"],
        "blocked_skill_set_incomplete"
    );
    assert_eq!(compact_choice["protected_skill_ids_complete"], false);
    assert_eq!(compact_choice["plan_request_template_available"], false);
    assert!(compact_choice.get("plan_request_template").is_none());

    let full = json_output(&run(
        &[
            &common[..],
            &["report", "--finding", &roster_finding_id, "--full"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(
        full["result"]["planning"]["decision"],
        "choose_mutable_placements_or_keep_unchanged"
    );
    assert_eq!(
        full["result"]["planning"]["decision_code"],
        "protect_read_only_skills_or_keep_unchanged"
    );
    let full_choice = &full["result"]["planning"]["resolution_choices"][0];
    assert_eq!(full_choice["choice"], "protect_blocked_skills_as_core");
    assert_eq!(full_choice["available"], true);
    assert_eq!(full_choice["protected_skill_ids_complete"], true);
    assert_eq!(
        full_choice["protected_skill_ids"].as_array().unwrap().len(),
        6
    );
    assert_eq!(full_choice["plan_request_template_available"], true);

    let request = full_choice["plan_request_template"].clone();
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert!(plan["result"]["plan_id"].is_string());
    assert_eq!(plan["result"]["files_changed"], false);
    assert!(
        plan["result"]["change_summary"]["roster_change_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    for source in source_paths {
        assert!(source.join("SKILL.md").is_file());
    }
}

#[test]
#[cfg(unix)]
fn large_roster_finding_reports_a_dependent_source_link_before_planning() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let source_root = temp.path().join("sources");
    fs::create_dir_all(&source_root).unwrap();
    let canonicals = (0..6)
        .map(|index| {
            let name = format!("zzz-canonical-{index}");
            let canonical = root.join(&name);
            fs::create_dir_all(&canonical).unwrap();
            fs::write(
                canonical.join("SKILL.md"),
                format!("---\nname: {name}\n---\nfixture\n"),
            )
            .unwrap();
            std::os::unix::fs::symlink(
                &canonical,
                source_root.join(format!("dependent-source-{index}")),
            )
            .unwrap();
            canonical
        })
        .collect::<Vec<_>>();
    for index in 0..51 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--source-root",
        source_root.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], false);
    assert_eq!(planning["reason"], "source_dependency_blocks_roster_change");
    assert_eq!(planning["decision"], "resolve_source_dependency");
    assert_eq!(planning["blocked_change_count"], 6);
    assert_eq!(
        planning["blocked_changes"][0]["reason"],
        "non_agent_source_link_depends_on_removal"
    );
    assert_eq!(planning["dependent_link_targets"], json!(canonicals));
    assert_eq!(planning["blocked_skill_count"], 6);
    assert_eq!(planning["blocked_skills_truncated"], true);
    assert_eq!(planning["blocked_skills"].as_array().unwrap().len(), 5);
    let blocked_skill = &planning["blocked_skills"][0];
    assert!(
        blocked_skill["name"]
            .as_str()
            .unwrap()
            .starts_with("zzz-canonical-")
    );
    assert_eq!(blocked_skill["agents"], json!(["codex"]));
    assert_eq!(
        blocked_skill["reasons"],
        json!(["non_agent_source_link_depends_on_removal"])
    );
    assert_eq!(
        blocked_skill["dependent_source_paths"],
        json!([fs::canonicalize(&source_root).unwrap().join(format!(
            "dependent-source-{}",
            blocked_skill["name"]
                .as_str()
                .unwrap()
                .strip_prefix("zzz-canonical-")
                .unwrap()
        ))])
    );
    assert_eq!(
        planning["resolution_choices"][0]["choice"],
        "protect_blocked_skills_as_core"
    );
    assert_eq!(
        planning["resolution_choices"][0]["requires_confirmation"],
        true
    );
    assert_eq!(
        planning["resolution_choices"][0]["plan_request_template_available"],
        false
    );
    assert_eq!(planning["resolution_choices"][0]["available"], false);
    assert_eq!(
        planning["resolution_choices"][0]["unavailable_reason"],
        "blocked_skill_set_incomplete"
    );
    assert!(
        planning["resolution_choices"][0]
            .get("plan_request_template")
            .is_none()
    );
    assert_eq!(
        planning["resolution_choices"][1]["choice"],
        "preserve_or_retarget_dependent_sources"
    );
    assert_eq!(
        planning["resolution_choices"][1]["dependent_source_paths"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    let full = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id, "--full"]].concat(),
        None,
    ));
    assert_eq!(full["result"]["planning"]["blocked_skill_count"], 6);
    assert_eq!(
        full["result"]["planning"]["blocked_skills"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    assert_eq!(
        full["result"]["planning"]["blocked_skills_truncated"],
        false
    );
    assert_eq!(
        full["result"]["planning"]["resolution_choices"][0]["plan_request_template_available"],
        true
    );
    assert_eq!(
        full["result"]["planning"]["resolution_choices"][0]["available"],
        true
    );
    assert_eq!(
        full["result"]["planning"]["resolution_choices"][0]["plan_request_template"]
            ["finding_roster_changes"][0]["protected_skill_ids"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    assert!(
        full["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );
    assert!(
        detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );
    assert!(
        detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "view_source_confirmation_finding")
    );

    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let blocked = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(blocked["error"]["code"], "roster_dependent_source_conflict");
    assert_eq!(
        blocked["error"]["details"]["reason"],
        "dependent_source_would_break"
    );
    let blocked_paths = blocked["error"]["details"]["paths"].as_array().unwrap();
    assert_eq!(blocked_paths.len(), 1);
    assert!(
        blocked_paths[0]
            .as_str()
            .unwrap()
            .starts_with(fs::canonicalize(&source_root).unwrap().to_str().unwrap())
    );
    assert_eq!(blocked["error"]["details"]["files_changed"], false);
}

#[test]
#[cfg(unix)]
fn large_roster_core_protection_choice_uses_production_forced_core_constraints() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let source_root = temp.path().join("sources");
    fs::create_dir_all(&source_root).unwrap();
    let bootstrap = root.join("skillroster");
    fs::create_dir_all(&bootstrap).unwrap();
    fs::write(
        bootstrap.join("SKILL.md"),
        "---\nname: skillroster\n---\nfixture\n",
    )
    .unwrap();
    for index in 0..100 {
        let name = format!("dependent-{index:03}");
        let canonical = root.join(&name);
        fs::create_dir_all(&canonical).unwrap();
        fs::write(
            canonical.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture\n"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&canonical, source_root.join(&name)).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--source-root",
        source_root.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let full = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id, "--full"]].concat(),
        None,
    ));
    let planning = &full["result"]["planning"];
    let protect = &planning["resolution_choices"][0];

    assert_eq!(planning["blocked_skill_count"], 51);
    assert_eq!(planning["blocked_skills"].as_array().unwrap().len(), 51);
    assert_eq!(planning["blocked_skills_truncated"], false);
    assert_eq!(protect["protected_skill_ids_complete"], true);
    assert_eq!(protect["available"], false);
    assert_eq!(protect["plan_request_template_available"], false);
    assert_eq!(
        protect["unavailable_reason"],
        "protected_core_selection_unavailable"
    );
    assert!(
        protect["unavailable_detail"]
            .as_str()
            .unwrap()
            .contains("protected, declared-Core, or bootstrap Skills")
    );
    assert!(protect.get("plan_request_template").is_none());
    assert!(
        planning["after_resolution"]["next"]
            .as_str()
            .unwrap()
            .contains("use user-approved source-link preservation")
    );
    assert!(
        full["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "plan")
    );
}

#[test]
fn incomplete_fingerprint_plan_paths_are_typed_and_zero_mutation() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = temp.path().join("skills");
    let content = "---\nname: bounded-copy\n---\nSame entrypoint.\n";
    for (name, bytes) in [
        ("copy-a", 16 * 1024 * 1024 + 1),
        ("copy-b", 16 * 1024 * 1024 + 2),
    ] {
        let directory = root.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
        fs::File::create(directory.join("asset.bin"))
            .unwrap()
            .set_len(bytes)
            .unwrap();
    }
    let explicit_root = format!("codex={}", root.display());
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--root",
        &explicit_root,
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    json_output(&run(&[&common[..], &["report"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let skill_id: String = database
        .query_row(
            "SELECT skill_id FROM placements WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let placement_ids = {
        let mut statement = database
            .prepare("SELECT id FROM placements WHERE scan_id = ?1 ORDER BY id")
            .unwrap();
        statement
            .query_map([snapshot], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let evidence_id: String = database
        .query_row(
            "SELECT fe.evidence_id FROM finding_evidence fe JOIN findings f ON f.id = fe.finding_id WHERE f.title = 'Some Skill package fingerprints are incomplete' ORDER BY fe.evidence_id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let raw_request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "library_changes": [{
            "skill_id": skill_id,
            "canonical_placement_id": placement_ids[0],
            "placement_ids": placement_ids,
            "requested_state": "managed"
        }]
    });

    let raw = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&raw_request.to_string()),
    );
    assert!(!raw.status.success());
    let raw: Value = serde_json::from_slice(&raw.stdout).unwrap();
    assert_eq!(raw["error"]["code"], "incomplete_package_fingerprint");
    assert_eq!(raw["error"]["details"]["stage"], "plan");
    assert_eq!(
        raw["error"]["details"]["next_action"],
        "resolve_fingerprint_incompleteness_then_scan"
    );
    assert_eq!(
        raw["error"]["details"]["remediation"]["required_before_rescan"],
        true
    );

    let report_id: String = database
        .query_row(
            "SELECT id FROM reports WHERE scan_id = ?1 ORDER BY rowid DESC LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let legacy_finding_id = "finding_legacy_incomplete";
    let details = json!({
        "affected_skill_ids": [skill_id],
        "affected_placement_ids": placement_ids,
        "coverage": {"basis": "skill_root_scan"}
    });
    database
        .execute(
            "INSERT INTO findings (id, report_id, category, severity, title, summary, details_json) VALUES (?1, ?2, 'overlap', 'warning', 'Exact duplicate Skill placements', 'legacy fixture', ?3)",
            rusqlite::params![legacy_finding_id, report_id, details.to_string()],
        )
        .unwrap();
    database
        .execute(
            "INSERT INTO finding_evidence (finding_id, evidence_id) VALUES (?1, ?2)",
            rusqlite::params![legacy_finding_id, evidence_id],
        )
        .unwrap();
    let finding_request = json!({
        "schema_version": 1,
        "finding_library_changes": [{
            "finding_id": legacy_finding_id,
            "canonical_placement_id": placement_ids[0],
            "requested_state": "managed"
        }]
    });
    let finding = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&finding_request.to_string()),
    );
    assert!(!finding.status.success());
    let finding: Value = serde_json::from_slice(&finding.stdout).unwrap();
    assert_eq!(finding["error"]["code"], "incomplete_package_fingerprint");
    assert_eq!(finding["error"]["details"]["stage"], "finding_plan");

    let plan_count: i64 = database
        .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get(0))
        .unwrap();
    assert_eq!(plan_count, 0);
    assert!(!state.join("library").exists());
    assert!(!state.join("plan-backups").exists());
    assert_eq!(
        fs::read_to_string(root.join("copy-a/SKILL.md")).unwrap(),
        content
    );
    assert_eq!(
        fs::read_to_string(root.join("copy-b/SKILL.md")).unwrap(),
        content
    );
}

#[test]
fn apply_rejects_a_legacy_ready_plan_without_mutation() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = temp.path().join("skills");
    let content = "---\nname: legacy-ready\n---\nComplete fixture.\n";
    for name in ["copy-a", "copy-b"] {
        let directory = root.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("SKILL.md"), content).unwrap();
    }
    let explicit_root = format!("codex={}", root.display());
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--root",
        &explicit_root,
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    json_output(&run(&[&common[..], &["report"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let skill_id: String = database
        .query_row(
            "SELECT skill_id FROM placements WHERE scan_id = ?1 ORDER BY id LIMIT 1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let placement_ids = {
        let mut statement = database
            .prepare("SELECT id FROM placements WHERE scan_id = ?1 ORDER BY id")
            .unwrap();
        statement
            .query_map([snapshot], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let evidence_id: String = database
        .query_row(
            "SELECT fe.evidence_id FROM finding_evidence fe JOIN findings f ON f.id = fe.finding_id WHERE f.title = 'Exact duplicate Skill placements' ORDER BY fe.evidence_id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "library_changes": [{
            "skill_id": skill_id,
            "canonical_placement_id": placement_ids[0],
            "placement_ids": placement_ids,
            "requested_state": "managed"
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();

    let encoded: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();

    for (payload, reason) in [
        (
            {
                let mut payload: Value = serde_json::from_str(&encoded).unwrap();
                payload["content_identity_algorithm"] = json!("sha256-content-v1");
                payload
            },
            "legacy_snapshot_requires_rescan",
        ),
        (
            {
                let mut payload: Value = serde_json::from_str(&encoded).unwrap();
                payload["identity_path_coverage"] = json!("incomplete");
                payload["non_unicode_identity_paths_skipped"] = json!(1);
                payload
            },
            "non_unicode_identity_coverage_incomplete",
        ),
    ] {
        database
            .execute(
                "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
                rusqlite::params![payload.to_string(), snapshot],
            )
            .unwrap();
        let rejected = run(&[&common[..], &["apply", plan_id]].concat(), None);
        assert!(!rejected.status.success());
        let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert_eq!(
            rejected["error"]["code"],
            "content_identity_rescan_required"
        );
        assert_eq!(rejected["error"]["details"]["reason"], reason);
    }

    database
        .execute(
            "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
            rusqlite::params![&encoded, snapshot],
        )
        .unwrap();
    let mut legacy: Value = serde_json::from_str(&encoded).unwrap();
    for placement in legacy["placements"].as_array_mut().unwrap() {
        placement
            .as_object_mut()
            .unwrap()
            .remove("fingerprint_completeness");
    }
    database
        .execute(
            "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
            rusqlite::params![legacy.to_string(), snapshot],
        )
        .unwrap();

    let rejected = run(&[&common[..], &["apply", plan_id]].concat(), None);
    assert!(!rejected.status.success());
    let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejected["error"]["code"], "incomplete_package_fingerprint");
    assert_eq!(rejected["error"]["details"]["stage"], "apply");
    assert_eq!(
        rejected["error"]["details"]["remediation"]["options"],
        json!(["scan_with_current_skillroster"])
    );
    let status: String = database
        .query_row("SELECT status FROM plans WHERE id = ?1", [plan_id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "ready");
    let receipt_count: i64 = database
        .query_row("SELECT COUNT(*) FROM receipts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(receipt_count, 0);
    assert!(!state.join("plan-backups").exists());
    assert_eq!(
        fs::read_to_string(root.join("copy-a/SKILL.md")).unwrap(),
        content
    );
    assert_eq!(
        fs::read_to_string(root.join("copy-b/SKILL.md")).unwrap(),
        content
    );
    assert!(
        !fs::symlink_metadata(root.join("copy-a"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        !fs::symlink_metadata(root.join("copy-b"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn large_roster_finding_prepares_and_reverses_a_semantic_layering_plan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    for index in 0..61 {
        let name = if index == 0 {
            "skillroster".to_owned()
        } else {
            format!("skill-{index:03}")
        };
        let directory = root.join(&name);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], true);
    assert_eq!(planning["request_field"], "finding_roster_changes");
    assert_eq!(
        planning["absence_of_usage_evidence"],
        "not_negative_evidence"
    );
    assert_eq!(planning["explicit_only_or_archive_decision_implied"], false);
    assert_eq!(planning["agents"][0]["before_default_exposure"], 61);
    assert_eq!(planning["agents"][0]["proposed_core_count"], 50);
    assert_eq!(planning["agents"][0]["proposed_on_demand_count"], 11);
    assert_eq!(
        planning["selection_evidence"]["stable_fallback_core_count"],
        49
    );
    assert_eq!(
        planning["uncertainty"]["code"],
        "fallback_dominated_core_selection"
    );
    assert_eq!(planning["uncertainty"]["review_required"], true);
    assert!(
        planning["agents"][0]["core_preview"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "skillroster" && item["reason"] == "skillroster_bootstrap")
    );

    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 10,
            "protected_skill_ids": []
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert_eq!(plan["result"]["findings"]["ids"], json!([finding_id]));
    assert_eq!(plan["result"]["change_summary"]["roster_change_count"], 61);
    assert_eq!(plan["result"]["affected"]["placement_count"], 61);
    assert_eq!(plan["result"]["impact"]["before_default_exposure"], 61);
    assert_eq!(plan["result"]["impact"]["after_default_exposure"], 10);
    assert_eq!(plan["result"]["canonical_deletion_count"], 0);
    assert_eq!(
        plan["result"]["selection_evidence"]["core_selection_count"],
        10
    );
    assert_eq!(plan["result"]["selection_evidence"]["forced_core_count"], 1);
    assert_eq!(
        plan["result"]["selection_evidence"]["positive_signal_core_count"],
        0
    );
    assert_eq!(
        plan["result"]["selection_evidence"]["stable_fallback_core_count"],
        9
    );
    assert_eq!(
        plan["result"]["selection_evidence"]["fallback_dominated"],
        true
    );
    assert_eq!(
        plan["result"]["selection_evidence"]["detail_level"],
        "summary"
    );
    let core_preview = plan["result"]["selection_evidence"]["agents"][0]["core_preview"]
        .as_array()
        .unwrap();
    assert_eq!(core_preview.len(), 5);
    assert!(core_preview.iter().all(|selection| {
        selection["skill_id"].is_string()
            && selection["name"].is_string()
            && selection["reason"].is_string()
    }));
    assert_eq!(
        plan["result"]["selection_evidence"]["agents"][0]["core_preview_truncated"],
        true
    );
    assert!(
        plan["result"]["selection_evidence"]["agents"][0]
            .get("core_selections")
            .is_none()
    );
    assert_eq!(
        plan["result"]["uncertainty"]["code"],
        "fallback_dominated_core_selection"
    );
    assert_eq!(plan["result"]["uncertainty"]["review_required"], true);

    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    assert_eq!(
        detail["result"]["selection_evidence"]["detail_level"],
        "full"
    );
    let core_selections = detail["result"]["selection_evidence"]["agents"][0]["core_selections"]
        .as_array()
        .unwrap();
    assert_eq!(core_selections.len(), 10);
    assert_eq!(&core_selections[..5], core_preview);
    assert!(core_selections.iter().all(|selection| {
        selection["skill_id"].is_string()
            && selection["name"].is_string()
            && selection["reason"].is_string()
    }));
    for field in [
        "core_selection_count",
        "forced_core_count",
        "positive_signal_core_count",
        "stable_fallback_core_count",
        "fallback_dominated",
        "fallback_dominated_agent_count",
        "reason_counts",
    ] {
        assert_eq!(
            detail["result"]["selection_evidence"][field],
            plan["result"]["selection_evidence"][field]
        );
    }
    assert_eq!(
        detail["result"]["uncertainty"],
        plan["result"]["uncertainty"]
    );
    assert_eq!(plan["result"]["files_changed"], false);
    assert_eq!(detail["result"]["files_changed"], false);
    assert!(
        plan["result"]["detail"]["contains"]
            .as_array()
            .unwrap()
            .contains(&json!("complete_core_selections"))
    );
    let human = run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "plan",
            "--show",
            plan_id,
        ],
        None,
    );
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("0 signals · 1 forced · 9 fallback"));
    assert!(human.contains("fallback-dominated Core selection"));

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 10);
    assert_eq!(fs::read_dir(state.join("library")).unwrap().count(), 51);

    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 61);
    assert!(!state.join("library").exists());
}

#[cfg(unix)]
#[test]
fn shared_physical_roster_plan_moves_once_and_blocks_conflicting_states() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let shared_root = home.join(".agents_skills");
    let shared_skill = shared_root.join("shared");
    fs::create_dir_all(&shared_skill).unwrap();
    fs::write(
        shared_skill.join("SKILL.md"),
        "---\nname: shared\n---\nfixture\n",
    )
    .unwrap();
    for (parent, logical_root) in [
        (home.join(".codex"), home.join(".codex/skills")),
        (home.join(".claude"), home.join(".claude/skills")),
        (home.join(".pi/agent"), home.join(".pi/agent/skills")),
    ] {
        fs::create_dir_all(parent).unwrap();
        symlink(&shared_root, logical_root).unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = payload["skills"][0]["id"].as_str().unwrap();
    let evidence_id = skill_evidence_id(&database, snapshot, skill_id);

    let conflict = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": [
            {"agent": "codex", "skill_id": skill_id, "state": "core"},
            {"agent": "claude-code", "skill_id": skill_id, "state": "on_demand"}
        ]
    });
    let blocked = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&conflict.to_string()),
    );
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(blocked["error"]["code"], "roster_physical_state_conflict");
    assert_eq!(blocked["error"]["retryable"], false);
    assert_eq!(
        blocked["error"]["details"]["reason"],
        "shared_physical_state_conflict"
    );
    assert_eq!(blocked["error"]["details"]["skill_id"], skill_id);
    assert_eq!(
        blocked["error"]["details"]["agents"],
        json!(["claude-code", "codex", "pi"])
    );
    assert_eq!(blocked["error"]["details"]["files_changed"], false);
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(shared_skill.join("SKILL.md").is_file());

    let roster_changes = ["codex", "claude-code", "pi"]
        .into_iter()
        .map(|agent| {
            json!({
                "agent": agent,
                "skill_id": skill_id,
                "state": "on_demand"
            })
        })
        .collect::<Vec<_>>();
    let demotion = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": roster_changes
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&demotion.to_string()),
    ));
    assert_eq!(plan["result"]["change_summary"]["roster_change_count"], 3);
    assert_eq!(plan["result"]["affected"]["placement_count"], 4);
    assert_eq!(plan["result"]["impact"]["before_default_exposure"], 3);
    assert_eq!(plan["result"]["impact"]["after_default_exposure"], 0);
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    assert_eq!(
        detail["result"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|operation| operation["kind"] == "move_recoverable")
            .count(),
        1
    );

    let alternate_root = home.join("alternate-skills");
    let alternate_skill = alternate_root.join("shared");
    fs::create_dir_all(&alternate_skill).unwrap();
    fs::write(
        alternate_skill.join("SKILL.md"),
        "---\nname: shared\n---\nalternate\n",
    )
    .unwrap();
    let codex_root = home.join(".codex/skills");
    fs::remove_file(&codex_root).unwrap();
    symlink(&alternate_root, &codex_root).unwrap();
    let drifted_apply = run(&[&common[..], &["apply", plan_id]].concat(), None);
    assert!(!drifted_apply.status.success());
    let drifted_apply: Value = serde_json::from_slice(&drifted_apply.stdout).unwrap();
    assert_eq!(drifted_apply["error"]["code"], "state_drift");
    assert_eq!(
        drifted_apply["error"]["details"]["reason"],
        "physical_source_drift"
    );
    assert_eq!(drifted_apply["error"]["details"]["files_changed"], false);
    assert!(shared_skill.join("SKILL.md").is_file());
    assert!(alternate_skill.join("SKILL.md").is_file());
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM receipts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    fs::remove_file(&codex_root).unwrap();
    symlink(&shared_root, &codex_root).unwrap();

    let applied = json_output(&run(&[&common[..], &["apply", plan_id]].concat(), None));
    assert_eq!(applied["result"]["verification"], "passed");
    assert!(!shared_skill.exists());
    let receipt_id = applied["result"]["receipt_id"].as_str().unwrap();
    let undone = json_output(&run(&[&common[..], &["undo", receipt_id]].concat(), None));
    assert_eq!(undone["result"]["verification"], "passed");
    assert!(shared_skill.join("SKILL.md").is_file());
    for logical_skill in [
        home.join(".codex/skills/shared"),
        home.join(".claude/skills/shared"),
        home.join(".pi/agent/skills/shared"),
    ] {
        assert!(logical_skill.join("SKILL.md").is_file());
    }
}

#[test]
fn large_roster_plan_uses_exact_cross_agent_usage_before_fallback() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_root = home.join(".codex/skills");
    fs::create_dir_all(&codex_root).unwrap();
    for index in 0..51 {
        let name = if index == 0 {
            "alpha".to_owned()
        } else if index == 50 {
            "zeta".to_owned()
        } else {
            format!("skill-{index:03}")
        };
        let directory = codex_root.join(&name);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture {index}\n"),
        )
        .unwrap();
    }

    let cursor_skill = home.join(".cursor/skills/zeta");
    fs::create_dir_all(&cursor_skill).unwrap();
    fs::copy(
        codex_root.join("zeta/SKILL.md"),
        cursor_skill.join("SKILL.md"),
    )
    .unwrap();
    let cursor_sessions = home.join(".cursor/projects");
    fs::create_dir_all(&cursor_sessions).unwrap();
    fs::write(
        cursor_sessions.join("session.jsonl"),
        json!({
            "role": "assistant",
            "message": {"role": "assistant", "content": [{
                "type": "tool_use",
                "name": "Read",
                "input": {"path": cursor_skill.join("SKILL.md")}
            }]}
        })
        .to_string(),
    )
    .unwrap();

    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 1,
            "protected_skill_ids": []
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));

    let evidence = &plan["result"]["selection_evidence"];
    assert_eq!(evidence["positive_signal_core_count"], 1);
    assert_eq!(evidence["direct_signal_core_count"], 0);
    assert_eq!(evidence["cross_agent_signal_core_count"], 1);
    assert_eq!(evidence["stable_fallback_core_count"], 0);
    assert_eq!(evidence["reason_counts"]["cross_agent_observed_loaded"], 1);
    let selected = &evidence["agents"][0]["core_preview"][0];
    assert_eq!(selected["name"], "zeta");
    assert_eq!(selected["evidence_scope"], "cross_agent");
    assert_eq!(selected["evidence_agents"], json!(["cursor"]));
    assert_eq!(
        plan["result"]["uncertainty"]["code"],
        "cross_agent_dominated_core_selection"
    );
    assert_eq!(plan["result"]["uncertainty"]["review_required"], true);
    assert_eq!(plan["result"]["files_changed"], false);
}

#[test]
fn legacy_large_roster_snapshot_requests_a_typed_physical_identity_rescan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    for index in 0..51 {
        let skill = root.join(format!("skill-{index:03}"));
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["kind"] == "large_default_roster")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    {
        let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
        let encoded: String = database
            .query_row(
                "SELECT payload_json FROM scan_payloads ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut payload: Value = serde_json::from_str(&encoded).unwrap();
        payload
            .as_object_mut()
            .unwrap()
            .remove("observed_physical_mutation_paths");
        database
            .execute(
                "UPDATE scan_payloads SET payload_json = ?1",
                rusqlite::params![payload.to_string()],
            )
            .unwrap();
    }

    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", &finding_id, "--full"]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], false);
    assert_eq!(
        planning["reason"],
        "physical_mutation_identity_rescan_required"
    );
    assert_eq!(
        planning["reason_code"],
        "snapshot_missing_physical_mutation_identity"
    );
    assert_eq!(planning["next_action"], "scan");
    assert_eq!(planning["action"]["action"], "scan");
    assert_eq!(
        planning["action"]["reason_code"],
        "snapshot_missing_physical_mutation_identity"
    );
    assert_eq!(
        planning["action"]["argv"],
        json!([
            env!("CARGO_BIN_EXE_skillroster"),
            "--state-dir",
            state.to_str().unwrap(),
            "--home",
            home.to_str().unwrap(),
            "scan",
            "--summary",
            "--json"
        ])
    );

    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let blocked = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    );
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(
        blocked["error"]["code"],
        "roster_physical_identity_rescan_required"
    );
    assert_eq!(blocked["error"]["details"]["next_action"], "scan");
    assert_eq!(blocked["suggested_actions"][0]["action"], "scan");
    assert_eq!(
        blocked["suggested_actions"][0]["reason_code"],
        "snapshot_missing_physical_mutation_identity"
    );
    assert_eq!(
        blocked["suggested_actions"][0]["argv"],
        planning["action"]["argv"]
    );

    let replay_argv = planning["action"]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    let replay = json_output(&run(&replay_argv, None));
    assert_eq!(replay["ok"], true);
}

#[test]
fn large_roster_same_name_library_target_exposes_a_validated_core_protection_plan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    for (agent_root, prefix, shared_body) in [
        (home.join(".codex/skills"), "codex", "codex variant"),
        (home.join(".hermes/skills"), "hermes", "hermes variant"),
        (home.join(".pi/agent/skills"), "pi", "pi variant"),
    ] {
        for index in 0..50 {
            let name = format!("{prefix}-skill-{index:03}");
            let skill = agent_root.join(&name);
            fs::create_dir_all(&skill).unwrap();
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: {name}\n---\nfixture\n"),
            )
            .unwrap();
        }
        let shared = agent_root.join("zz-shared");
        fs::create_dir_all(&shared).unwrap();
        fs::write(
            shared.join("SKILL.md"),
            format!("---\nname: zz-shared\n---\n{shared_body}\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["kind"] == "large_default_roster")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", &finding_id, "--full"]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    assert_eq!(planning["supported"], false);
    assert_eq!(planning["reason"], "same_name_library_target_conflict");
    assert_eq!(
        planning["reason_code"],
        "same_name_variants_require_explicit_preservation"
    );
    let claims = &planning["library_target_claims"];
    assert_eq!(claims["claimant_count"], 3);
    assert_eq!(claims["claimants_complete"], true);
    assert_eq!(claims["groups"].as_array().unwrap().len(), 1);
    assert_eq!(
        claims["groups"][0]["claimants"].as_array().unwrap().len(),
        3
    );
    assert_eq!(
        claims["groups"][0]["same_name_finding"]["state"],
        "available"
    );
    assert!(claims["groups"][0]["same_name_finding"]["finding_id"].is_string());
    let choice = &planning["resolution_choices"][0];
    assert_eq!(choice["choice"], "protect_library_target_claimants_as_core");
    assert_eq!(choice["available"], true);
    assert_eq!(choice["protected_skill_ids_complete"], true);
    assert_eq!(choice["protected_skill_ids"].as_array().unwrap().len(), 3);

    let unprotected = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let blocked = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&unprotected.to_string()),
    );
    assert!(!blocked.status.success());
    let blocked: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(
        blocked["error"]["code"],
        "roster_library_target_claim_conflict"
    );
    assert_eq!(
        blocked["error"]["details"]["claimants"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(blocked["error"]["details"]["claimant_count"], 3);

    let protected = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&choice["plan_request_template"].to_string()),
    ));
    assert_eq!(protected["result"]["state"], "ready", "{protected}");
    let plan_id = protected["result"]["plan_id"].as_str().unwrap();
    let protected_detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));
    let operations = protected_detail["result"]["operations"].as_array().unwrap();
    let targets = operations
        .iter()
        .filter_map(|operation| operation["target"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(targets.len(), operations.len());
}

#[test]
fn compact_large_roster_hides_a_template_when_later_claim_groups_exceed_its_bound() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    for index in 0..50 {
        let name = format!("core-skill-{index:03}");
        let skill = root.join(&name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture\n"),
        )
        .unwrap();
    }
    for (group, name) in [("alpha", "zz-alpha"), ("beta", "zz-beta")] {
        for index in 0..6 {
            let skill = root.join(format!("{group}-{index:02}"));
            fs::create_dir_all(&skill).unwrap();
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: {name}\n---\n{group} variant {index}\n"),
            )
            .unwrap();
        }
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["kind"] == "large_default_roster")
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let compact = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    let planning = &compact["result"]["planning"];
    assert_eq!(planning["reason"], "library_target_claim_conflicts");
    assert_eq!(planning["library_target_claims"]["group_count"], 2);
    assert_eq!(planning["library_target_claims"]["claimant_count"], 12);
    assert_eq!(
        planning["library_target_claims"]["claimants_complete"],
        false
    );
    let compact_choice = &planning["resolution_choices"][0];
    assert_eq!(compact_choice["available"], false);
    assert_eq!(compact_choice["protected_skill_ids_complete"], false);
    assert!(compact_choice["protected_skill_ids"].is_null());
    assert!(compact_choice["plan_request_template"].is_null());

    let full = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id, "--full"]].concat(),
        None,
    ));
    let full_planning = &full["result"]["planning"];
    assert_eq!(
        full_planning["library_target_claims"]["claimants_complete"],
        true
    );
    assert_eq!(
        full_planning["library_target_claims"]["groups"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let full_choice = &full_planning["resolution_choices"][0];
    assert_eq!(full_choice["available"], true);
    assert_eq!(full_choice["protected_skill_ids_complete"], true);
    assert_eq!(
        full_choice["protected_skill_ids"].as_array().unwrap().len(),
        12
    );
    assert!(full_choice["plan_request_template"].is_object());
}

#[test]
fn repeated_claims_for_one_library_target_merge_before_finding_and_confirmation() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    for index in 0..48 {
        let name = format!("core-skill-{index:03}");
        let skill = root.join(&name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture\n"),
        )
        .unwrap();
    }
    for index in 0..4 {
        let skill = root.join(format!("shared-{index:02}"));
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!("---\nname: zz-shared\n---\nvariant {index}\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["kind"] == "large_default_roster")
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id, "--full"]].concat(),
        None,
    ));
    let planning = &detail["result"]["planning"];
    let claims = &planning["library_target_claims"];
    assert_eq!(claims["group_count"], 1);
    assert_eq!(claims["claimant_count"], 4);
    assert_eq!(claims["groups"][0]["claimant_count"], 4);
    assert_eq!(
        claims["groups"][0]["claimants"].as_array().unwrap().len(),
        4
    );
    assert_eq!(
        claims["groups"][0]["same_name_finding"]["state"],
        "available"
    );
    let choice = &planning["resolution_choices"][0];
    assert_eq!(choice["available"], true);
    assert_eq!(choice["protected_skill_ids"].as_array().unwrap().len(), 4);

    let protected = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&choice["plan_request_template"].to_string()),
    ));
    assert_eq!(protected["result"]["state"], "ready");
}

#[test]
#[cfg(unix)]
fn roster_plan_keeps_summary_detail_and_confirmation_scope_consistent() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_root = home.join(".codex/skills");
    let opencode_root = home.join(".config/opencode/skills");
    fs::create_dir_all(&codex_root).unwrap();
    fs::create_dir_all(&opencode_root).unwrap();

    for index in 0..51 {
        let name = format!("skill-{index:03}");
        let directory = codex_root.join(&name);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let canonical = codex_root.join("zzz-canonical");
    fs::create_dir(&canonical).unwrap();
    fs::write(
        canonical.join("SKILL.md"),
        "---\nname: zzz-canonical\n---\nfixture\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&canonical, opencode_root.join("zzz-canonical")).unwrap();

    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let request = json!({
        "schema_version": 1,
        "finding_roster_changes": [{
            "finding_id": finding_id,
            "core_budget": 50,
            "protected_skill_ids": []
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    let plan_id = plan["result"]["plan_id"].as_str().unwrap();
    let detail = json_output(&run(
        &[&common[..], &["plan", "--show", plan_id]].concat(),
        None,
    ));

    assert_eq!(plan["result"]["risk"], "roster_change");
    assert_eq!(detail["result"]["risk"], plan["result"]["risk"]);
    assert_eq!(
        plan["result"]["affected"]["agents"],
        json!(["codex", "opencode"])
    );
    assert_eq!(detail["result"]["affected"], plan["result"]["affected"]);
    assert_eq!(
        detail["result"]["change_summary"],
        plan["result"]["change_summary"]
    );
    assert_eq!(detail["result"]["impact"], plan["result"]["impact"]);
    assert_eq!(
        detail["result"]["diff_summary"],
        plan["result"]["diff_summary"]
    );
    assert_eq!(plan["result"]["diff_summary"]["item_count"], 3);
    assert_eq!(plan["result"]["diff_summary"]["items"][0]["kind"], "roster");
    assert_eq!(
        plan["result"]["diff_summary"]["items"][1]["kind"],
        "library"
    );
    assert_eq!(
        plan["result"]["diff_summary"]["items"][2]["kind"],
        "filesystem"
    );

    let human = run(
        &[
            "--home",
            home.to_str().unwrap(),
            "--state-dir",
            state.to_str().unwrap(),
            "plan",
            "--show",
            plan_id,
        ],
        None,
    );
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("default exposure 53 → 51"));
    assert!(human.contains("2 Agents · 52 Skills"), "{human}");
    assert!(human.contains("Risk                   roster_change"));
}

#[test]
fn large_roster_apply_reduces_exposure_and_undo_restores_every_skill() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    for index in 0..101 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let evidence_ids = payload["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill_evidence_id(&database, snapshot, skill["id"].as_str().unwrap()))
        .collect::<Vec<_>>();
    let roster_changes = payload["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| {
            json!({
                "agent": "codex",
                "skill_id": skill["id"],
                "state": "on_demand"
            })
        })
        .collect::<Vec<_>>();
    let proposal = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": evidence_ids,
        "roster_changes": roster_changes
    });
    let plan_output = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&proposal.to_string()),
    );
    assert!(
        plan_output.stdout.len() < 12_000,
        "large Roster summary was {} bytes",
        plan_output.stdout.len()
    );
    let plan = json_output(&plan_output);
    assert_eq!(plan["result"]["detail_level"], "summary");
    assert!(plan["result"].get("operations").is_none());
    assert!(plan["result"].get("roster_changes").is_none());
    assert_eq!(plan["result"]["affected"]["skill_count"], 101);
    assert!(
        plan["result"]["affected"]["skill_ids"]
            .as_array()
            .unwrap()
            .len()
            <= 10
    );
    assert_eq!(plan["result"]["affected"]["skill_ids_truncated"], true);
    assert_eq!(plan["result"]["impact"]["before_default_exposure"], 101);
    assert!(
        plan["result"]["impact"]["after_default_exposure"]
            .as_u64()
            .unwrap()
            <= 50
    );
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    assert_eq!(fs::read_dir(state.join("library")).unwrap().count(), 101);
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let governed_find = json_output(&run(&[&common[..], &["find", "skill-000"]].concat(), None));
    assert_eq!(
        governed_find["result"]["matches"][0]["roster_state"],
        "on_demand"
    );
    assert_find_paths_are_readable(&governed_find);
    assert!(
        governed_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| Path::new(path.as_str().unwrap())
                .components()
                .any(|component| component.as_os_str() == "library"))
    );
    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 101);
    assert!(!state.join("library").exists());
    for index in 0..101 {
        assert!(root.join(format!("skill-{index:03}/SKILL.md")).is_file());
    }
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let restored_find = json_output(&run(&[&common[..], &["find", "skill-000"]].concat(), None));
    assert_eq!(
        restored_find["result"]["matches"][0]["roster_state"],
        "unassigned"
    );
    assert_find_paths_are_readable(&restored_find);
}

#[test]
fn core_roster_adds_verified_link_and_undo_restores_absence() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex_root = home.join(".codex/skills");
    let canonical = home.join(".claude/skills/shared");
    fs::create_dir_all(&codex_root).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::write(canonical.join("SKILL.md"), "---\nname: shared\n---\nbody\n").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = payload["skills"][0]["id"].as_str().unwrap();
    let evidence_id = skill_evidence_id(&database, snapshot, skill_id);
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": [{"agent": "codex", "skill_id": skill_id, "state": "core"}]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    assert_eq!(
        plan["result"]["impact"]["operation_groups"]["add_core_exposure"],
        1
    );
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    let linked = codex_root.join("shared");
    assert!(
        fs::symlink_metadata(&linked)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&linked).unwrap(),
        fs::canonicalize(&canonical).unwrap()
    );
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let core_find = json_output(&run(&[&common[..], &["find", "shared"]].concat(), None));
    assert_eq!(core_find["result"]["matches"][0]["roster_state"], "core");
    assert_find_paths_are_readable(&core_find);
    assert!(
        core_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == linked.join("SKILL.md").to_str().unwrap())
    );
    let undone = json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(undone["result"]["verification"], "passed");
    assert!(!linked.exists());
    assert!(canonical.join("SKILL.md").is_file());
    let restored_scan = json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let restored_find = json_output(&run(&[&common[..], &["find", "shared"]].concat(), None));
    assert_eq!(
        restored_find["result"]["matches"][0]["roster_state"],
        "unassigned"
    );
    assert_find_paths_are_readable(&restored_find);
    assert!(
        restored_find["result"]["matches"][0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| path != linked.join("SKILL.md").to_str().unwrap())
    );

    let restored_snapshot = restored_scan["result"]["snapshot_id"].as_str().unwrap();
    let restored_evidence_id = skill_evidence_id(&database, restored_snapshot, skill_id);
    let mut drift_request = request.clone();
    drift_request["scan_id"] = json!(restored_snapshot);
    drift_request["evidence_ids"] = json!([restored_evidence_id]);
    let drift_plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&drift_request.to_string()),
    ));
    fs::write(
        canonical.join("SKILL.md"),
        "---\nname: shared\n---\nchanged after plan\n",
    )
    .unwrap();
    let drifted = run(
        &[
            &common[..],
            &["apply", drift_plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    );
    assert!(!drifted.status.success());
    assert!(!linked.exists());
}

#[test]
fn public_find_uses_full_fts_body_and_archive_undo_restores_routing() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/deep-search");
    fs::create_dir_all(&skill).unwrap();
    let filler = "ordinary ".repeat(80);
    fs::write(
        skill.join("SKILL.md"),
        format!(
            "---\nname: deep-search\ndescription: generic helper\n---\n{filler} phosphorescent-reconciliation\n"
        ),
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let found = json_output(&run(
        &[&common[..], &["find", "phosphorescent reconciliation"]].concat(),
        None,
    ));
    assert_eq!(found["result"]["matches"][0]["name"], "deep-search");

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let skill_id = payload["skills"][0]["id"].as_str().unwrap();
    let evidence_id = skill_evidence_id(&database, snapshot, skill_id);
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": [{"agent": "codex", "skill_id": skill_id, "state": "archived"}]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let archived = json_output(&run(
        &[&common[..], &["find", "phosphorescent reconciliation"]].concat(),
        None,
    ));
    assert_eq!(archived["result"]["matches"], json!([]));
    let archived_load = run(
        &[
            &common[..],
            &["find", "phosphorescent reconciliation", "--load"],
        ]
        .concat(),
        None,
    );
    assert!(!archived_load.status.success());
    let archived_load: Value = serde_json::from_slice(&archived_load.stdout).unwrap();
    assert_eq!(
        archived_load["error"]["code"],
        "verified_skill_load_blocked"
    );
    assert_eq!(
        archived_load["error"]["details"]["reason"],
        "archived_skill_not_routable"
    );
    assert_eq!(
        archived_load["error"]["details"]["retry_mode"],
        "agent_choice_required"
    );
    assert_eq!(archived_load["suggested_actions"], json!([]));
    assert!(archived_load["error"]["details"].get("content").is_none());

    json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));
    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));
    let restored = json_output(&run(
        &[&common[..], &["find", "phosphorescent reconciliation"]].concat(),
        None,
    ));
    assert_eq!(restored["result"]["matches"][0]["name"], "deep-search");
}

#[test]
fn archived_same_name_identity_cannot_return_through_active_variant() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex = home.join(".codex/skills/shared-route");
    let claude = home.join(".claude/skills/shared-route");
    fs::create_dir_all(&codex).unwrap();
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        codex.join("SKILL.md"),
        "---\nname: shared-route\ndescription: archived identity\n---\narchived-only-marker\n",
    )
    .unwrap();
    fs::write(
        claude.join("SKILL.md"),
        "---\nname: shared-route\ndescription: active identity\n---\nactive-search-marker\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    json_output(&run(&[&common[..], &["report"]].concat(), None));
    let before = json_output(&run(
        &[&common[..], &["find", "active search marker"]].concat(),
        None,
    ));
    assert_eq!(before["result"]["matches"][0]["variant_count"], 2);
    let ambiguous_load = run(
        &[
            &common[..],
            &[
                "find",
                "active search marker",
                "--hint",
                "inspect active identity",
                "--load",
                "--limit",
                "1",
            ],
        ]
        .concat(),
        None,
    );
    assert!(!ambiguous_load.status.success());
    let ambiguous_load: Value = serde_json::from_slice(&ambiguous_load.stdout).unwrap();
    assert_eq!(
        ambiguous_load["error"]["details"]["reason"],
        "same_name_variants_ambiguous"
    );
    assert_eq!(
        ambiguous_load["error"]["details"]["next_action"],
        "inspect_same_name_variants"
    );
    assert_eq!(
        ambiguous_load["suggested_actions"][0],
        json!({
            "action": "inspect_same_name_variants",
            "description": "inspect_same_name_variants",
            "argv": [
                env!("CARGO_BIN_EXE_skillroster"),
                "--state-dir",
                state,
                "--home",
                home,
                "find",
                "--hint",
                "inspect active identity",
                "--limit",
                "1",
                "--require-snapshot",
                snapshot,
                "--json",
                "--",
                "active search marker"
            ],
            "mutates": false,
            "requires_confirmation": false,
            "reason_code": "same_name_variants_ambiguous"
        })
    );
    let variants = json_output(&run_suggested_action(
        &ambiguous_load["suggested_actions"][0],
    ));
    assert_eq!(variants["result"]["matches"][0]["variant_count"], 2);
    assert_eq!(
        variants["result"]["matches"][0]["variant_finding"]["state"],
        "available"
    );
    assert_eq!(
        variants["result"]["matches"][0]["variant_finding"]["snapshot_id"],
        snapshot
    );
    assert!(
        variants["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["action"] == "inspect_variant_finding")
    );
    assert_eq!(
        variants["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| action["action"] == "load_exact_variant_for_comparison")
            .count(),
        2
    );
    assert!(
        variants["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| action["action"] == "load_exact_variant_for_comparison")
            .all(|action| action["argv"]
                .as_array()
                .unwrap()
                .windows(2)
                .any(|pair| { pair == [json!("--require-snapshot"), json!(snapshot)] }))
    );

    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let codex_skill_id = payload["placements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|placement| placement["agent"] == "codex")
        .and_then(|placement| placement["skill_id"].as_str())
        .unwrap();
    let codex_skill_id = codex_skill_id.to_owned();
    let evidence_id = skill_evidence_id(&database, snapshot, &codex_skill_id);
    let request = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": [{
            "agent": "codex",
            "skill_id": codex_skill_id,
            "state": "archived"
        }]
    });
    let plan = json_output(&run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&request.to_string()),
    ));
    let applied = json_output(&run(
        &[
            &common[..],
            &["apply", plan["result"]["plan_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));

    json_output(&run(&[&common[..], &["scan", "--summary"]].concat(), None));

    let after = json_output(&run(
        &[&common[..], &["find", "active search marker"]].concat(),
        None,
    ));
    assert_eq!(after["result"]["matches"][0]["variant_count"], 1);
    assert!(after["result"]["matches"][0]["variants"].is_null());
    assert!(after["result"]["matches"][0]["variant_finding"].is_null());

    let archived_variant = run(
        &[
            &common[..],
            &[
                "find",
                "active search marker",
                "--load",
                "--limit",
                "1",
                "--variant-skill-id",
                &codex_skill_id,
            ],
        ]
        .concat(),
        None,
    );
    assert!(!archived_variant.status.success());
    let archived_variant: Value = serde_json::from_slice(&archived_variant.stdout).unwrap();
    assert_eq!(
        archived_variant["error"]["details"]["reason"],
        "variant_selector_requires_ambiguous_top_match"
    );
    assert!(
        archived_variant["error"]["details"]
            .get("content")
            .is_none()
    );

    json_output(&run(
        &[
            &common[..],
            &["undo", applied["result"]["receipt_id"].as_str().unwrap()],
        ]
        .concat(),
        None,
    ));

    let newer_scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let stale_retry = run_suggested_action(&ambiguous_load["suggested_actions"][0]);
    assert!(!stale_retry.status.success());
    let stale_retry: Value = serde_json::from_slice(&stale_retry.stdout).unwrap();
    assert_eq!(stale_retry["error"]["code"], "find_snapshot_changed");
    assert_eq!(
        stale_retry["error"]["details"]["expected_snapshot_id"],
        snapshot
    );
    assert_eq!(
        stale_retry["error"]["details"]["actual_snapshot_id"],
        newer_scan["result"]["snapshot_id"]
    );
    assert_eq!(stale_retry["error"]["details"]["files_changed"], false);
    assert!(stale_retry["result"].is_null());
    let fresh_snapshot = newer_scan["result"]["snapshot_id"].as_str().unwrap();
    let fresh_retry = &stale_retry["suggested_actions"][0];
    assert_eq!(fresh_retry["action"], "rerun_find_on_latest_snapshot");
    assert!(
        fresh_retry["argv"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair == [json!("--require-snapshot"), json!(fresh_snapshot)])
    );

    let fresh_variants = json_output(&run_suggested_action(fresh_retry));
    assert_eq!(
        fresh_variants["result"]["matches"][0]["variant_finding"]["snapshot_id"],
        fresh_snapshot
    );
    let fresh_load_actions = fresh_variants["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["action"] == "load_exact_variant_for_comparison")
        .collect::<Vec<_>>();
    assert_eq!(fresh_load_actions.len(), 2);
    assert!(fresh_load_actions.iter().all(|action| {
        action["argv"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair == [json!("--require-snapshot"), json!(fresh_snapshot)])
    }));
}

#[test]
fn find_loads_only_an_exposed_exact_variant_from_the_ambiguous_top_match() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex = home.join(".codex/skills/shared-route");
    let claude = home.join(".claude/skills/shared-route");
    let unique = home.join(".codex/skills/unique-route");
    fs::create_dir_all(&codex).unwrap();
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&unique).unwrap();
    let codex_content = "---\nname: shared-route\ndescription: compare exact variants\n---\ncodex-entrypoint-marker\n";
    let claude_content = "---\nname: shared-route\ndescription: compare exact variants\n---\nclaude-entrypoint-marker\n";
    fs::write(codex.join("SKILL.md"), codex_content).unwrap();
    fs::write(claude.join("SKILL.md"), claude_content).unwrap();
    fs::write(
        unique.join("SKILL.md"),
        "---\nname: unique-route\ndescription: unique selector guard\n---\nunique-marker\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();

    let found = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "compare exact variants",
                "--hint",
                "inspect alternative instructions",
                "--limit",
                "1",
            ],
        ]
        .concat(),
        None,
    ));
    let ranked = &found["result"]["matches"][0];
    assert_eq!(ranked["variant_count"], 2);
    let variants = ranked["variants"].as_array().unwrap();
    let actions = found["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["action"] == "load_exact_variant_for_comparison")
        .collect::<Vec<_>>();
    assert_eq!(actions.len(), variants.len());
    for action in &actions {
        assert_eq!(action["mutates"], false);
        assert_eq!(action["requires_confirmation"], false);
        let argv = action["argv"].as_array().unwrap();
        assert!(
            argv.windows(2).any(|pair| {
                pair == [json!("--hint"), json!("inspect alternative instructions")]
            })
        );
        assert!(argv.windows(2).any(|pair| {
            pair[0] == json!("--variant-skill-id")
                && variants
                    .iter()
                    .any(|variant| pair[1] == variant["skill_id"])
        }));
    }

    for variant in variants {
        let skill_id = variant["skill_id"].as_str().unwrap();
        let loaded = json_output(&run(
            &[
                &common[..],
                &[
                    "find",
                    "compare exact variants",
                    "--hint",
                    "inspect alternative instructions",
                    "--load",
                    "--limit",
                    "1",
                    "--variant-skill-id",
                    skill_id,
                ],
            ]
            .concat(),
            None,
        ));
        let exact = &loaded["result"]["loaded_skill"];
        assert_eq!(exact["selection"]["skill_id"], skill_id);
        assert_eq!(
            exact["selection"]["variant_selection"]["requested_skill_id"],
            skill_id
        );
        assert_eq!(
            exact["selection"]["variant_selection"]["ranked_variant_count"],
            2
        );
        assert_eq!(
            exact["selection"]["ranking_evidence_scope"],
            "ranked_capability_group"
        );
        let expected = if variant["agents"]
            .as_array()
            .unwrap()
            .contains(&json!("codex"))
        {
            codex_content
        } else {
            claude_content
        };
        assert_eq!(exact["content"]["text"], expected);
        assert!(
            variant["paths"]
                .as_array()
                .unwrap()
                .contains(&exact["content"]["path"])
        );
        assert_eq!(exact["content"]["complete"], true);
        assert_eq!(exact["governance"]["content_endorsed"], false);
        assert_eq!(exact["task_success"], "not_evaluated");
        assert_eq!(loaded["result"]["files_changed"], false);
    }

    let unknown = run(
        &[
            &common[..],
            &[
                "find",
                "compare exact variants",
                "--load",
                "--limit",
                "1",
                "--variant-skill-id",
                "skill_not_exposed",
            ],
        ]
        .concat(),
        None,
    );
    assert!(!unknown.status.success());
    let unknown: Value = serde_json::from_slice(&unknown.stdout).unwrap();
    assert_eq!(
        unknown["error"]["details"]["reason"],
        "variant_not_in_top_match"
    );
    assert_eq!(
        unknown["error"]["details"]["retry_mode"],
        "agent_choice_required"
    );
    assert!(unknown["error"]["details"].get("content").is_none());

    let missing_load = run(
        &[
            &common[..],
            &[
                "find",
                "compare exact variants",
                "--variant-skill-id",
                variants[0]["skill_id"].as_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    );
    assert!(!missing_load.status.success());
    let missing_load: Value = serde_json::from_slice(&missing_load.stdout).unwrap();
    assert_eq!(
        missing_load["error"]["details"]["reason"],
        "variant_selector_requires_load"
    );
    assert_eq!(
        missing_load["error"]["details"]["retry_mode"],
        "agent_correction_required"
    );

    let unambiguous = run(
        &[
            &common[..],
            &[
                "find",
                "unique selector guard",
                "--load",
                "--limit",
                "1",
                "--variant-skill-id",
                variants[0]["skill_id"].as_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    );
    assert!(!unambiguous.status.success());
    let unambiguous: Value = serde_json::from_slice(&unambiguous.stdout).unwrap();
    assert_eq!(
        unambiguous["error"]["details"]["reason"],
        "variant_selector_requires_ambiguous_top_match"
    );

    let claude_id = variants
        .iter()
        .find(|variant| {
            variant["agents"]
                .as_array()
                .unwrap()
                .contains(&json!("claude-code"))
        })
        .and_then(|variant| variant["skill_id"].as_str())
        .unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let mut payload: Value = serde_json::from_str(&payload).unwrap();
    let claude_placement = payload["placements"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|placement| placement["skill_id"] == claude_id)
        .unwrap();
    claude_placement["mutation_scope"] = json!("untrusted_external");
    claude_placement["governable"] = json!(false);
    database
        .execute(
            "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
            (payload.to_string(), snapshot),
        )
        .unwrap();
    let untrusted = run(
        &[
            &common[..],
            &[
                "find",
                "compare exact variants",
                "--load",
                "--limit",
                "1",
                "--variant-skill-id",
                claude_id,
            ],
        ]
        .concat(),
        None,
    );
    assert!(!untrusted.status.success());
    let untrusted: Value = serde_json::from_slice(&untrusted.stdout).unwrap();
    assert_eq!(
        untrusted["error"]["details"]["reason"],
        "untrusted_external_source"
    );
    assert!(untrusted["error"]["details"].get("content").is_none());

    fs::write(codex.join("SKILL.md"), "changed after scan\n").unwrap();
    let codex_id = variants
        .iter()
        .find(|variant| {
            variant["agents"]
                .as_array()
                .unwrap()
                .contains(&json!("codex"))
        })
        .and_then(|variant| variant["skill_id"].as_str())
        .unwrap();
    let drifted = run(
        &[
            &common[..],
            &[
                "find",
                "compare exact variants",
                "--load",
                "--limit",
                "1",
                "--variant-skill-id",
                codex_id,
            ],
        ]
        .concat(),
        None,
    );
    assert!(!drifted.status.success());
    let drifted: Value = serde_json::from_slice(&drifted.stdout).unwrap();
    assert_eq!(
        drifted["error"]["details"]["reason"],
        "entrypoint_content_drift"
    );
    assert!(drifted["error"]["details"].get("content").is_none());
}

#[test]
fn find_rejects_content_drift_and_requests_rescan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/drift-check");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: drift-check\n---\nneedle-before-drift\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: drift-check\n---\nlocally changed\n",
    )
    .unwrap();

    let found = json_output(&run(
        &[&common[..], &["find", "needle before drift"]].concat(),
        None,
    ));
    assert_eq!(found["result"]["matches"][0]["paths"], json!([]));
    assert_eq!(found["result"]["rescan_required"], true);
    assert!(
        found["result"]["warnings"][0]
            .as_str()
            .unwrap()
            .contains("skillroster scan")
    );

    let loaded = run(
        &[
            &common[..],
            &["find", "needle before drift", "--load", "--limit", "1"],
        ]
        .concat(),
        None,
    );
    assert!(!loaded.status.success());
    let loaded: Value = serde_json::from_slice(&loaded.stdout).unwrap();
    assert_eq!(
        loaded["error"]["details"]["reason"],
        "entrypoint_content_drift"
    );
    let retry_argv = loaded["suggested_actions"][0]["argv"].as_array().unwrap();
    assert_eq!(
        &retry_argv[retry_argv.len() - 3..],
        &[json!("scan"), json!("--summary"), json!("--json")]
    );
    assert!(loaded["error"]["details"].get("content").is_none());

    fs::write(skill.join("SKILL.md"), vec![b'x'; 128 * 1024 + 1]).unwrap();
    let oversized = run(
        &[
            &common[..],
            &["find", "needle before drift", "--load", "--limit", "1"],
        ]
        .concat(),
        None,
    );
    assert!(!oversized.status.success());
    let oversized: Value = serde_json::from_slice(&oversized.stdout).unwrap();
    assert_eq!(
        oversized["error"]["details"]["reason"],
        "entrypoint_exceeds_content_limit"
    );
    assert_eq!(
        oversized["error"]["details"]["retry_mode"],
        "manual_resolution_required"
    );
    assert_eq!(oversized["suggested_actions"], json!([]));

    fs::remove_file(skill.join("SKILL.md")).unwrap();
    let unreadable = run(
        &[
            &common[..],
            &["find", "needle before drift", "--load", "--limit", "1"],
        ]
        .concat(),
        None,
    );
    assert!(!unreadable.status.success());
    let unreadable: Value = serde_json::from_slice(&unreadable.stdout).unwrap();
    assert_eq!(
        unreadable["error"]["details"]["reason"],
        "entrypoint_unreadable"
    );
    assert_eq!(
        unreadable["error"]["details"]["next_action"],
        "repair_local_read_access_then_scan"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_unicode_identity_coverage_blocks_exact_snapshot_consumers() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = temp.path().join("skills");
    let valid = root.join("valid");
    fs::create_dir_all(&valid).unwrap();
    fs::write(
        valid.join("SKILL.md"),
        "---\nname: valid\ndescription: exact unicode fixture\n---\n",
    )
    .unwrap();
    let invalid = root.join(OsString::from_vec(vec![0x80]));
    fs::create_dir(&invalid).unwrap();
    fs::write(invalid.join("SKILL.md"), "---\nname: hidden\n---\n").unwrap();
    let explicit_root = format!("codex={}", root.display());
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--root",
        &explicit_root,
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    for (tail, stdin) in [
        (vec!["find", "exact unicode fixture", "--load"], None),
        (vec!["report", "--summary"], None),
        (vec!["plan", "--stdin"], Some("{}")),
    ] {
        let rejected = run(&[&common[..], &tail].concat(), stdin);
        assert!(!rejected.status.success());
        let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert_eq!(
            rejected["error"]["code"],
            "content_identity_rescan_required"
        );
        assert_eq!(
            rejected["error"]["details"]["reason"],
            "non_unicode_identity_coverage_incomplete"
        );
    }
}

#[test]
fn find_preserves_leading_hyphen_tasks_and_replayable_variant_actions() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let codex = home.join(".codex/skills/hyphen-route");
    let claude = home.join(".claude/skills/hyphen-route");
    fs::create_dir_all(&codex).unwrap();
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        codex.join("SKILL.md"),
        "---\nname: hyphen-route\ndescription: Review a leading hyphen route\n---\ncodex variant\n",
    )
    .unwrap();
    fs::write(
        claude.join("SKILL.md"),
        "---\nname: hyphen-route\ndescription: Review a leading hyphen route\n---\nclaude variant\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let task = "--review leading hyphen route";
    let hint = "review route";

    for tail in [&["find"][..], &["find", "--", ""][..]] {
        let rejected = run(&[&common[..], tail].concat(), None);
        assert!(!rejected.status.success());
        let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert_eq!(rejected["error"]["code"], "invalid_cli_arguments");
    }

    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    json_output(&run(&[&common[..], &["report"]].concat(), None));
    let found = json_output(&run(
        &[&common[..], &["find", task, "--hint", hint, "--limit", "1"]].concat(),
        None,
    ));

    assert_eq!(found["result"]["task"], task);
    assert_eq!(found["result"]["retrieval_hints"], json!([hint]));
    assert_eq!(found["result"]["matches"][0]["name"], "hyphen-route");
    assert_eq!(found["result"]["matches"][0]["variant_count"], 2);
    let load_action = found["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "load_exact_variant_for_comparison")
        .unwrap();
    assert!(
        load_action["argv"]
            .as_array()
            .unwrap()
            .contains(&json!(task))
    );
    assert_eq!(
        &load_action["argv"].as_array().unwrap()
            [load_action["argv"].as_array().unwrap().len() - 2..],
        &[json!("--"), json!(task)]
    );
    assert!(
        load_action["argv"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair == [json!("--require-snapshot"), json!(snapshot)])
    );

    let loaded = json_output(&run_suggested_action(load_action));
    assert_eq!(loaded["result"]["task"], task);
    assert_eq!(loaded["result"]["retrieval_hints"], json!([hint]));
    assert_eq!(
        loaded["result"]["loaded_skill"]["selection"]["name"],
        "hyphen-route"
    );
    assert_eq!(
        loaded["result"]["loaded_skill"]["verification"]["identity_matches_snapshot"],
        true
    );

    let refreshed = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_ne!(refreshed["result"]["snapshot_id"], snapshot);
    let stale = run_suggested_action(load_action);
    assert!(!stale.status.success());
    let stale: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(stale["error"]["code"], "find_snapshot_changed");
    assert_eq!(stale["error"]["details"]["expected_snapshot_id"], snapshot);
    assert_eq!(
        stale["error"]["details"]["actual_snapshot_id"],
        refreshed["result"]["snapshot_id"]
    );

    let option_shaped_task = "--json";
    let reserved = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "--hint",
                hint,
                "--limit",
                "1",
                "--",
                option_shaped_task,
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(reserved["result"]["task"], option_shaped_task);
    assert_eq!(reserved["result"]["matches"][0]["name"], "hyphen-route");
    assert!(
        reserved["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| action["action"] == "load_exact_variant_for_comparison")
            .all(|action| {
                let argv = action["argv"].as_array().unwrap();
                argv[argv.len() - 2..] == [json!("--"), json!(option_shaped_task)]
            })
    );
}

#[test]
fn find_load_returns_the_complete_verified_top_match_in_one_envelope() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/event-manifest");
    fs::create_dir_all(&skill).unwrap();
    let content = "---\nname: event-manifest\ndescription: Build a deterministic event manifest\n---\n\nFollow the complete event manifest instructions.\n";
    fs::write(skill.join("SKILL.md"), content).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));

    let found = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "build a deterministic event manifest",
                "--load",
                "--limit",
                "1",
            ],
        ]
        .concat(),
        None,
    ));
    let loaded = &found["result"]["loaded_skill"];
    assert_eq!(found["result"]["matches"][0]["name"], "event-manifest");
    assert_eq!(loaded["selection"]["rank"], 1);
    assert_eq!(
        loaded["selection"]["ranking_evidence_scope"],
        "loaded_identity"
    );
    assert_eq!(loaded["content"]["text"], content);
    assert_eq!(loaded["content"]["byte_length"], content.len());
    assert_eq!(
        loaded["content"]["sha256"],
        format!("{:x}", Sha256::digest(content.as_bytes()))
    );
    assert_eq!(loaded["content"]["complete"], true);
    assert_eq!(
        loaded["verification"]["entrypoint_digest_matches_snapshot"],
        true
    );
    assert_eq!(loaded["governance"]["content_endorsed"], false);
    assert_eq!(loaded["task_success"], "not_evaluated");
    assert_eq!(found["result"]["files_changed"], false);
}

#[test]
fn find_load_rejects_an_untrusted_external_source_without_returning_content() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/external-helper");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: external-helper\ndescription: external trust fixture\n---\nsecret instructions\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot = scan["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let mut payload: Value = serde_json::from_str(&payload).unwrap();
    payload["placements"][0]["mutation_scope"] = json!("untrusted_external");
    payload["placements"][0]["governable"] = json!(false);
    database
        .execute(
            "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
            (payload.to_string(), snapshot),
        )
        .unwrap();

    let loaded = run(
        &[
            &common[..],
            &["find", "external trust fixture", "--load", "--limit", "1"],
        ]
        .concat(),
        None,
    );
    assert!(!loaded.status.success());
    let loaded: Value = serde_json::from_slice(&loaded.stdout).unwrap();
    assert_eq!(
        loaded["error"]["details"]["reason"],
        "untrusted_external_source"
    );
    assert!(!loaded.to_string().contains("secret instructions"));
}

#[test]
fn public_report_covers_issue_nine_finding_families_without_runtime_claims() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    for (directory, version, body) in [
        ("mismatch-one", "v1", "first body"),
        ("mismatch-copy", "v1", "changed local body"),
        ("mismatch-two", "v2", "second version body"),
    ] {
        let package = root.join(directory);
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("SKILL.md"),
            format!(
                "---\nname: declared-name\nsource: github:owner/report-fixture\nversion: {version}\n---\n{body}\n"
            ),
        )
        .unwrap();
        fs::write(package.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(&[&common[..], &["report", "--full"]].concat(), None));
    let findings = report["result"]["findings"].as_array().unwrap();
    for title in [
        "Declared identity has divergent local content",
        "Skill packages contain executable scripts",
        "Declared Skill names differ from placement directories",
        "Declared source has version divergence",
        "Upstream update drift is not verified",
    ] {
        let finding = findings
            .iter()
            .find(|finding| finding["title"] == title)
            .unwrap_or_else(|| panic!("missing public Finding: {title}"));
        assert!(finding["affected_skill_count"].as_u64().unwrap() > 0);
        assert!(!finding["evidence_ids"].as_array().unwrap().is_empty());
    }
    let encoded = report.to_string().to_lowercase();
    assert!(!encoded.contains("is malicious"));
    assert!(!encoded.contains("is safe at runtime"));
}

#[cfg(unix)]
#[test]
fn find_rejects_a_skill_symlink_that_now_escapes_approved_roots() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let skill = home.join(".codex/skills/escape-check");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: escape-check\n---\nunique escape needle\n",
    )
    .unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let outside = temp.path().join("outside-skill");
    fs::rename(&skill, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &skill).unwrap();

    let found = json_output(&run(
        &[&common[..], &["find", "unique escape needle"]].concat(),
        None,
    ));
    assert_eq!(found["result"]["matches"][0]["paths"], json!([]));
    assert_eq!(found["result"]["rescan_required"], true);

    let loaded = run(
        &[
            &common[..],
            &["find", "unique escape needle", "--load", "--limit", "1"],
        ]
        .concat(),
        None,
    );
    assert!(!loaded.status.success());
    let loaded: Value = serde_json::from_slice(&loaded.stdout).unwrap();
    assert_eq!(
        loaded["error"]["details"]["reason"],
        "entrypoint_escapes_approved_roots"
    );
}

#[cfg(unix)]
#[test]
fn escaping_link_resolution_separates_durable_and_temporary_read_paths() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    let mut sources = Vec::new();
    for index in 0..11 {
        let source = temp.path().join(format!("source-{index}"));
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            format!("---\nname: external-{index}\ndescription: fixture\n---\n"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&source, root.join(format!("external-{index}"))).unwrap();
        sources.push(source);
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    let initial_scan = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let initial_snapshot = initial_scan["result"]["snapshot_id"].as_str().unwrap();
    let report = json_output(&run(&[&common[..], &["report"]].concat(), None));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let detail = json_output(&run(
        &[
            &common[..],
            &[
                "--source-root",
                sources[0].to_str().unwrap(),
                "report",
                "--finding",
                finding_id,
            ],
        ]
        .concat(),
        None,
    ));
    let resolution = &detail["result"]["resolution"];
    assert_eq!(detail["result"]["page"]["returned"]["items"], 5);
    assert_eq!(
        detail["result"]["page"]["totals"]["affected_placements"],
        11
    );
    assert_eq!(
        resolution["decision_code"],
        "source_read_permission_required"
    );
    assert_eq!(resolution["content_trust"], "not_assessed");
    assert_eq!(resolution["observed_link_target_count"], 11);
    assert_eq!(
        resolution["observed_link_targets"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(resolution["observed_link_targets_truncated"], true);
    assert_eq!(
        resolution["page_observed_link_targets"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(resolution["permission_paths"]["exclusive"], true);
    assert_eq!(
        resolution["permission_paths"]["durable_permission"]["next"]["confirmed_source_root_option_required"],
        false
    );
    let durable_argv =
        resolution["permission_paths"]["durable_permission"]["next"]["argv_template"]
            .as_array()
            .unwrap();
    assert!(!durable_argv.iter().any(|arg| arg == "--source-root"));
    assert_eq!(
        resolution["permission_paths"]["temporary_one_scan"]["persists"],
        false
    );
    assert!(
        resolution["permission_paths"]["temporary_one_scan"]["argv_template"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "--source-root")
    );
    let confirm_actions = detail["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["action"] == "confirm_source_root_read_permission")
        .count();
    assert_eq!(confirm_actions, 5);

    let expanded_detail = json_output(&run(
        &[
            &common[..],
            &["report", "--finding", finding_id, "--limit", "11"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(
        expanded_detail["result"]["resolution"]["page_observed_link_targets"]
            .as_array()
            .unwrap()
            .len(),
        11
    );
    assert_eq!(
        expanded_detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| action["action"] == "confirm_source_root_read_permission")
            .count(),
        11
    );

    let temporary_scan = json_output(&run(
        &[
            &common[..],
            &["--source-root", sources[0].to_str().unwrap(), "scan"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(
        temporary_scan["result"]["source_root_policy"]["permissions"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let temporary_snapshot = temporary_scan["result"]["snapshot_id"].as_str().unwrap();
    let temporary_status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(temporary_status["result"]["snapshot_state"], "current");
    assert_eq!(temporary_status["result"]["state"], "report_required");
    let temporary_load = json_output(&run(
        &[
            &common[..],
            &[
                "find",
                "external-0 fixture",
                "--require-snapshot",
                temporary_snapshot,
                "--load",
                "--limit",
                "1",
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(
        temporary_load["result"]["loaded_skill"]["selection"]["name"],
        "external-0"
    );
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let load_payload = |snapshot| {
        database
            .query_row(
                "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
                [snapshot],
                |row| row.get::<_, String>(0),
            )
            .map(|payload| serde_json::from_str::<Value>(&payload).unwrap())
            .unwrap()
    };
    let initial_payload = load_payload(initial_snapshot);
    let payload = load_payload(temporary_snapshot);
    let initial_placements = initial_payload["placements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|placement| {
            (
                placement["id"].as_str().unwrap(),
                placement["default_exposed"].as_bool().unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(initial_placements.len(), 11);
    let payload_placement_ids = payload["placements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|placement| placement["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert!(
        initial_placements
            .keys()
            .all(|id| payload_placement_ids.contains(id))
    );
    let mut statement = database
        .prepare("SELECT id, exposed FROM placements WHERE scan_id = ?1")
        .unwrap();
    let stored_placements = statement
        .query_map([temporary_snapshot], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })
        .unwrap()
        .collect::<Result<BTreeMap<_, _>, _>>()
        .unwrap();
    assert!(initial_placements.iter().all(|(id, exposed)| {
        stored_placements
            .get(*id)
            .is_some_and(|stored| stored == exposed)
    }));
    assert_eq!(
        stored_placements.keys().cloned().collect::<BTreeSet<_>>(),
        payload_placement_ids
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    );
    let foreign_key_violations: u64 = database
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(foreign_key_violations, 0);
}

#[cfg(unix)]
#[test]
fn escaping_link_finding_deduplicates_canonical_source_targets() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let source = temp.path().join("trusted-source");
    let alias = temp.path().join("source-alias");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: external\ndescription: fixture\n---\n",
    )
    .unwrap();
    let source = fs::canonicalize(source).unwrap();
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    std::os::unix::fs::symlink(&source, root.join("direct")).unwrap();
    std::os::unix::fs::symlink(&alias, root.join("through-alias")).unwrap();
    let entrypoint_link = root.join("entrypoint-link");
    fs::create_dir(&entrypoint_link).unwrap();
    std::os::unix::fs::symlink(source.join("SKILL.md"), entrypoint_link.join("SKILL.md")).unwrap();
    let malformed_target = temp.path().join("not-a-skill-directory");
    fs::write(&malformed_target, "not a Skill directory\n").unwrap();
    std::os::unix::fs::symlink(&malformed_target, root.join("malformed-directory-link")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));

    assert_eq!(
        detail["result"]["resolution"]["observed_link_targets"],
        json!([source])
    );
    assert_eq!(
        detail["result"]["resolution"]["observed_link_target_count"],
        1
    );
    assert_eq!(detail["result"]["impact"]["affected_placement_count"], 3);
    assert!(
        detail["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["facts"]["link_target"] == json!(source))
    );
    assert_eq!(
        detail["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|action| action["action"] == "confirm_source_root_read_permission")
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn escaping_link_finding_requests_exact_read_permission_instead_of_a_plan() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let source = temp.path().join("trusted-source");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: external\ndescription: fixture\n---\n",
    )
    .unwrap();
    let source = fs::canonicalize(source).unwrap();
    std::os::unix::fs::symlink(&source, root.join("external")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();

    let detail = json_output(&run(
        &[
            &common[..],
            &[
                "--source-root",
                source.to_str().unwrap(),
                "report",
                "--finding",
                finding_id,
            ],
        ]
        .concat(),
        None,
    ));
    assert_eq!(
        detail["result"]["resolution"]["decision"],
        "confirm_trusted_source_roots"
    );
    assert_eq!(
        detail["result"]["resolution"]["observed_link_targets"],
        json!([source])
    );
    assert_eq!(
        detail["result"]["resolution"]["automatic_change_supported"],
        false
    );
    let actions = detail["suggested_actions"].as_array().unwrap();
    assert_eq!(actions.len(), 2);
    assert!(
        actions
            .iter()
            .any(|action| action["action"] == "show_full_finding")
    );
    let confirm = actions
        .iter()
        .find(|action| action["action"] == "confirm_source_root_read_permission")
        .unwrap();
    assert_eq!(confirm["requires_confirmation"], true);
    assert_eq!(confirm["mutates"], true);
    assert!(
        detail["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["facts"]["link_target"] == json!(source))
    );

    let sibling = temp.path().join("unobserved-sibling");
    fs::create_dir(&sibling).unwrap();
    let unobserved = run(
        &[
            &common[..],
            &[
                "source-root",
                "confirm",
                "--finding",
                finding_id,
                "--path",
                sibling.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    );
    assert!(!unobserved.status.success());
    let unobserved: Value = serde_json::from_slice(&unobserved.stdout).unwrap();
    assert_eq!(unobserved["error"]["code"], "source_root_path_not_observed");
    assert_eq!(unobserved["error"]["details"]["path"], json!(sibling));

    let unobserved_alias = temp.path().join("unobserved-alias");
    std::os::unix::fs::symlink(&source, &unobserved_alias).unwrap();
    let alias = run(
        &[
            &common[..],
            &[
                "source-root",
                "confirm",
                "--finding",
                finding_id,
                "--path",
                unobserved_alias.to_str().unwrap(),
            ],
        ]
        .concat(),
        None,
    );
    assert!(!alias.status.success());
    let alias: Value = serde_json::from_slice(&alias.stdout).unwrap();
    assert_eq!(alias["error"]["code"], "source_root_path_not_observed");
    assert_eq!(alias["error"]["details"]["path"], json!(unobserved_alias));

    let confirmed = json_output(&run_suggested_action(confirm));
    assert_eq!(
        confirmed["result"]["permission_scope"],
        "exact_local_read_only"
    );
    assert_eq!(confirmed["result"]["content_endorsed"], false);
    assert_eq!(confirmed["result"]["evidence_quality_changed"], false);
    assert_eq!(confirmed["result"]["plan_apply_authorized"], false);
    assert_eq!(confirmed["result"]["files_changed"], false);
    assert_eq!(confirmed["result"]["state_files_changed"], true);
    let next_scan = confirmed["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "scan")
        .unwrap();
    assert!(
        !next_scan["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "--source-root")
    );
    let mut permission_id = confirmed["result"]["permission"]["permission_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let rescanned = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(
        rescanned["result"]["source_root_policy"]["permissions"][0]["state"],
        "active"
    );
    let found = json_output(&run(&[&common[..], &["find", "fixture"]].concat(), None));
    assert_eq!(found["result"]["matches"][0]["name"], "external");
    assert_eq!(found["result"]["matches"][0]["owned_by_agent"], true);
    assert_eq!(
        found["result"]["matches"][0]["mutation_scopes"],
        json!(["durable_read_only"])
    );
    assert_eq!(found["result"]["matches"][0]["governable"], false);

    let snapshot = rescanned["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot],
            |row| row.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    let external_skill = payload["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|skill| skill["name"] == "external")
        .unwrap();
    let skill_id = external_skill["id"].as_str().unwrap();
    let placements = payload["placements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|placement| placement["skill_id"] == skill_id)
        .collect::<Vec<_>>();
    assert!(!placements.is_empty());
    assert!(
        placements
            .iter()
            .all(|placement| placement["governable"] == false)
    );
    assert!(
        placements
            .iter()
            .any(|placement| placement["owned_by_agent"] == true)
    );
    assert!(
        placements
            .iter()
            .all(|placement| placement["mutation_scope"] == "durable_read_only")
    );
    assert!(
        placements
            .iter()
            .all(|placement| { placement["fingerprint_completeness"] == "complete" })
    );
    let evidence_id = skill_evidence_id(&database, snapshot, skill_id);
    let placement_ids = placements
        .iter()
        .map(|placement| placement["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    let current_fingerprint = placements[0]["content_digest"].as_str().unwrap();
    let upstream = "---\nname: external\nsource: fixture\nversion: v2\n---\nchanged\n";
    let upstream_digest = format!("{:x}", Sha256::digest(upstream.as_bytes()));
    let source_update = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id.clone()],
        "source_updates": [{
            "skill_id": skill_id,
            "placement_id": placement_ids[0],
            "source": "fixture",
            "current_revision": "v1",
            "current_fingerprint": current_fingerprint,
            "base_digest": null,
            "upstream_revision": "v2",
            "upstream_content": upstream,
            "upstream_digest": upstream_digest
        }]
    });
    let source_update = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&source_update.to_string()),
    );
    assert!(!source_update.status.success());
    assert!(
        String::from_utf8_lossy(&source_update.stdout).contains("read-only"),
        "{}",
        String::from_utf8_lossy(&source_update.stdout)
    );
    let library_change = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id.clone()],
        "library_changes": [{
            "skill_id": skill_id,
            "canonical_placement_id": placement_ids[0],
            "placement_ids": placement_ids,
            "requested_state": "managed"
        }]
    });
    let library_change = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&library_change.to_string()),
    );
    assert!(!library_change.status.success());
    assert!(String::from_utf8_lossy(&library_change.stdout).contains("read-only"));
    let roster_change = json!({
        "schema_version": 1,
        "scan_id": snapshot,
        "evidence_ids": [evidence_id],
        "roster_changes": [{
            "agent": "codex",
            "skill_id": skill_id,
            "state": "core"
        }]
    });
    let roster_change = run(
        &[&common[..], &["plan", "--stdin"]].concat(),
        Some(&roster_change.to_string()),
    );
    assert!(roster_change.status.success());
    let roster_change: Value = serde_json::from_slice(&roster_change.stdout).unwrap();
    assert_eq!(roster_change["result"]["files_changed"], false);
    assert_eq!(
        roster_change["result"]["change_summary"]["operation_count"],
        0
    );
    let plan_count: i64 = database
        .query_row("SELECT COUNT(*) FROM plans", [], |row| row.get(0))
        .unwrap();
    assert_eq!(plan_count, 1);

    let stale = run_suggested_action(confirm);
    assert!(!stale.status.success());
    let stale: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(stale["error"]["code"], "source_root_finding_stale");

    fs::remove_dir_all(&source).unwrap();
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: external\ndescription: replacement fixture\n---\n",
    )
    .unwrap();
    let drifted = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(
        drifted["result"]["source_root_policy"]["permissions"][0]["state"],
        "replaced"
    );
    let persisted_payload: String = database
        .query_row(
            "SELECT p.payload_json FROM scan_payloads p JOIN scans s ON s.id = p.scan_id WHERE s.status = 'completed' ORDER BY s.completed_at DESC, s.started_at DESC, s.rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let persisted_payload: Value = serde_json::from_str(&persisted_payload).unwrap();
    assert_ne!(
        persisted_payload["source_root_policy"][0]["state"],
        "active"
    );
    let drift_report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let drift_finding_id = drift_report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let drift_detail = json_output(&run(
        &[&common[..], &["report", "--finding", drift_finding_id]].concat(),
        None,
    ));
    let drift_confirm = drift_detail["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "confirm_source_root_read_permission")
        .unwrap();
    let identity_conflict = run_suggested_action(drift_confirm);
    assert!(!identity_conflict.status.success());
    let identity_conflict: Value = serde_json::from_slice(&identity_conflict.stdout).unwrap();
    assert_eq!(
        identity_conflict["error"]["code"],
        "source_root_permission_identity_drift"
    );
    assert_eq!(
        identity_conflict["error"]["details"]["permission_id"],
        permission_id
    );

    json_output(&run(
        &[
            &common[..],
            &["source-root", "revoke", permission_id.as_str()],
        ]
        .concat(),
        None,
    ));
    let reconfirmed = json_output(&run_suggested_action(drift_confirm));
    assert_eq!(reconfirmed["result"]["already_permitted"], false);
    permission_id = reconfirmed["result"]["permission"]["permission_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let inspect = json_output(&run(
        &[&common[..], &["source-root", "inspect"]].concat(),
        None,
    ));
    assert_eq!(inspect["result"]["permission_count"], 2);
    assert_eq!(inspect["result"]["active_count"], 1);
    assert_eq!(inspect["result"]["revoked_count"], 1);

    let revoked = json_output(&run(
        &[
            &common[..],
            &["source-root", "revoke", permission_id.as_str()],
        ]
        .concat(),
        None,
    ));
    assert_eq!(revoked["result"]["permission"]["state"], "revoked");
    assert_eq!(revoked["result"]["files_changed"], false);
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let blocked_again = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    assert!(
        blocked_again["result"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["title"] == "Skill links escape an approved root")
    );
}

#[cfg(unix)]
#[test]
fn revoked_durable_source_root_invalidates_snapshot_reads_immediately() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let source = temp.path().join("trusted-source");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: revocable-external\ndescription: revocation boundary fixture\n---\nprivate revocation boundary content\n",
    )
    .unwrap();
    let source = fs::canonicalize(source).unwrap();
    std::os::unix::fs::symlink(&source, root.join("revocable-external")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[
            &common[..],
            &[
                "--source-root",
                source.to_str().unwrap(),
                "report",
                "--finding",
                finding_id,
            ],
        ]
        .concat(),
        None,
    ));
    let confirm = detail["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "confirm_source_root_read_permission")
        .unwrap();
    let confirmed = json_output(&run_suggested_action(confirm));
    let permission_id = confirmed["result"]["permission"]["permission_id"]
        .as_str()
        .unwrap();

    let rescanned = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot_id = rescanned["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut legacy_payload: Value = serde_json::from_str(&payload).unwrap();
    assert!(
        legacy_payload
            .as_object_mut()
            .unwrap()
            .remove("durable_read_used_permission_ids")
            .is_some()
    );
    database
        .execute(
            "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
            [legacy_payload.to_string(), snapshot_id.to_owned()],
        )
        .unwrap();
    drop(database);
    let load_args = [
        "find",
        "revocation boundary",
        "--require-snapshot",
        snapshot_id,
        "--load",
        "--limit",
        "1",
    ];
    let loaded = json_output(&run(&[&common[..], &load_args].concat(), None));
    assert_eq!(
        loaded["result"]["loaded_skill"]["selection"]["name"],
        "revocable-external"
    );

    json_output(&run(
        &[&common[..], &["source-root", "revoke", permission_id]].concat(),
        None,
    ));
    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["state"], "rescan_required");
    assert_eq!(status["result"]["snapshot_state"], "rescan_required");
    assert_eq!(
        status["result"]["snapshot_invalidated_by_source_root_permission_ids"],
        json!([permission_id])
    );
    assert!(
        status["suggested_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["action"] == "scan"
                && action["reason_code"] == "source_root_permission_changed_requires_rescan")
    );

    let stale_load = run(&[&common[..], &load_args].concat(), None);
    assert!(!stale_load.status.success());
    let stale_stdout = stale_load.stdout;
    let stale_load: Value = serde_json::from_slice(&stale_stdout).unwrap();
    assert_eq!(
        stale_load["error"]["code"],
        "source_root_snapshot_rescan_required"
    );
    assert_eq!(
        stale_load["error"]["details"]["reason"],
        "source_root_permission_no_longer_active"
    );
    assert_eq!(
        stale_load["error"]["details"]["permission_ids"],
        json!([permission_id])
    );
    assert_eq!(stale_load["error"]["retryable"], true);
    assert_eq!(stale_load["error"]["details"]["permission_count"], 1);
    assert_eq!(
        stale_load["error"]["details"]["permission_ids_truncated"],
        false
    );
    assert_eq!(stale_load["error"]["details"]["files_changed"], false);
    assert_eq!(stale_load["error"]["details"]["state_files_changed"], false);
    assert_eq!(stale_load["error"]["details"]["snapshot_id"], snapshot_id);
    let scan_action = &stale_load["suggested_actions"][0];
    assert_eq!(scan_action["action"], "scan");
    assert_eq!(scan_action["mutates"], false);
    assert_eq!(scan_action["requires_confirmation"], false);
    let scan_argv = scan_action["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(scan_argv[0], env!("CARGO_BIN_EXE_skillroster"));
    assert!(
        scan_argv
            .windows(2)
            .any(|pair| pair == ["--home", home.to_str().unwrap()])
    );
    assert!(
        scan_argv
            .windows(2)
            .any(|pair| pair == ["--state-dir", state.to_str().unwrap()])
    );
    assert!(
        scan_argv
            .windows(2)
            .any(|pair| pair == ["scan", "--summary"])
    );
    assert!(
        !String::from_utf8_lossy(&stale_stdout).contains("private revocation boundary content")
    );
}

#[cfg(unix)]
#[test]
fn durable_source_root_identity_drift_invalidates_snapshot_reads_immediately() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let source = temp.path().join("trusted-source");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&source).unwrap();
    let skill_content = "---\nname: driftable-external\ndescription: identity drift boundary fixture\n---\nidentity drift private content\n";
    fs::write(source.join("SKILL.md"), skill_content).unwrap();
    let source = fs::canonicalize(source).unwrap();
    std::os::unix::fs::symlink(&source, root.join("driftable-external")).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[
            &common[..],
            &[
                "--source-root",
                source.to_str().unwrap(),
                "report",
                "--finding",
                finding_id,
            ],
        ]
        .concat(),
        None,
    ));
    let confirm = detail["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "confirm_source_root_read_permission")
        .unwrap();
    let confirmed = json_output(&run_suggested_action(confirm));
    let permission_id = confirmed["result"]["permission"]["permission_id"]
        .as_str()
        .unwrap();
    let rescanned = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let snapshot_id = rescanned["result"]["snapshot_id"].as_str().unwrap();

    let original = temp.path().join("original-source");
    fs::rename(&source, &original).unwrap();
    fs::create_dir(&source).unwrap();
    fs::write(source.join("SKILL.md"), skill_content).unwrap();

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["state"], "rescan_required");
    assert_eq!(status["result"]["snapshot_state"], "rescan_required");
    assert_eq!(
        status["result"]["snapshot_invalidated_by_source_root_permission_ids"],
        json!([permission_id])
    );
    let stale_load = run(
        &[
            &common[..],
            &[
                "find",
                "identity drift boundary",
                "--require-snapshot",
                snapshot_id,
                "--load",
                "--limit",
                "1",
            ],
        ]
        .concat(),
        None,
    );
    assert!(!stale_load.status.success());
    assert!(
        !String::from_utf8_lossy(&stale_load.stdout).contains("identity drift private content")
    );
    let stale_load: Value = serde_json::from_slice(&stale_load.stdout).unwrap();
    assert_eq!(
        stale_load["error"]["code"],
        "source_root_snapshot_rescan_required"
    );
    assert_eq!(
        stale_load["error"]["details"]["permission_ids"],
        json!([permission_id])
    );
}

#[cfg(unix)]
#[test]
fn revoking_an_unused_source_root_does_not_invalidate_the_snapshot() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    let source = temp.path().join("trusted-source");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(
        source.join("nested/SKILL.md"),
        "---\nname: unused-external\ndescription: unused permission fixture\n---\n",
    )
    .unwrap();
    let source = fs::canonicalize(source).unwrap();
    let linked = root.join("unused-external");
    std::os::unix::fs::symlink(&source, &linked).unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[
            &common[..],
            &[
                "--source-root",
                source.to_str().unwrap(),
                "report",
                "--finding",
                finding_id,
            ],
        ]
        .concat(),
        None,
    ));
    let confirm = detail["suggested_actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "confirm_source_root_read_permission")
        .unwrap();
    let confirmed = json_output(&run_suggested_action(confirm));
    let permission_id = confirmed["result"]["permission"]["permission_id"]
        .as_str()
        .unwrap();

    fs::remove_file(linked).unwrap();
    fs::remove_file(source.join("nested/SKILL.md")).unwrap();
    let rescanned = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(rescanned["result"]["skill_count"], 0);
    assert_eq!(
        rescanned["result"]["source_root_policy"]["permissions"][0]["state"],
        "active"
    );
    let snapshot_id = rescanned["result"]["snapshot_id"].as_str().unwrap();
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let payload: String = database
        .query_row(
            "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
            [snapshot_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut legacy_payload: Value = serde_json::from_str(&payload).unwrap();
    assert!(
        legacy_payload
            .as_object_mut()
            .unwrap()
            .remove("durable_read_used_permission_ids")
            .is_some()
    );
    database
        .execute(
            "UPDATE scan_payloads SET payload_json = ?1 WHERE scan_id = ?2",
            [legacy_payload.to_string(), snapshot_id.to_owned()],
        )
        .unwrap();
    drop(database);
    json_output(&run(
        &[&common[..], &["source-root", "revoke", permission_id]].concat(),
        None,
    ));

    let status = json_output(&run(&[&common[..], &["status"]].concat(), None));
    assert_eq!(status["result"]["snapshot_state"], "current");
    assert_eq!(status["result"]["state"], "report_required");
    assert_eq!(
        status["result"]["snapshot_invalidated_by_source_root_permission_ids"],
        json!([])
    );
}

#[cfg(unix)]
#[test]
fn unreadable_link_scan_preserves_a_retained_skill_identity() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".claude/skills");
    let source = temp.path().join("external-source");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: retained-external\ndescription: fixture\n---\n",
    )
    .unwrap();
    let source = fs::canonicalize(source).unwrap();
    let linked = root.join("retained-external");
    std::os::unix::fs::symlink(&source, &linked).unwrap();
    let entrypoint = linked.join("SKILL.md");
    let skill_id = format!(
        "skill_{:x}",
        Sha256::digest(format!("unreadable-link:{}", entrypoint.display()).as_bytes())
    );
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["status"]].concat(), None));
    let connection = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    connection
        .execute(
            "INSERT INTO skills
                (id, identity_key, name, description, declared_source, declared_revision,
                 content_digest, digest_version, governance_state, canonical_path)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, 1, 'managed', ?6)",
            rusqlite::params![
                skill_id,
                "content:retained-strong-identity",
                "retained-external",
                "Historical generic helper metadata",
                "retained-package-digest",
                linked.to_string_lossy(),
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO skills_fts (skill_id, name, description, triggers, body)
             VALUES (?1, 'retained-external', ?2, '', ?3)",
            rusqlite::params![
                skill_id,
                "Historical generic helper metadata",
                "instructions include phosphorescent telemetry reconciliation",
            ],
        )
        .unwrap();
    drop(connection);

    let first = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(first["result"]["skill_count"], 1);
    let second = json_output(&run(&[&common[..], &["scan"]].concat(), None));
    assert_eq!(second["result"]["skill_count"], 1);

    let historical_only = json_output(&run(
        &[
            &common[..],
            &["find", "phosphorescent telemetry reconciliation"],
        ]
        .concat(),
        None,
    ));
    assert_eq!(historical_only["result"]["matches"], json!([]));

    let report = json_output(&run(
        &[&common[..], &["report", "--summary"]].concat(),
        None,
    ));
    let finding_id = report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Skill links escape an approved root")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let detail = json_output(&run(
        &[&common[..], &["report", "--finding", finding_id]].concat(),
        None,
    ));
    assert_eq!(detail["result"]["severity"], "high");
    assert!(
        detail["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["facts"]["link_target"] == json!(source))
    );

    let load = run(
        &[&common[..], &["find", "retained-external", "--load"]].concat(),
        None,
    );
    assert!(!load.status.success());
    let load: Value = serde_json::from_slice(&load.stdout).unwrap();
    assert_eq!(
        load["error"]["details"]["reason"],
        "untrusted_external_source"
    );

    let connection = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    let retained: (String, String, String) = connection
        .query_row(
            "SELECT identity_key, governance_state, description FROM skills WHERE id = ?1",
            [&skill_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(retained.0, "content:retained-strong-identity");
    assert_eq!(retained.1, "managed");
    assert_eq!(retained.2, "Historical generic helper metadata");
    let current_index: (String, String) = connection
        .query_row(
            "SELECT description, body FROM skills_fts WHERE skill_id = ?1",
            [&skill_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(current_index, (String::new(), String::new()));
}

#[test]
fn report_distinguishes_inaccessible_and_missing_session_roots() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/sessions"), "not a directory").unwrap();
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
    ];
    let mut json_common = common.to_vec();
    json_common.push("--json");
    json_output(&run(&[&json_common[..], &["scan"]].concat(), None));

    let report = json_output(&run(
        &[&json_common[..], &["report", "--summary"]].concat(),
        None,
    ));
    assert_eq!(report["result"]["coverage_missing_agent_count"], 7);
    assert_eq!(report["result"]["coverage_inaccessible_agent_count"], 1);
    assert_eq!(
        report["result"]["session_coverage"]["missing_root_agents"],
        7
    );
    assert_eq!(
        report["result"]["session_coverage"]["inaccessible_agents"],
        1
    );
    let usage_findings = json_output(&run(
        &[
            &json_common[..],
            &["report", "--findings", "--category", "usage"],
        ]
        .concat(),
        None,
    ));
    let coverage_summary = usage_findings["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Usage coverage is incomplete")
        .unwrap()["summary"]
        .as_str()
        .unwrap();
    assert!(coverage_summary.contains("7 session roots are missing"));
    assert!(coverage_summary.contains("1 are inaccessible"));

    let human = run(&[&common[..], &["report", "--summary"]].concat(), None);
    assert!(human.status.success());
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("limited 0/8 · missing 7/8"), "{stdout}");
    assert!(stdout.contains("Inaccessible           1/8"), "{stdout}");
}

#[test]
fn historical_finding_detail_exposes_current_continuity_by_placement() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = home.join(".codex/skills");
    fs::create_dir_all(&root).unwrap();
    for index in 0..51 {
        let directory = root.join(format!("skill-{index:03}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: skill-{index:03}\n---\nfixture\n"),
        )
        .unwrap();
    }
    let common = [
        "--home",
        home.to_str().unwrap(),
        "--state-dir",
        state.to_str().unwrap(),
        "--json",
    ];
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let first_report = json_output(&run(&[&common[..], &["report", "--full"]].concat(), None));
    let historical_finding_id = first_report["result"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["title"] == "Large default Rosters need review")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let finding_detail = |extra: &[&str]| {
        let mut arguments = vec!["report", "--finding", historical_finding_id.as_str()];
        arguments.extend_from_slice(extra);
        json_output(&run(&[&common[..], &arguments].concat(), None))
    };
    let no_newer_report = finding_detail(&["--limit", "20"]);
    assert_eq!(
        no_newer_report["result"]["current_continuity"]["status"],
        "unavailable"
    );
    assert_eq!(
        no_newer_report["result"]["current_continuity"]["reason"],
        "no_newer_report"
    );
    let database = rusqlite::Connection::open(state.join("skillroster.db")).unwrap();
    database
        .execute(
            "UPDATE scan_payloads SET updated_at = ?1 WHERE scan_id = (SELECT scan_id FROM reports WHERE id = ?2)",
            rusqlite::params![i64::MAX, first_report["result"]["report_id"].as_str().unwrap()],
        )
        .unwrap();
    let missing_current_report = finding_detail(&["--limit", "20"]);
    assert_eq!(
        missing_current_report["result"]["current_continuity"]["status"],
        "unavailable"
    );
    assert_eq!(
        missing_current_report["result"]["current_continuity"]["reason"],
        "latest_report_unavailable"
    );
    database
        .execute(
            "UPDATE scan_payloads SET updated_at = 0 WHERE scan_id = (SELECT scan_id FROM reports WHERE id = ?1)",
            [first_report["result"]["report_id"].as_str().unwrap()],
        )
        .unwrap();
    fs::create_dir_all(root.join("skill-051")).unwrap();
    fs::write(
        root.join("skill-051/SKILL.md"),
        "---\nname: skill-051\n---\nfixture\n",
    )
    .unwrap();
    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let current_report = json_output(&run(&[&common[..], &["report", "--full"]].concat(), None));

    let compact = finding_detail(&["--limit", "20"]);
    let full_second_page = finding_detail(&["--full", "--limit", "20", "--offset", "20"]);
    let continuity = &compact["result"]["current_continuity"];
    assert_eq!(continuity["status"], "available");
    assert_eq!(continuity["basis"], "stable_placement_id_intersection");
    assert_eq!(continuity["missing_current_placement_count"], 0);
    assert_eq!(continuity["matching_placement_count"], 51);
    assert!(continuity["matching_finding_count"].as_u64().unwrap() >= 1);
    assert_eq!(
        continuity["current_placements"].as_array().unwrap().len(),
        20
    );
    assert!(continuity["current_placements"][0]["path"].is_string());
    assert!(continuity["current_placements"][0]["current_finding_ids"].is_array());
    assert_eq!(continuity["zero_overlap_is_not_resolution"], true);
    assert_eq!(
        full_second_page["result"]["current_continuity"]["matching_placement_count"],
        continuity["matching_placement_count"]
    );
    assert_eq!(
        full_second_page["result"]["current_continuity"]["matching_finding_count"],
        continuity["matching_finding_count"]
    );
    assert_eq!(
        full_second_page["result"]["current_continuity"]["current_finding_ids"],
        continuity["current_finding_ids"]
    );
    assert_eq!(
        full_second_page["result"]["current_continuity"]["current_placements"]
            .as_array()
            .unwrap()
            .len(),
        20
    );

    database
        .execute(
            "UPDATE reports SET summary_json = '{}' WHERE id = ?1",
            [current_report["result"]["report_id"].as_str().unwrap()],
        )
        .unwrap();
    let malformed = finding_detail(&["--limit", "20"]);
    assert_eq!(
        malformed["result"]["current_continuity"]["reason"],
        "latest_report_summary_malformed"
    );

    json_output(&run(&[&common[..], &["scan"]].concat(), None));
    let stale = finding_detail(&["--limit", "20"]);
    assert_eq!(
        stale["result"]["current_continuity"]["status"],
        "unavailable"
    );
    assert_eq!(
        stale["result"]["current_continuity"]["reason"],
        "latest_report_stale"
    );
}
