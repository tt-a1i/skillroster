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

pub fn classify_session_record(agent: AgentKind, line: &str) -> Option<SessionSignal> {
    let value: Value = serde_json::from_str(line).ok()?;
    let keys = adapter_event_keys(agent);
    classify_value(agent, &value, keys)
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

fn classify_value(agent: AgentKind, value: &Value, event_keys: &[&str]) -> Option<SessionSignal> {
    if let Some(signal) = structured_session_signal(value) {
        return Some(signal);
    }
    let object = value.as_object()?;
    let record_type = object.get("type").and_then(Value::as_str);
    let payload = object.get("payload").and_then(Value::as_object);
    let active = if matches!(record_type, Some("response_item" | "event_msg")) {
        payload.unwrap_or(object)
    } else {
        object
    };
    let mut event_parts = Vec::new();
    for key in event_keys {
        event_parts.extend([
            object.get(*key).and_then(Value::as_str),
            active.get(*key).and_then(Value::as_str),
        ]);
    }
    event_parts.extend(
        ["type", "name", "event", "tool", "tool_name", "subtype"]
            .into_iter()
            .map(|key| active.get(key).and_then(Value::as_str)),
    );
    let event = event_parts
        .into_iter()
        .flatten()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    let contains_field = |key: &str| object.contains_key(key) || active.contains_key(key);
    let explicit_skill_field = [
        "skill_id",
        "skill_name",
        "selected_skill",
        "matched_skill",
        "loaded_skill",
        "applied_skill",
        "invoked_skill",
        "outcome_skill",
    ]
    .iter()
    .any(|key| contains_field(key));
    let serialized = Value::Object(active.clone())
        .to_string()
        .to_ascii_lowercase();
    let reads_skill_file = serialized.contains("skill.md")
        && ["read_file", "read", "load", "open"]
            .iter()
            .any(|marker| event.contains(marker));
    let codex_shell_read = agent == AgentKind::Codex
        && active.get("type").and_then(Value::as_str) == Some("custom_tool_call")
        && active.get("name").and_then(Value::as_str) == Some("exec")
        && active
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
        && record_type == Some("response_item")
        && active.get("type").and_then(Value::as_str) == Some("message")
        && active.get("role").and_then(Value::as_str) == Some("user")
        && serialized.contains("skill.md");

    if explicit_skill_field && (contains_field("outcome_skill") || event.contains("skill_outcome"))
    {
        Some(SessionSignal::Outcome)
    } else if explicit_skill_field
        && (contains_field("applied_skill")
            || contains_field("invoked_skill")
            || event.contains("invoke_skill")
            || event.contains("apply_skill"))
    {
        Some(SessionSignal::Applied)
    } else if reads_skill_file
        || codex_shell_read
        || contains_field("loaded_skill")
        || (explicit_skill_field && event.contains("load_skill"))
    {
        Some(SessionSignal::Loaded)
    } else if codex_user_skill_reference
        || contains_field("selected_skill")
        || contains_field("matched_skill")
        || (explicit_skill_field && event.contains("match_skill"))
    {
        Some(SessionSignal::Matched)
    } else {
        None
    }
}

fn structured_session_signal(value: &Value) -> Option<SessionSignal> {
    fn stronger(
        left: Option<SessionSignal>,
        right: Option<SessionSignal>,
    ) -> Option<SessionSignal> {
        let rank = |signal: SessionSignal| match signal {
            SessionSignal::Matched => 0,
            SessionSignal::Loaded => 1,
            SessionSignal::Applied => 2,
            SessionSignal::Outcome => 3,
        };
        match (left, right) {
            (Some(left), Some(right)) if rank(left) >= rank(right) => Some(left),
            (_, Some(right)) => Some(right),
            (left, None) => left,
        }
    }

    fn object_signal(object: &serde_json::Map<String, Value>) -> Option<SessionSignal> {
        for (field, signal) in [
            ("outcome_skill", SessionSignal::Outcome),
            ("applied_skill", SessionSignal::Applied),
            ("invoked_skill", SessionSignal::Applied),
            ("loaded_skill", SessionSignal::Loaded),
            ("selected_skill", SessionSignal::Matched),
            ("matched_skill", SessionSignal::Matched),
        ] {
            if object.get(field).and_then(Value::as_str).is_some() {
                return Some(signal);
            }
        }

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
        if name == "skill"
            && arguments
                .and_then(Value::as_object)
                .and_then(|arguments| arguments.get("skill"))
                .and_then(Value::as_str)
                .is_some_and(|skill| !skill.trim().is_empty())
        {
            return Some(SessionSignal::Applied);
        }
        if !serialized.contains("skill.md") {
            return None;
        }
        if matches!(
            name.as_str(),
            "read" | "read_file" | "open" | "load" | "rg" | "grep"
        ) {
            return Some(SessionSignal::Loaded);
        }
        if matches!(name.as_str(), "bash" | "exec" | "shell" | "terminal")
            && ["cat ", "sed ", "rg ", "grep ", "head ", "tail ", "less "]
                .iter()
                .any(|marker| serialized.contains(marker))
        {
            return Some(SessionSignal::Loaded);
        }
        None
    }

    match value {
        Value::Object(object) => object
            .values()
            .fold(object_signal(object), |signal, child| {
                stronger(signal, structured_session_signal(child))
            }),
        Value::Array(values) => values.iter().fold(None, |signal, child| {
            stronger(signal, structured_session_signal(child))
        }),
        _ => None,
    }
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
