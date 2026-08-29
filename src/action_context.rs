use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::cli::Cli;
use crate::model::SuggestedAction;

/// Discovery and state options that suggested actions retain so they operate
/// on the same local analysis context as the command that produced them.
#[derive(Clone, Debug)]
pub struct ActionContext {
    pub(crate) executable: String,
    pub(crate) argv: Vec<String>,
    pub(crate) argv_without_source_roots: Vec<String>,
}

impl Default for ActionContext {
    fn default() -> Self {
        Self {
            executable: "skillroster".into(),
            argv: Vec::new(),
            argv_without_source_roots: Vec::new(),
        }
    }
}

impl ActionContext {
    pub fn from_cli(cli: &Cli) -> Result<Self> {
        Self::from_cli_with_executable(cli, Path::new("skillroster"))
    }

    pub(crate) fn from_process(cli: &Cli) -> Result<Self> {
        let executable = std::env::current_exe().context("cannot resolve current executable")?;
        Self::from_cli_with_executable(cli, &executable)
    }

    fn from_cli_with_executable(cli: &Cli, executable: &Path) -> Result<Self> {
        let mut argv = Vec::new();
        if let Some(state_dir) = &cli.state_dir {
            let state_dir = if state_dir.is_absolute() {
                state_dir.clone()
            } else {
                std::path::absolute(state_dir).with_context(|| {
                    format!("cannot resolve --state-dir {}", state_dir.display())
                })?
            };
            argv.extend([
                "--state-dir".to_owned(),
                action_path(&state_dir, "--state-dir")?,
            ]);
        }
        if let Some(home) = &cli.home {
            argv.extend(["--home".to_owned(), action_path(home, "--home")?]);
        }
        for root in &cli.roots {
            argv.extend(["--root".to_owned(), root.clone()]);
        }
        let argv_without_source_roots = argv.clone();
        for source_root in &cli.source_roots {
            argv.extend([
                "--source-root".to_owned(),
                action_path(source_root, "--source-root")?,
            ]);
        }
        Ok(Self {
            executable: action_path(executable, "current executable")?,
            argv,
            argv_without_source_roots,
        })
    }

    pub(crate) fn apply(&self, actions: &mut [SuggestedAction]) {
        for action in actions {
            let context = if action.action == "scan"
                && action.reason_code == "source_root_permission_recorded"
            {
                &self.argv_without_source_roots
            } else {
                &self.argv
            };
            let reuses_temporary_source_roots = context.len()
                > self.argv_without_source_roots.len()
                && action.argv.get(1).map(String::as_str) == Some("scan");
            if reuses_temporary_source_roots {
                action.requires_confirmation = true;
            }
            let insertion =
                usize::from(action.argv.first().is_some_and(|arg| arg == "skillroster"));
            action.argv.splice(insertion..insertion, context.clone());
            if insertion == 1 {
                action.argv[0].clone_from(&self.executable);
            }
        }
    }

    pub(crate) fn command_argv_prefix(&self) -> Vec<String> {
        std::iter::once(self.executable.clone())
            .chain(self.argv.iter().cloned())
            .collect()
    }

    pub(crate) fn apply_json_argv(&self, argv: &mut Value) {
        apply_json_argv_context(argv, &self.argv, &self.executable);
    }

    pub(crate) fn bind_json_executable(&self, argv: &mut Value) {
        let Some(first) = argv.as_array_mut().and_then(|values| values.first_mut()) else {
            return;
        };
        if first.as_str() == Some("skillroster") {
            *first = Value::String(self.executable.clone());
        }
    }

    pub(crate) fn apply_result(&self, command: &str, result: &mut Value) {
        match command {
            "find" => {
                if let Some(matches) = result.get_mut("matches").and_then(Value::as_array_mut) {
                    for found in matches {
                        if let Some(argv) = found.pointer_mut("/variant_finding/argv") {
                            self.apply_json_argv(argv);
                        }
                    }
                }
            }
            "report" => {
                if let Some(action) = result.pointer_mut("/planning/action") {
                    self.apply_json_action(action);
                }
                for pointer in [
                    "/resolution/after_confirmation/argv_template",
                    "/resolution/permission_paths/temporary_one_scan/argv_template",
                ] {
                    if let Some(argv) = result.pointer_mut(pointer) {
                        self.apply_json_argv(argv);
                    }
                }
                if let Some(argv) = result.pointer_mut(
                    "/resolution/permission_paths/durable_permission/next/argv_template",
                ) {
                    apply_json_argv_context(
                        argv,
                        &self.argv_without_source_roots,
                        &self.executable,
                    );
                }
            }
            _ => {}
        }
    }

    fn apply_json_action(&self, action: &mut Value) {
        let uses_temporary_source_roots = self.argv.len() > self.argv_without_source_roots.len();
        let is_scan = action.get("action").and_then(Value::as_str) == Some("scan");
        if !is_scan {
            return;
        }
        if uses_temporary_source_roots {
            action["requires_confirmation"] = Value::Bool(true);
        }
        if let Some(argv) = action.get_mut("argv") {
            self.apply_json_argv(argv);
        }
    }
}

