use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
}

fn call(id: u64, name: &str, arguments: Value) -> Value {
    request(
        id,
        "tools/call",
        json!({"name":name, "arguments":arguments}),
    )
}

fn handshake() -> Vec<Value> {
    vec![
        request(
            1,
            "initialize",
            json!({"protocolVersion":"2025-11-25", "capabilities":{}, "clientInfo":{"name":"fixture", "version":"1"}}),
        ),
        json!({"jsonrpc":"2.0", "method":"notifications/initialized"}),
    ]
}

fn exchange(temp: &TempDir, requests: Vec<Value>) -> Vec<Value> {
    exchange_raw(
        temp,
        &requests
            .iter()
            .map(|r| format!("{r}\n"))
            .collect::<String>(),
    )
}

fn exchange_raw(temp: &TempDir, frames: &str) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .args([
            "--home",
            temp.path().join("home").to_str().unwrap(),
            "--state-dir",
            temp.path().join("state").to_str().unwrap(),
            "mcp",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(frames.as_bytes()).unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.contains(&27));
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout contains only JSON-RPC frames"))
        .collect()
}

fn fixture() -> TempDir {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("home/.codex/skills/review")).unwrap();
    fs::write(
        temp.path().join("home/.codex/skills/review/SKILL.md"),
        "---\nname: review\ndescription: Review source changes\n---\nReview the exact source.\n",
    )
    .unwrap();
    temp
}

