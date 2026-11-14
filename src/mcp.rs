//! Optional synchronous stdio MCP transport. All tool work uses the CLI read paths.
use std::io::{self, BufRead, Read, Write};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cli::{Cli, Command, FindArgs, MAX_FIND_RESULTS};

const PROTOCOL_VERSION: &str = "2025-11-25";
const MAX_FRAME_BYTES: u64 = 1024 * 1024;

pub fn serve(cli: Cli) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    let mut initialized = false;
    let mut ready = false;
    loop {
        let mut frame = Vec::new();
        if input
            .by_ref()
            .take(MAX_FRAME_BYTES + 1)
            .read_until(b'\n', &mut frame)?
            == 0
        {
            return Ok(());
        }
        let response = if frame.len() as u64 > MAX_FRAME_BYTES {
            // A single bounded failure, then clean exit instead of parsing a partial frame.
            write_frame(
                &mut output,
                &rpc_error(Value::Null, -32600, "MCP frame exceeds 1 MiB"),
            )?;
            return Ok(());
        } else {
            if frame.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match serde_json::from_slice::<Value>(&frame) {
                Ok(request) => dispatch(&cli, request, &mut initialized, &mut ready),
                Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
            }
        };
        if let Some(response) = response {
            if let Err(error) = write_frame(&mut output, &response) {
                if error.kind() == io::ErrorKind::BrokenPipe {
                    return Ok(());
                }
                return Err(error.into());
            }
        }
    }
}

fn write_frame(output: &mut impl Write, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message}})
}

fn dispatch(cli: &Cli, request: Value, initialized: &mut bool, ready: &mut bool) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request["method"].as_str();
    if request["jsonrpc"] != "2.0"
        || method.is_none()
        || id
            .as_ref()
            .is_some_and(|id| !(id.is_string() || id.is_i64() || id.is_u64()))
        || request.get("params").is_some_and(|p| !p.is_object())
    {
        return Some(rpc_error(Value::Null, -32600, "Invalid request"));
    }
    let method = method.unwrap();
    let Some(id) = id else {
        if method == "notifications/initialized" && *initialized {
            *ready = true;
        }
        return None;
    };
    let mut params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if let Some(params) = params.as_object_mut() {
        params.remove("_meta");
    }
    let result = match method {
        "initialize" => {
            if *initialized
                || !params["protocolVersion"].is_string()
                || !params["capabilities"].is_object()
                || !params["clientInfo"]["name"].is_string()
                || !params["clientInfo"]["version"].is_string()
            {
                return Some(rpc_error(
                    id,
                    -32602,
                    "Invalid initialize parameters or already initialized",
                ));
            }
            *initialized = true;
            let version = params["protocolVersion"].as_str().unwrap();
            let version = if ["2024-11-05", "2025-03-26", "2025-06-18", PROTOCOL_VERSION]
                .contains(&version)
            {
                version
            } else {
                PROTOCOL_VERSION
            };
            json!({
                "protocolVersion":version,
                "capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":"skillroster", "version":env!("CARGO_PKG_VERSION")},
                "instructions":"Local read-only Skill search and diagnostics. inspect_skills exposes planning.request_template on supported Findings; use CLI plan --stdin with schema_version:1, or plan --finding FINDING_ID [--core-limit N]. Shared adoption uses its separate template or plan --adopt-shared. Review the immutable Plan and obtain user confirmation before CLI apply PLAN_ID --json; never use init from an Agent. Failed load candidates remain in error.details.retrieval.matches; refine capability hints or choose a verified variant without treating ranking as authorization. No MCP tool mutates Agent files."
            })
        }
        "ping" => json!({}),
        _ if !*ready => {
            return Some(rpc_error(
                id,
                -32600,
                "Initialize and send notifications/initialized first",
            ));
        }
        "tools/list" => {
            if params["cursor"].is_null() {
                params.as_object_mut().unwrap().remove("cursor");
            }
            if !params.as_object().is_some_and(|p| p.is_empty()) {
                return Some(rpc_error(id, -32602, "tools/list takes no parameters"));
            }
            json!({"tools":tools()})
        }
        "tools/call" => match call_tool(cli, params) {
            Ok(result) => result,
            Err(message) => return Some(rpc_error(id, -32602, &message)),
        },
        _ => return Some(rpc_error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0", "id":id, "result":result}))
}

