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
    classify_value(&value, keys)
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

fn classify_value(value: &Value, event_keys: &[&str]) -> Option<SessionSignal> {
    let object = value.as_object()?;
    let event = event_keys
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
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
    .any(|key| object.contains_key(*key));
    let serialized = value.to_string().to_ascii_lowercase();
    let reads_skill_file = serialized.contains("skill.md")
        && ["read_file", "read", "load", "open"]
            .iter()
            .any(|marker| event.contains(marker));

    if explicit_skill_field
        && (object.contains_key("outcome_skill") || event.contains("skill_outcome"))
    {
        Some(SessionSignal::Outcome)
    } else if explicit_skill_field
        && (object.contains_key("applied_skill")
            || object.contains_key("invoked_skill")
            || event.contains("invoke_skill")
            || event.contains("apply_skill"))
    {
        Some(SessionSignal::Applied)
    } else if reads_skill_file
        || object.contains_key("loaded_skill")
        || (explicit_skill_field && event.contains("load_skill"))
    {
        Some(SessionSignal::Loaded)
    } else if object.contains_key("selected_skill")
        || object.contains_key("matched_skill")
        || (explicit_skill_field && event.contains("match_skill"))
    {
        Some(SessionSignal::Matched)
    } else {
        None
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
}