#[test]
fn stdio_initialize_list_inspect_load_and_eof_keep_stdout_protocol_only() {
    let temp = fixture();
    let requests = [
        handshake(),
        vec![
            request(2, "tools/list", json!({})),
            call(3, "inspect_skills", json!({})),
            call(
                4,
                "find_skill",
                json!({"task":"review", "hints":["review source changes"], "load":true, "limit":1}),
            ),
            request(5, "ping", json!({})),
        ],
    ]
    .concat();
    let responses = exchange(&temp, requests);
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0]["result"]["protocolVersion"], "2025-11-25");
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["find_skill", "inspect_skills"]
    );
    assert!(
        tools
            .iter()
            .all(|t| t["annotations"]["readOnlyHint"] == true)
    );
    let inspection = &responses[2]["result"]["structuredContent"];
    assert_eq!(inspection["result"]["scan_performed"], true);
    assert!(
        inspection["result"]["exposed_metadata_footprint"]["exposed_metadata_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    let found = &responses[3]["result"]["structuredContent"];
    assert_eq!(found["ok"], true);
    assert_eq!(
        found["result"]["ranking_strategy"],
        "task_hint_reciprocal_rank_fusion"
    );
    assert_eq!(found["result"]["scan_performed"], false);
    assert_eq!(found["result"]["loaded_skill"]["content"]["complete"], true);
    assert_eq!(
        found["result"]["loaded_skill"]["content"]["text"],
        fs::read_to_string(temp.path().join("home/.codex/skills/review/SKILL.md")).unwrap()
    );
    let text: Value = serde_json::from_str(
        responses[3]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(&text, found);
    assert_eq!(responses[4]["result"], json!({}));
    let db = rusqlite::Connection::open(temp.path().join("state/skillroster.db")).unwrap();
    for table in ["plans", "receipts"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn same_name_loading_blocks_then_accepts_only_an_explicit_verified_variant() {
    let temp = fixture();
    fs::create_dir_all(temp.path().join("home/.claude/skills/review")).unwrap();
    fs::write(temp.path().join("home/.claude/skills/review/SKILL.md"), "---\nname: review\ndescription: Review source changes\n---\nDifferent review instructions.\n").unwrap();
    let responses = exchange(
        &temp,
        [
            handshake(),
            vec![
                call(2, "find_skill", json!({"task":"review", "load":true})),
                call(3, "find_skill", json!({"task":"review"})),
            ],
        ]
        .concat(),
    );
    assert_eq!(responses[1]["result"]["isError"], true);
    let blocked = &responses[1]["result"]["structuredContent"];
    assert_eq!(
        blocked["error"]["details"]["reason"],
        "same_name_variants_ambiguous"
    );
    assert!(blocked["result"].is_null());
    let found = &responses[2]["result"]["structuredContent"]["result"];
    let variants = found["matches"][0]["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 2);
    let selected = &variants[0]["skill_id"];
    let exact = exchange(&temp, [handshake(), vec![call(2, "find_skill", json!({"task":"review", "load":true, "variant_skill_id":selected, "require_snapshot":found["snapshot_id"]}))]].concat());
    assert_eq!(exact[1]["result"]["isError"], false);
    assert_eq!(
        &exact[1]["result"]["structuredContent"]["result"]["loaded_skill"]["selection"]["skill_id"],
        selected
    );
}

#[test]
fn invalid_arguments_and_write_tools_are_refused_without_local_state() {
    let temp = fixture();
    let mut requests = handshake();
    for (index, (name, arguments)) in [
        ("apply", json!({"plan_id":"anything"})),
        ("find_skill", json!({"task":"review", "limit":0})),
        ("find_skill", json!({"task":"review", "load":"yes"})),
        (
            "find_skill",
            json!({"task":"review", "variant_skill_id":"arbitrary"}),
        ),
        ("find_skill", json!({"task":"review", "unexpected":true})),
        ("find_skill", json!({"task":""})),
        ("inspect_skills", json!({"apply":true})),
    ]
    .into_iter()
    .enumerate()
    {
        requests.push(call(index as u64 + 2, name, arguments));
    }
    let responses = exchange(&temp, requests);
    assert!(responses[1..].iter().all(|r| r["error"]["code"] == -32602));
    assert!(!temp.path().join("state").exists());
    assert!(exchange(&temp, vec![]).is_empty());
    assert!(!temp.path().join("state").exists());
}

#[test]
fn preinitialize_requests_are_refused_and_notifications_never_get_replies() {
    let temp = fixture();
    let responses = exchange(
        &temp,
        vec![
            request(1, "tools/list", json!({})),
            json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":1}}),
            json!([request(2, "ping", json!({}))]),
        ],
    );
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], -32600);
    assert_eq!(responses[1]["error"]["code"], -32600);
    assert!(responses[1]["id"].is_null());
    assert!(!temp.path().join("state").exists());
}

#[test]
fn malformed_json_recovers_and_oversized_frames_stop_without_state_or_logs() {
    let temp = fixture();
    let responses = exchange_raw(
        &temp,
        "{bad json}\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n",
    );
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], -32700);
    assert_eq!(responses[1]["id"], 7);
    assert_eq!(responses[1]["result"], json!({}));
    let oversized = exchange_raw(&temp, &(" ".repeat(1024 * 1024 + 1) + "\n"));
    assert_eq!(oversized.len(), 1);
    assert_eq!(oversized[0]["error"]["code"], -32600);
    assert!(!temp.path().join("state").exists());
}

#[test]
fn sdk_nulls_and_whitespace_lines_are_accepted_without_extra_error_frames() {
    let temp = fixture();
    let requests = [
        handshake(),
        vec![
            request(2, "tools/list", json!({"cursor":null})),
            call(3, "inspect_skills", Value::Null),
            request(4, "tools/list", json!({"cursor":"unknown"})),
            call(5, "inspect_skills", json!([])),
        ],
    ]
    .concat();
    let wire = requests
        .iter()
        .map(|r| format!(" \t\r\n{r}\n\n\t \n"))
        .collect::<String>();
    let result = exchange_raw(&temp, &wire);
    assert_eq!(result.len(), 5);
    assert_eq!(result[1]["result"]["tools"].as_array().unwrap().len(), 2);
    assert_eq!(result[2]["result"]["isError"], false);
    assert_eq!(result[3]["error"]["code"], -32602);
    assert_eq!(result[4]["error"]["code"], -32602);
    assert!(result.iter().all(|r| !r["id"].is_null()));
}

#[test]
fn find_creates_only_the_needed_snapshot_and_never_requires_a_report() {
    let temp = fixture();
    let responses = exchange(
        &temp,
        [
            handshake(),
            vec![
                call(2, "find_skill", json!({"task":"review","load":true})),
                call(3, "find_skill", json!({"task":"review","load":true})),
            ],
        ]
        .concat(),
    );
    assert_eq!(
        responses[1]["result"]["structuredContent"]["result"]["scan_performed"],
        true
    );
    assert_eq!(
        responses[2]["result"]["structuredContent"]["result"]["scan_performed"],
        false
    );
    assert_eq!(responses[1]["result"]["isError"], false);
    assert_eq!(responses[2]["result"]["isError"], false);
    let db = rusqlite::Connection::open(temp.path().join("state/skillroster.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM scans", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM reports", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn required_snapshot_never_silently_scans_before_reading_or_reporting_drift() {
    let temp = fixture();
    let missing = exchange(
        &temp,
        [
            handshake(),
            vec![call(
                2,
                "find_skill",
                json!({"task":"review","load":true,"require_snapshot":"scan_missing"}),
            )],
        ]
        .concat(),
    );
    assert_eq!(
        missing[1]["result"]["structuredContent"]["error"]["code"],
        "snapshot_required"
    );
    let first = exchange(
        &temp,
        [
            handshake(),
            vec![call(2, "find_skill", json!({"task":"review","load":true}))],
        ]
        .concat(),
    );
    let id = first[1]["result"]["structuredContent"]["result"]["snapshot_id"].clone();
    fs::write(
        temp.path().join("home/.codex/config.toml"),
        "# changed observed configuration\n",
    )
    .unwrap();
    let guard = exchange(
        &temp,
        [
            handshake(),
            vec![call(
                2,
                "find_skill",
                json!({"task":"review","load":true,"require_snapshot":id}),
            )],
        ]
        .concat(),
    );
    assert_eq!(guard[1]["result"]["isError"], true);
    let db = rusqlite::Connection::open(temp.path().join("state/skillroster.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM scans", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM reports", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
    let fresh = exchange(
        &temp,
        [
            handshake(),
            vec![
                call(2, "find_skill", json!({"task":"review","load":true})),
                call(
                    3,
                    "find_skill",
                    json!({"task":"review","load":true,"require_snapshot":id}),
                ),
            ],
        ]
        .concat(),
    );
    assert_eq!(
        fresh[1]["result"]["structuredContent"]["result"]["scan_performed"],
        true
    );
    assert_eq!(
        fresh[2]["result"]["structuredContent"]["error"]["code"],
        "find_snapshot_changed"
    );
}
