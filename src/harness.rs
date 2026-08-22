use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Agents with a first-party filesystem layout understood by SkillRoster.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Codex,
    ClaudeCode,
    Pi,
    OpenCode,
    Hermes,
    Cursor,
    GeminiCli,
    GitHubCopilot,
}

impl AgentKind {
    pub const ALL: [Self; 8] = [
        Self::Codex,
        Self::ClaudeCode,
        Self::Pi,
        Self::OpenCode,
        Self::Hermes,
        Self::Cursor,
        Self::GeminiCli,
        Self::GitHubCopilot,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::Pi => "pi",
            Self::OpenCode => "opencode",
            Self::Hermes => "hermes",
            Self::Cursor => "cursor",
            Self::GeminiCli => "gemini-cli",
            Self::GitHubCopilot => "github-copilot",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
            Self::Pi => "Pi",
            Self::OpenCode => "OpenCode",
            Self::Hermes => "Hermes",
            Self::Cursor => "Cursor",
            Self::GeminiCli => "Gemini CLI",
            Self::GitHubCopilot => "GitHub Copilot",
        }
    }
}

/// Known local roots for one Agent. Missing roots are still reported by a Scan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentRoots {
    pub agent: AgentKind,
    pub skill_roots: Vec<PathBuf>,
    pub session_roots: Vec<PathBuf>,
}

/// Normalized evidence emitted by one adapter after inspecting a structured
/// session record. Adapters deliberately refuse to infer invocation merely
/// because prose contains words such as `result` or `executed`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionSignal {
    Matched,
    Loaded,
    Applied,
    Outcome,
}

/// One atomic usage event extracted from a structured session record.
///
/// Keeping the signal and references together prevents a stronger sibling
/// event from being attributed to every Skill mentioned by the parent record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionObservation {
    pub signal: SessionSignal,
    pub record_text: String,
    pub explicit_references: Vec<String>,
}

pub fn classify_session_record(agent: AgentKind, line: &str) -> Option<SessionSignal> {
    session_record_observations(agent, line)
        .into_iter()
        .map(|observation| observation.signal)
        .max_by_key(|signal| signal_rank(*signal))
}

pub fn session_record_observations(agent: AgentKind, record: &str) -> Vec<SessionObservation> {
    let Ok(value) = serde_json::from_str::<Value>(record) else {
        return Vec::new();
    };
    let mut observations = Vec::new();
    collect_observations(agent, &value, &mut observations);
    observations
}

const fn signal_rank(signal: SessionSignal) -> u8 {
    match signal {
        SessionSignal::Matched => 0,
        SessionSignal::Loaded => 1,
        SessionSignal::Applied => 2,
        SessionSignal::Outcome => 3,
    }
}

fn adapter_event_keys(agent: AgentKind) -> &'static [&'static str] {
    match agent {
        AgentKind::Codex => &["type", "event_type", "tool_name"],
        AgentKind::ClaudeCode => &["type", "subtype", "tool_name"],
        AgentKind::Pi => &["type", "event", "tool"],
        AgentKind::OpenCode => &["type", "event", "tool"],
        AgentKind::Hermes => &["type", "event", "tool_name"],
        AgentKind::Cursor => &["type", "event", "tool"],
        AgentKind::GeminiCli => &["type", "event", "tool_name"],
        AgentKind::GitHubCopilot => &["type", "event", "tool_name"],
    }
}