fn tools() -> Value {
    let annotations = json!({"readOnlyHint":true, "destructiveHint":false, "openWorldHint":false});
    json!([
        {
            "name":"find_skill",
            "description":"Search local Skills for the verbatim task. Use specific capability hints for cross-language retrieval; load=true returns complete SKILL.md only after path and fingerprint verification. Resolve ambiguity explicitly; retrieval is not authorization or proof of task success. Creates a read-only local Snapshot when needed.",
            "inputSchema":{
                "type":"object", "additionalProperties":false, "required":["task"],
                "properties":{
                    "task":{"type":"string", "minLength":1},
                    "hints":{"type":"array", "items":{"type":"string"}, "default":[]},
                    "load":{"type":"boolean", "default":false},
                    "limit":{"type":"integer", "minimum":1, "maximum":100, "default":10},
                    "variant_skill_id":{"type":"string"},
                    "variant_placement_id":{"type":"string"},
                    "require_snapshot":{"type":"string"}
                }
            }, "annotations":annotations
        },
        {
            "name":"inspect_skills",
            "description":"Read-only local Inventory, default-exposed metadata bytes and up to three review Findings. Refreshes missing or invalidated/configuration-drifted Snapshots. Counts describe observed local files, not runtime tokens or Skill usefulness.",
            "inputSchema":{"type":"object", "properties":{}, "additionalProperties":false},
            "annotations":annotations
        }
    ])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    name: String,
    #[serde(default = "empty_object")]
    arguments: Value,
}

fn empty_object() -> Value {
    json!({})
}
fn default_limit() -> u16 {
    10
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FindInput {
    task: String,
    #[serde(default)]
    hints: Vec<String>,
    #[serde(default)]
    load: bool,
    #[serde(default = "default_limit")]
    limit: u16,
    variant_skill_id: Option<String>,
    variant_placement_id: Option<String>,
    require_snapshot: Option<String>,
}

fn call_tool(cli: &Cli, params: Value) -> std::result::Result<Value, String> {
    let call: Call = serde_json::from_value(params).map_err(|e| e.to_string())?;
    // Validate every parameter before any local observation or state initialization.
    let find = match call.name.as_str() {
        "inspect_skills"
            if call.arguments.is_null()
                || call.arguments.as_object().is_some_and(|p| p.is_empty()) =>
        {
            None
        }
        "inspect_skills" => return Err("inspect_skills takes an empty arguments object".into()),
        "find_skill" => {
            let input: FindInput =
                serde_json::from_value(call.arguments).map_err(|e| e.to_string())?;
            if input.task.trim().is_empty()
                || !(1..=MAX_FIND_RESULTS).contains(&input.limit)
                || (!input.load
                    && (input.variant_skill_id.is_some() || input.variant_placement_id.is_some()))
            {
                return Err("task must be nonempty, limit must be 1..100, and variant selection requires load=true".into());
            }
            Some(FindArgs {
                task: input.task,
                hints: input.hints,
                load: input.load,
                limit: input.limit,
                variant_skill_id: input.variant_skill_id,
                variant_placement_id: input.variant_placement_id,
                require_snapshot: input.require_snapshot,
            })
        }
        _ => return Err("Unknown tool; available tools: find_skill, inspect_skills".into()),
    };
    let envelope = execute(cli, find.map_or(Command::Check, Command::Find));
    Ok(json!({
        "content":[{"type":"text", "text":envelope.to_string()}],
        "structuredContent":envelope,
        "isError":envelope["ok"] != true
    }))
}

fn execute(base: &Cli, command: Command) -> Value {
    let cli = Cli {
        json: true,
        state_dir: base.state_dir.clone(),
        home: base.home.clone(),
        roots: base.roots.clone(),
        source_roots: base.source_roots.clone(),
        command: Some(command),
    };
    let command_name = cli.command_name();
    let process = if matches!(cli.command, Some(Command::Find(_))) {
        crate::app::run_mcp_find_process(cli)
    } else {
        crate::app::run_process(cli)
    };
    let output = match process {
        Ok((_, Ok(output))) => output.json,
        Ok((context, Err(error))) => {
            crate::app::error_json_with_context(command_name, error.as_ref(), &context)
        }
        Err(error) => crate::app::error_json(command_name, error.as_ref()),
    };
    let mut envelope: Value = serde_json::from_str(&output).expect("CLI produces a JSON envelope");
    if command_name == "find"
        && envelope["ok"] == true
        && envelope["result"].get("scan_performed").is_none()
    {
        envelope["result"]["scan_performed"] = json!(false);
    }
    envelope
}
