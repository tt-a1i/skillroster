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

/// The first-party Skill discovery behavior verified for an Agent adapter.
///
/// This is deliberately a small policy vocabulary: adapters whose loader has
/// not been verified retain the historical conservative recursive scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillDiscoverySemantics {
    Codex,
    ClaudeCode,
    Pi,
    Hermes,
    Conservative,
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

    pub const fn skill_discovery_semantics(self) -> SkillDiscoverySemantics {
        match self {
            Self::Codex => SkillDiscoverySemantics::Codex,
            Self::ClaudeCode => SkillDiscoverySemantics::ClaudeCode,
            Self::Pi => SkillDiscoverySemantics::Pi,
            Self::Hermes => SkillDiscoverySemantics::Hermes,
            Self::OpenCode | Self::Cursor | Self::GeminiCli | Self::GitHubCopilot => {
                SkillDiscoverySemantics::Conservative
            }
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
    pub event_index: usize,
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
    let mut next_event_index = 0;
    collect_observations(agent, &value, &mut next_event_index, &mut observations);
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

fn collect_observations(
    agent: AgentKind,
    value: &Value,
    next_event_index: &mut usize,
    output: &mut Vec<SessionObservation>,
) {
    match value {
        Value::Object(object) => {
            let event_index = *next_event_index;
            *next_event_index = next_event_index.saturating_add(1);
            if collect_object_observations(agent, object, event_index, output) {
                for child in object.values() {
                    collect_observations(agent, child, next_event_index, output);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_observations(agent, child, next_event_index, output);
            }
        }
        _ => {}
    }
}

fn collect_object_observations(
    agent: AgentKind,
    object: &serde_json::Map<String, Value>,
    event_index: usize,
    output: &mut Vec<SessionObservation>,
) -> bool {
    // Wrappers and ordinary message objects have no atomic Skill event. Avoid
    // cloning and serializing their entire conversation subtree before walking
    // the actual tool calls below them.
    let has_explicit_signal = [
        "outcome_skill",
        "applied_skill",
        "invoked_skill",
        "loaded_skill",
        "selected_skill",
        "matched_skill",
    ]
    .iter()
    .any(|field| object.contains_key(*field));
    let has_event_marker = [
        "type",
        "name",
        "event",
        "tool",
        "tool_name",
        "subtype",
        "event_type",
    ]
    .iter()
    .filter_map(|key| object.get(*key).and_then(Value::as_str))
    .any(|event| {
        let event = event.to_ascii_lowercase();
        matches!(
            event.as_str(),
            "tool_use" | "toolcall" | "custom_tool_call" | "function_call" | "function"
        ) || ["skill", "read", "load", "open"]
            .iter()
            .any(|marker| event.contains(marker))
    });
    let codex_user_message = agent == AgentKind::Codex
        && object.get("type").and_then(Value::as_str) == Some("message")
        && object.get("role").and_then(Value::as_str) == Some("user");
    if !has_explicit_signal && !has_event_marker && !codex_user_message {
        return true;
    }
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
                event_index,
                signal,
                record_text: record_text.clone(),
                explicit_references: vec![reference.to_owned()],
            });
        }
    }

    if let Some(observation) = structured_tool_observation(object, event_index, &record_text) {
        object_observations.push(observation);
        append_unique_observations(output, object_observations);
        return false;
    }
    if matches!(
        object
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("tool_use" | "toolcall" | "custom_tool_call" | "function_call")
    ) {
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
    let mut generic_references = ["skill_id", "skill_name"]
        .into_iter()
        .filter_map(|field| object.get(field).and_then(Value::as_str).map(str::to_owned))
        .collect::<Vec<_>>();
    generic_references.sort();
    generic_references.dedup();
    let serialized = record_text.to_ascii_lowercase();
    let reads_skill_file = serialized.contains("skill.md")
        && ["read_file", "read", "load", "open"]
            .iter()
            .any(|marker| event.contains(marker));
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
    } else if reads_skill_file || (!generic_references.is_empty() && event.contains("load_skill")) {
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
            event_index,
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
    event_index: usize,
    record_text: &str,
) -> Option<SessionObservation> {
    let call_type = object
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    let direct_call = matches!(
        call_type.as_deref(),
        Some("tool_use" | "toolcall" | "custom_tool_call" | "function_call")
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
    let explicit_references = argument_skill_references(arguments);
    let read_text = tool_read_text(&name, arguments);
    let signal = if name == "skill" && !explicit_references.is_empty() {
        SessionSignal::Applied
    } else if read_text.is_some() {
        SessionSignal::Loaded
    } else {
        return None;
    };
    Some(SessionObservation {
        event_index,
        signal,
        record_text: read_text.map_or_else(
            || record_text.to_owned(),
            |text| Value::String(text).to_string(),
        ),
        explicit_references,
    })
}

/// Only an actual read argument contributes a path. Other arguments, message
/// prose and the bodies of scripts that write fixtures are never read Evidence.
fn tool_read_text(name: &str, arguments: Option<&Value>) -> Option<String> {
    let parsed = arguments
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    let value = parsed.as_ref().or(arguments)?;
    let name = name.rsplit('.').next().unwrap_or(name);
    if matches!(name, "read" | "read_file" | "open" | "load" | "rg" | "grep") {
        let path = ["path", "file_path", "filePath"]
            .into_iter()
            .find_map(|key| value.get(key).and_then(Value::as_str))?;
        return path
            .to_ascii_lowercase()
            .contains("skill.md")
            .then(|| path.to_owned());
    }
    if !matches!(
        name,
        "bash" | "exec" | "exec_command" | "shell" | "terminal"
    ) {
        return None;
    }
    let commands = if let Some(command) = ["cmd", "command"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
    {
        vec![command.to_owned()]
    } else if let Some(input) = value.as_str() {
        if name == "exec" {
            literal_commands(input)
        } else {
            vec![input.to_owned()]
        }
    } else {
        Vec::new()
    };
    let reads = commands
        .iter()
        .flat_map(|command| {
            // A here-document body is data, even when it contains shell examples.
            if command.contains("<<") || command.contains(['$', '`', '#']) {
                return Vec::new();
            }
            command
                .split(['\n', ';', '&', '|'])
                .filter_map(|segment| {
                    let executable = segment.split_whitespace().next()?.rsplit('/').next()?;
                    (matches!(
                        executable,
                        "cat" | "sed" | "head" | "tail" | "rg" | "grep" | "awk" | "less"
                    ) && segment.to_ascii_lowercase().contains("skill.md"))
                    .then(|| segment.to_owned())
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    (!reads.is_empty()).then(|| reads.join("\n"))
}

/// Extract literal cmd/command properties from Codex's JavaScript exec wrapper.
/// Consume each quoted string once: a command's quoted fixture contents must
/// never become a second apparent command. Dynamic template strings are refused.
fn literal_commands(input: &str) -> Vec<String> {
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut commands = Vec::new();
    let mut command_value = false;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            index += bytes[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(bytes.len() - index);
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            index += 2;
            index += bytes[index..]
                .windows(2)
                .position(|part| part == b"*/")
                .map_or(bytes.len() - index, |end| end + 2);
            continue;
        }
        let quote = bytes[index];
        if matches!(quote, b'\'' | b'"' | b'`') {
            index += 1;
            let mut text = Vec::new();
            let mut closed = false;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if byte == quote {
                    closed = true;
                    break;
                }
                if byte == b'\\' && index < bytes.len() {
                    let escaped = bytes[index];
                    index += 1;
                    text.push(match escaped {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        other => other,
                    });
                } else {
                    text.push(byte);
                }
            }
            let text = String::from_utf8_lossy(&text).into_owned();
            let mut next = index;
            while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                next += 1;
            }
            if command_value
                && closed
                && !text.contains("${")
                && matches!(bytes.get(next), Some(b',' | b'}'))
            {
                commands.push(text.clone());
            }
            command_value =
                matches!(text.as_str(), "cmd" | "command") && bytes.get(next) == Some(&b':');
            if command_value {
                index = next + 1;
            }
            continue;
        }
        if quote.is_ascii_alphabetic() || quote == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let key = &input[start..index];
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            command_value = matches!(key, "cmd" | "command") && bytes.get(index) == Some(&b':');
            if command_value {
                index += 1;
            }
            continue;
        }
        if !quote.is_ascii_whitespace() {
            command_value = false;
        }
        index += 1;
    }
    commands
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
            skill_roots: vec![home.join(".codex/skills"), home.join(".agents/skills")],
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
    fn verified_adapters_have_explicit_discovery_semantics() {
        assert_eq!(
            AgentKind::Codex.skill_discovery_semantics(),
            SkillDiscoverySemantics::Codex
        );
        assert_eq!(
            AgentKind::ClaudeCode.skill_discovery_semantics(),
            SkillDiscoverySemantics::ClaudeCode
        );
        assert_eq!(
            AgentKind::Pi.skill_discovery_semantics(),
            SkillDiscoverySemantics::Pi
        );
        assert_eq!(
            AgentKind::Hermes.skill_discovery_semantics(),
            SkillDiscoverySemantics::Hermes
        );
        for agent in [
            AgentKind::OpenCode,
            AgentKind::Cursor,
            AgentKind::GeminiCli,
            AgentKind::GitHubCopilot,
        ] {
            assert_eq!(
                agent.skill_discovery_semantics(),
                SkillDiscoverySemantics::Conservative
            );
        }
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
            {"type": "load_skill", "skill_id": "research", "skill_name": "research", "loaded_skill": "research"},
            {"type": "load_skill", "skill_id": "research", "skill_name": "research", "loaded_skill": "research"}
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
            "skill_id": "research",
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
    fn codex_function_calls_scope_loaded_paths_to_literal_read_commands() {
        for name in ["exec_command", "functions.exec_command"] {
            let record = serde_json::json!({"type":"response_item", "payload":{
                "type":"function_call", "name":name,
                "arguments":serde_json::json!({"cmd":"cat /skills/research/SKILL.md && echo /skills/other/SKILL.md"}).to_string()
            }});
            let observations = session_record_observations(AgentKind::Codex, &record.to_string());
            assert_eq!(observations.len(), 1);
            assert_eq!(observations[0].signal, SessionSignal::Loaded);
            assert!(observations[0].record_text.contains("research/SKILL.md"));
            assert!(!observations[0].record_text.contains("other/SKILL.md"));
        }
    }

    #[test]
    fn fixture_writers_and_prompt_examples_are_not_loaded_reads() {
        for input in [
            "await tools.exec_command({cmd: \"python3 - <<'PY'\\nprint('cat /skills/research/SKILL.md')\\nPY\"})",
            "await tools.exec_command({cmd: \"printf '%s' 'cat /skills/research/SKILL.md' > fixture.json\"})",
            "// example: cmd: 'cat /skills/research/SKILL.md'\nawait tools.exec_command({cmd: 'true'})",
            "await tools.exec_command({cmd: `cat ${root}/skills/research/SKILL.md`})",
            "await tools.exec_command({cmd: 'cat /skills/research/SKILL.md' + '.fixture'})",
            "await tools.exec_command({cmd: 'cat fixture.json # /skills/research/SKILL.md'})",
        ] {
            let record = serde_json::json!({"type":"response_item", "payload":{"type":"custom_tool_call","name":"exec","input":input}});
            assert!(session_record_observations(AgentKind::Codex, &record.to_string()).is_empty());
        }
        let prompt = serde_json::json!({"type":"message","message":{"role":"user","content":[{"type":"text","text":"Example: cat /skills/research/SKILL.md"}]}});
        assert!(session_record_observations(AgentKind::Pi, &prompt.to_string()).is_empty());
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