fn collect_observations(agent: AgentKind, value: &Value, output: &mut Vec<SessionObservation>) {
    match value {
        Value::Object(object) => {
            if collect_object_observations(agent, object, output) {
                for child in object.values() {
                    collect_observations(agent, child, output);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_observations(agent, child, output);
            }
        }
        _ => {}
    }
}

fn collect_object_observations(
    agent: AgentKind,
    object: &serde_json::Map<String, Value>,
    output: &mut Vec<SessionObservation>,
) -> bool {
    let record_text = Value::Object(object.clone()).to_string();
    let mut object_observations = Vec::new();
    for (field, signal) in [
        ("outcome_skill", SessionSignal::Outcome),
        ("applied_skill", SessionSignal::Applied),
        ("invoked_skill", SessionSignal::Applied),
        ("loaded_skill", SessionSignal::Loaded),
        ("selected_skill", SessionSignal::Matched),
        ("matched_skill", SessionSignal::Matched),
    ] {
        if let Some(reference) = object.get(field).and_then(Value::as_str) {
            object_observations.push(SessionObservation {
                signal,
                record_text: record_text.clone(),
                explicit_references: vec![reference.to_owned()],
            });
        }
    }

    if let Some(observation) = structured_tool_observation(object, &record_text) {
        object_observations.push(observation);
        append_unique_observations(output, object_observations);
        return false;
    }
    if object.get("type").and_then(Value::as_str) == Some("function")
        && object
            .get("function")
            .and_then(Value::as_object)
            .is_some_and(|function| !function.contains_key("arguments"))
    {
        append_unique_observations(output, object_observations);
        return false;
    }

    let event_keys = adapter_event_keys(agent);
    let mut event_parts = Vec::new();
    for key in event_keys {
        event_parts.push(object.get(*key).and_then(Value::as_str));
    }
    event_parts.extend(
        ["type", "name", "event", "tool", "tool_name", "subtype"]
            .into_iter()
            .map(|key| object.get(key).and_then(Value::as_str)),
    );
    let event = event_parts
        .into_iter()
        .flatten()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    let generic_references = ["skill_id", "skill_name"]
        .into_iter()
        .filter_map(|field| object.get(field).and_then(Value::as_str).map(str::to_owned))
        .collect::<Vec<_>>();
    let serialized = record_text.to_ascii_lowercase();
    let reads_skill_file = serialized.contains("skill.md")
        && ["read_file", "read", "load", "open"]
            .iter()
            .any(|marker| event.contains(marker));
    let codex_shell_read = agent == AgentKind::Codex
        && object.get("type").and_then(Value::as_str) == Some("custom_tool_call")
        && object.get("name").and_then(Value::as_str) == Some("exec")
        && object
            .get("input")
            .and_then(Value::as_str)
            .is_some_and(|input| {
                let input = input.to_ascii_lowercase();
                input.contains("skill.md")
                    && input.contains("exec_command")
                    && ["sed ", "rg ", "head ", "tail ", "awk ", "grep ", "less "]
                        .iter()
                        .any(|marker| input.contains(marker))
            });
    let codex_user_skill_reference = agent == AgentKind::Codex
        && object.get("type").and_then(Value::as_str) == Some("message")
        && object.get("role").and_then(Value::as_str) == Some("user")
        && serialized.contains("skill.md");

    let signal = if !generic_references.is_empty() && event.contains("skill_outcome") {
        Some(SessionSignal::Outcome)
    } else if !generic_references.is_empty()
        && (event.contains("invoke_skill") || event.contains("apply_skill"))
    {
        Some(SessionSignal::Applied)
    } else if reads_skill_file
        || codex_shell_read
        || (!generic_references.is_empty() && event.contains("load_skill"))
    {
        Some(SessionSignal::Loaded)
    } else if codex_user_skill_reference
        || (!generic_references.is_empty() && event.contains("match_skill"))
    {
        Some(SessionSignal::Matched)
    } else {
        None
    };
    if let Some(signal) = signal {
        object_observations.push(SessionObservation {
            signal,
            record_text,
            explicit_references: generic_references,
        });
    }
    append_unique_observations(output, object_observations);
    true
}

fn append_unique_observations(
    output: &mut Vec<SessionObservation>,
    observations: Vec<SessionObservation>,
) {
    let mut unique = Vec::new();
    for observation in observations {
        if !unique.contains(&observation) {
            unique.push(observation);
        }
    }
    output.extend(unique);
}

fn structured_tool_observation(
    object: &serde_json::Map<String, Value>,
    record_text: &str,
) -> Option<SessionObservation> {
    let call_type = object
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    let direct_call = matches!(
        call_type.as_deref(),
        Some("tool_use" | "toolcall" | "custom_tool_call")
    );
    let function = object.get("function").and_then(Value::as_object);
    let function_call = call_type.as_deref() == Some("function")
        && function.is_some_and(|function| function.contains_key("arguments"));
    if !direct_call && !function_call {
        return None;
    }
    let name = if direct_call {
        object.get("name").and_then(Value::as_str)
    } else {
        function
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
    }?
    .to_ascii_lowercase();
    let arguments = if direct_call {
        object.get("input").or_else(|| object.get("arguments"))
    } else {
        function.and_then(|function| function.get("arguments"))
    };
    let serialized = arguments
        .map(Value::to_string)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let explicit_references = argument_skill_references(arguments);
    let reads_skill_file = serialized.contains("skill.md")
        && (matches!(
            name.as_str(),
            "read" | "read_file" | "open" | "load" | "rg" | "grep"
        ) || (matches!(name.as_str(), "bash" | "exec" | "shell" | "terminal")
            && ["cat ", "sed ", "rg ", "grep ", "head ", "tail ", "less "]
                .iter()
                .any(|marker| serialized.contains(marker))));
    let signal = if name == "skill" && !explicit_references.is_empty() {
        SessionSignal::Applied
    } else if reads_skill_file {
        SessionSignal::Loaded
    } else {
        return None;
    };
    Some(SessionObservation {
        signal,
        record_text: record_text.to_owned(),
        explicit_references,
    })
}

fn argument_skill_references(arguments: Option<&Value>) -> Vec<String> {
    let parsed;
    let value = match arguments {
        Some(Value::String(text)) => {
            parsed = serde_json::from_str::<Value>(text).ok();
            parsed.as_ref()
        }
        value => value,
    };
    value
        .and_then(Value::as_object)
        .and_then(|arguments| arguments.get("skill"))
        .and_then(Value::as_str)
        .filter(|skill| !skill.trim().is_empty())
        .map(|skill| vec![skill.to_owned()])
        .unwrap_or_default()
}

/// Return the conservative, documented layouts SkillRoster knows about.
///
/// Project-local roots are intentionally absent: callers may add them through
/// `ScanOptions::explicit_skill_roots` after resolving the intended project.
pub fn known_agent_roots(home: &Path) -> Vec<AgentRoots> {
    let config = home.join(".config");
    let local_share = home.join(".local/share");

    vec![
        AgentRoots {
            agent: AgentKind::Codex,
            skill_roots: vec![home.join(".codex/skills")],
            session_roots: vec![home.join(".codex/sessions")],
        },
        AgentRoots {
            agent: AgentKind::ClaudeCode,
            skill_roots: vec![home.join(".claude/skills")],
            session_roots: vec![home.join(".claude/projects")],
        },
        AgentRoots {
            agent: AgentKind::Pi,
            skill_roots: vec![home.join(".pi/agent/skills")],
            session_roots: vec![home.join(".pi/agent/sessions")],
        },
        AgentRoots {
            agent: AgentKind::OpenCode,
            skill_roots: vec![config.join("opencode/skills")],
            session_roots: vec![local_share.join("opencode/storage/session")],
        },
        AgentRoots {
            agent: AgentKind::Hermes,
            skill_roots: vec![home.join(".hermes/skills")],
            session_roots: vec![home.join(".hermes/sessions")],
        },
        AgentRoots {
            agent: AgentKind::Cursor,
            skill_roots: vec![home.join(".cursor/skills")],
            session_roots: vec![home.join(".cursor/projects")],
        },
        AgentRoots {
            agent: AgentKind::GeminiCli,
            skill_roots: vec![home.join(".gemini/skills")],
            session_roots: vec![home.join(".gemini/tmp")],
        },
        AgentRoots {
            agent: AgentKind::GitHubCopilot,
            skill_roots: vec![home.join(".copilot/skills")],
            session_roots: vec![home.join(".copilot/session-state")],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_exactly_the_eight_supported_agents() {
        let roots = known_agent_roots(Path::new("/home/tester"));
        assert_eq!(roots.len(), 8);
        assert_eq!(AgentKind::ALL.len(), 8);
        assert!(roots.iter().all(|roots| !roots.skill_roots.is_empty()));
        assert!(roots.iter().all(|roots| !roots.session_roots.is_empty()));
    }

    #[test]
    fn generic_result_text_is_not_claimed_as_skill_application() {
        let line = r#"{"type":"tool_result","text":"research completed"}"#;
        assert_eq!(classify_session_record(AgentKind::Codex, line), None);
    }

    #[test]
    fn explicit_structured_skill_events_are_normalized() {
        assert_eq!(
            classify_session_record(
                AgentKind::ClaudeCode,
                r#"{"type":"invoke_skill","invoked_skill":"research"}"#
            ),
            Some(SessionSignal::Applied)
        );
    }

    #[test]
    fn sibling_skill_events_keep_their_own_stage_and_reference() {
        let record = serde_json::json!({
            "events": [
                {"selected_skill": "research"},
                {"applied_skill": "review"}
            ]
        })
        .to_string();
        let observations = session_record_observations(AgentKind::ClaudeCode, &record);
        assert!(observations.iter().any(|observation| {
            observation.signal == SessionSignal::Matched
                && observation.explicit_references == ["research"]
        }));
        assert!(observations.iter().any(|observation| {
            observation.signal == SessionSignal::Applied
                && observation.explicit_references == ["review"]
        }));
    }

    #[test]
    fn identical_sibling_events_remain_distinct_observations() {
        let record = serde_json::json!([
            {"type": "load_skill", "skill_name": "research", "loaded_skill": "research"},
            {"type": "load_skill", "skill_name": "research", "loaded_skill": "research"}
        ])
        .to_string();
        let observations = session_record_observations(AgentKind::Hermes, &record);
        assert_eq!(
            observations
                .iter()
                .filter(|observation| observation.signal == SessionSignal::Loaded)
                .count(),
            2
        );
    }

    #[test]
    fn overlapping_schemas_count_one_object_once() {
        let record = serde_json::json!({
            "type": "load_skill",
            "skill_name": "research",
            "loaded_skill": "research"
        })
        .to_string();
        let observations = session_record_observations(AgentKind::Hermes, &record);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].signal, SessionSignal::Loaded);
        assert_eq!(observations[0].explicit_references, ["research"]);
    }

    #[test]
    fn codex_nested_exec_read_is_loaded_evidence() {
        let line = serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "name": "exec",
                "input": "await tools.exec_command({cmd: \"sed -n '1,80p' /skills/research/SKILL.md\"})"
            }
        })
        .to_string();
        assert_eq!(
            classify_session_record(AgentKind::Codex, &line),
            Some(SessionSignal::Loaded)
        );
    }

    #[test]
    fn codex_user_skill_link_is_matched_but_catalog_metadata_is_not() {
        let invoked = serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": "[$research](/skills/research/SKILL.md)"
                }]
            }
        })
        .to_string();
        assert_eq!(
            classify_session_record(AgentKind::Codex, &invoked),
            Some(SessionSignal::Matched)
        );

        let catalog = serde_json::json!({
            "type": "session_meta",
            "payload": {
                "base_instructions": "research is stored at /skills/research/SKILL.md"
            }
        })
        .to_string();
        assert_eq!(classify_session_record(AgentKind::Codex, &catalog), None);
    }

    #[test]
    fn nested_agent_tool_calls_are_normalized_without_reading_message_prose() {
        let claude = serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "Skill",
                    "input": {"skill": "research", "args": ""}
                }]
            }
        });
        assert_eq!(
            classify_session_record(AgentKind::ClaudeCode, &claude.to_string()),
            Some(SessionSignal::Applied)
        );

        let pi = serde_json::json!({
            "type": "message",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "toolCall",
                    "name": "read",
                    "arguments": {"path": "/skills/research/SKILL.md"}
                }]
            }
        });
        assert_eq!(
            classify_session_record(AgentKind::Pi, &pi.to_string()),
            Some(SessionSignal::Loaded)
        );

        let cursor = serde_json::json!({
            "role": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "Read",
                    "input": {"path": "/skills/review/SKILL.md"}
                }]
            }
        });
        assert_eq!(
            classify_session_record(AgentKind::Cursor, &cursor.to_string()),
            Some(SessionSignal::Loaded)
        );

        let hermes = serde_json::json!({
            "request": {"body": {"messages": [{
                "role": "assistant",
                "tool_calls": [{
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "arguments": "{\"path\":\"/skills/research/SKILL.md\"}"
                    }
                }]
            }]}}
        });
        assert_eq!(
            classify_session_record(AgentKind::Hermes, &hermes.to_string()),
            Some(SessionSignal::Loaded)
        );
    }

    #[test]
    fn tool_declarations_and_skill_catalogs_are_not_usage_events() {
        let declaration = serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a SKILL.md file",
                "parameters": {"type": "object"}
            }
        });
        assert_eq!(
            classify_session_record(AgentKind::Hermes, &declaration.to_string()),
            None
        );
    }
}