fn apply_json_argv_context(argv: &mut Value, context: &[String], executable: &str) {
    let Some(values) = argv.as_array_mut() else {
        return;
    };
    let insertion = usize::from(values.first().and_then(Value::as_str) == Some("skillroster"));
    values.splice(
        insertion..insertion,
        context.iter().cloned().map(Value::String),
    );
    if insertion == 1 {
        values[0] = Value::String(executable.to_owned());
    }
}

fn action_path(path: &Path, option: &str) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{option} path must be valid Unicode for suggested action argv"))
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use serde_json::json;

    use super::*;

    fn action(action: &str, reason_code: &str, argv: &[&str]) -> SuggestedAction {
        SuggestedAction {
            action: action.into(),
            description: String::new(),
            argv: argv.iter().map(|value| (*value).into()).collect(),
            mutates: false,
            requires_confirmation: false,
            reason_code: reason_code.into(),
        }
    }

    #[test]
    fn executable_binding_does_not_require_discovery_options() {
        let context = ActionContext {
            executable: "/verified/skillroster".into(),
            argv: Vec::new(),
            argv_without_source_roots: Vec::new(),
        };
        let mut actions = [action("scan", "retry_scan", &["skillroster", "scan"])];

        context.apply(&mut actions);

        assert_eq!(actions[0].argv, ["/verified/skillroster", "scan"]);
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_current_executable_fails_closed() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let cli = Cli::try_parse_from(["skillroster", "--json"]).unwrap();
        let executable = std::path::PathBuf::from(OsString::from_vec(vec![b'/', 0xff]));

        let error = ActionContext::from_cli_with_executable(&cli, &executable).unwrap_err();

        assert_eq!(
            error.to_string(),
            "current executable path must be valid Unicode for suggested action argv"
        );
    }

    #[test]
    fn temporary_source_roots_preserve_order_and_require_confirmation() {
        let context = ActionContext {
            executable: "skillroster".into(),
            argv: vec![
                "--home".into(),
                "/home".into(),
                "--source-root".into(),
                "/temporary".into(),
            ],
            argv_without_source_roots: vec!["--home".into(), "/home".into()],
        };
        let mut actions = [action("scan", "retry_scan", &["skillroster", "scan"])];

        context.apply(&mut actions);

        assert_eq!(
            actions[0].argv,
            [
                "skillroster",
                "--home",
                "/home",
                "--source-root",
                "/temporary",
                "scan"
            ]
        );
        assert!(actions[0].requires_confirmation);
    }

    #[test]
    fn only_recorded_permission_followup_drops_temporary_source_roots() {
        let context = ActionContext {
            executable: "skillroster".into(),
            argv: vec![
                "--home".into(),
                "/home".into(),
                "--source-root".into(),
                "/temporary".into(),
            ],
            argv_without_source_roots: vec!["--home".into(), "/home".into()],
        };
        let mut actions = [
            action(
                "scan",
                "source_root_permission_recorded",
                &["skillroster", "scan"],
            ),
            action(
                "scan",
                "source_root_permission_revoked",
                &["skillroster", "scan"],
            ),
            action(
                "scan_with_source_root_override",
                "source_root_permission_required",
                &["skillroster", "scan"],
            ),
        ];

        context.apply(&mut actions);

        assert_eq!(actions[0].argv, ["skillroster", "--home", "/home", "scan"]);
        for action in &actions[1..] {
            assert_eq!(
                action.argv,
                [
                    "skillroster",
                    "--home",
                    "/home",
                    "--source-root",
                    "/temporary",
                    "scan"
                ]
            );
        }
    }

    #[test]
    fn durable_permission_followup_drops_temporary_source_roots() {
        let context = ActionContext {
            executable: "skillroster".into(),
            argv: vec![
                "--home".into(),
                "/home".into(),
                "--source-root".into(),
                "/temporary".into(),
            ],
            argv_without_source_roots: vec!["--home".into(), "/home".into()],
        };
        let mut result = json!({
            "resolution": {"permission_paths": {"durable_permission": {
                "next": {"argv_template": ["skillroster", "source-root", "confirm"]}
            }}}
        });

        context.apply_result("report", &mut result);

        assert_eq!(
            result.pointer("/resolution/permission_paths/durable_permission/next/argv_template"),
            Some(&json!([
                "skillroster",
                "--home",
                "/home",
                "source-root",
                "confirm"
            ]))
        );
    }

    #[test]
    fn report_planning_scan_keeps_context_and_marks_temporary_roots() {
        let context = ActionContext {
            executable: "skillroster".into(),
            argv: vec![
                "--state-dir".into(),
                "/state".into(),
                "--source-root".into(),
                "/temporary".into(),
            ],
            argv_without_source_roots: vec!["--state-dir".into(), "/state".into()],
        };
        let mut result = json!({
            "planning": {"action": {
                "action": "scan",
                "argv": ["skillroster", "scan", "--summary", "--json"],
                "requires_confirmation": false
            }}
        });

        context.apply_result("report", &mut result);

        assert_eq!(
            result.pointer("/planning/action/argv"),
            Some(&json!([
                "skillroster",
                "--state-dir",
                "/state",
                "--source-root",
                "/temporary",
                "scan",
                "--summary",
                "--json"
            ]))
        );
        assert_eq!(
            result.pointer("/planning/action/requires_confirmation"),
            Some(&Value::Bool(true))
        );
    }
}
