use serde_json::Value;
use std::{fs, process::Command};
use tempfile::TempDir;

fn run(temp: &TempDir, args: &[&str]) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_skillroster"))
        .arg("--home")
        .arg(temp.path().join("home"))
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    (
        output.status.success(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}

fn fixture(skills: &[(&str, &str)]) -> TempDir {
    let temp = TempDir::new().unwrap();
    for (name, description) in skills {
        let path = temp.path().join("home/.codex/skills").join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nPRIVATE_INSTRUCTIONS\n"),
        )
        .unwrap();
    }
    assert!(run(&temp, &["scan"]).0);
    temp
}

#[test]
fn english_without_hints_cannot_load_a_weak_generic_match() {
    let temp = fixture(&[(
        "general-helper",
        "Work with code and files for everyday tasks",
    )]);
    let (ok, blocked) = run(
        &temp,
        &["find", "--load", "--", "Please help me review code"],
    );
    assert!(!ok, "{blocked}");
    assert_eq!(blocked["error"]["code"], "verified_skill_load_blocked");
    assert_eq!(
        blocked["error"]["details"]["reason"],
        "direct_selection_evidence_required"
    );
    assert_eq!(
        blocked["error"]["details"]["retrieval"]["matches"][0]["name"],
        "general-helper"
    );
    assert!(blocked["error"]["details"]["retrieval"]["loaded_skill"].is_null());
    assert!(!blocked.to_string().contains("PRIVATE_INSTRUCTIONS"));
    assert_eq!(
        blocked["suggested_actions"][0]["action"],
        "inspect_ranked_skill_evidence"
    );
}

#[test]
fn precise_capability_hints_load_vendor_suffixed_names_with_aliases() {
    let temp = fixture(&[
        (
            "vendor-technical-writer-pro",
            "Write Chinese technical articles and explain architecture with illustrations",
        ),
        (
            "general-helper",
            "Work with code and files for everyday tasks",
        ),
    ]);
    for (task, hint) in [
        (
            "写一篇中文技术文章并解释架构",
            "zh tech article architecture",
        ),
        (
            "Please write a Chinese technical article about architecture",
            "",
        ),
    ] {
        let mut args = vec!["find", "--load"];
        if !hint.is_empty() {
            args.extend(["--hint", hint]);
        }
        args.extend(["--", task]);
        let (ok, loaded) = run(&temp, &args);
        assert!(ok, "{loaded}");
        assert_eq!(
            loaded["result"]["matches"][0]["name"],
            "vendor-technical-writer-pro"
        );
        assert!(
            loaded["result"]["loaded_skill"]
                .to_string()
                .contains("PRIVATE_INSTRUCTIONS")
        );
    }
    let (ok, unrelated) = run(
        &temp,
        &[
            "find",
            "--load",
            "--",
            "Please help me tune postgres vacuum",
        ],
    );
    assert!(!ok, "{unrelated}");
    assert!(!unrelated.to_string().contains("PRIVATE_INSTRUCTIONS"));
}

#[test]
fn pull_request_alias_reaches_original_metadata_and_preserves_collision_safety() {
    let temp = fixture(&[(
        "vendor-pr-review",
        "Review pull requests and inspect regressions before merging",
    )]);
    let (ok, loaded) = run(
        &temp,
        &[
            "find",
            "--hint",
            "PR review regressions",
            "--load",
            "--",
            "审查这次变更的回归",
        ],
    );
    assert!(ok, "{loaded}");
    let duplicate = temp.path().join("home/.hermes/skills/vendor-pr-review");
    fs::create_dir_all(&duplicate).unwrap();
    fs::write(duplicate.join("SKILL.md"), "---\nname: vendor-pr-review\ndescription: Review pull requests and inspect regressions before merging\n---\nPRIVATE_SECOND\n").unwrap();
    assert!(run(&temp, &["scan"]).0);
    let (ok, blocked) = run(
        &temp,
        &[
            "find",
            "--hint",
            "PR review regressions",
            "--load",
            "--",
            "审查这次变更的回归",
        ],
    );
    assert!(!ok, "{blocked}");
    assert_eq!(
        blocked["error"]["details"]["reason"],
        "same_name_variants_ambiguous"
    );
    assert_eq!(
        blocked["error"]["details"]["retrieval"]["matches"][0]["variant_count"],
        2
    );
    assert!(!blocked.to_string().contains("PRIVATE_"));
}

#[test]
fn blocked_retrieval_limits_candidate_preview_without_discarding_its_count() {
    let names = (0..12)
        .map(|n| format!("general-helper-{n:02}"))
        .collect::<Vec<_>>();
    let skills = names
        .iter()
        .map(|name| (name.as_str(), "Work with code and files for everyday tasks"))
        .collect::<Vec<_>>();
    let temp = fixture(&skills);
    let (ok, blocked) = run(
        &temp,
        &[
            "find",
            "--limit",
            "100",
            "--load",
            "--",
            "Please help me review code",
        ],
    );
    assert!(!ok, "{blocked}");
    let retrieval = &blocked["error"]["details"]["retrieval"];
    assert_eq!(retrieval["matches"].as_array().unwrap().len(), 10);
    assert_eq!(retrieval["returned_match_count"], 12);
    assert_eq!(retrieval["matches_truncated"], true);
    assert!(!blocked.to_string().contains("PRIVATE_INSTRUCTIONS"));
}

#[test]
fn ordinary_two_feature_hints_reach_vendor_and_variant_names() {
    let temp = fixture(&[
        (
            "gemini-frontend-design",
            "Design and build polished websites and web apps tailored to the brief",
        ),
        (
            "tech-essay-writer",
            "Write technical articles and explain engineering concepts",
        ),
        (
            "humanizer-zh",
            "Edit Chinese text to remove generated phrasing",
        ),
        (
            "github-pr-workflow",
            "Manage GitHub pull requests through review and merging",
        ),
    ]);
    for (hint, name) in [
        ("frontend design", "gemini-frontend-design"),
        ("technical article writing", "tech-essay-writer"),
        ("technical essay writer", "tech-essay-writer"),
        ("Chinese humanizer", "humanizer-zh"),
        ("GitHub pull request workflow", "github-pr-workflow"),
    ] {
        let (ok, loaded) = run(
            &temp,
            &["find", "--hint", hint, "--load", "--", "帮我完成这个工作"],
        );
        assert!(ok, "hint={hint}: {loaded}");
        assert_eq!(loaded["result"]["matches"][0]["name"], name, "hint={hint}");
    }
}

#[test]
fn domain_generic_tokens_and_unknown_skill_requests_cannot_authorize_load() {
    let temp = fixture(&[
        (
            "agent-workflow-helper",
            "An AI tool and skill for agent workflow tasks",
        ),
        ("technical-writer", "Write technical articles"),
    ]);
    for task in [
        "agent ai skill tool workflow",
        "use nonexistent xxx skill",
        "writer",
    ] {
        let (ok, blocked) = run(&temp, &["find", "--load", "--", task]);
        assert!(!ok, "task={task}: {blocked}");
        assert!(!blocked.to_string().contains("PRIVATE_INSTRUCTIONS"));
    }
}

#[test]
fn generic_exact_names_and_triggers_do_not_authorize_automatic_load() {
    for word in ["skill", "agent", "ai", "tool", "workflow", "writer"] {
        let temp = fixture(&[(word, "General local instructions")]);
        let (ok, blocked) = run(&temp, &["find", "--load", "--", word]);
        assert!(!ok, "{word}: {blocked}");
        assert_eq!(
            blocked["error"]["details"]["reason"],
            "direct_selection_evidence_required"
        );
        assert!(!blocked.to_string().contains("PRIVATE_INSTRUCTIONS"));
    }
    let temp = fixture(&[("specialized-operations", "General local instructions")]);
    fs::write(temp.path().join("home/.codex/skills/specialized-operations/SKILL.md"), "---\nname: specialized-operations\ndescription: General local instructions\ntriggers: [skill, workflow, deploy services]\n---\nPRIVATE_INSTRUCTIONS\n").unwrap();
    assert!(run(&temp, &["scan"]).0);
    let (ok, blocked) = run(
        &temp,
        &[
            "find",
            "--hint",
            "skill",
            "--load",
            "--",
            "帮我完成这个工作",
        ],
    );
    assert!(!ok, "{blocked}");
    assert!(!blocked.to_string().contains("PRIVATE_INSTRUCTIONS"));
    let (ok, specific) = run(
        &temp,
        &[
            "find",
            "--hint",
            "deploy services",
            "--load",
            "--",
            "workflow",
        ],
    );
    assert!(
        ok,
        "specific declared trigger must survive generic task-channel evidence: {specific}"
    );
}

#[test]
fn generic_name_overlap_cannot_outrank_multiple_specific_metadata_features() {
    let temp = fixture(&[
        (
            "agent-skills-manager",
            "Manage skills across AI agents including Codex and synchronize shared tool workflows",
        ),
        (
            "skill-installer",
            "Install Codex skills from a curated list or GitHub repository",
        ),
    ]);
    let (ok, loaded) = run(
        &temp,
        &[
            "find",
            "--hint",
            "install Codex curated skill",
            "--load",
            "--",
            "从官方清单安装一个技能",
        ],
    );
    assert!(ok, "{loaded}");
    assert_eq!(loaded["result"]["matches"][0]["name"], "skill-installer");
}

#[test]
fn fully_covered_specific_hints_outrank_unrelated_correlated_cjk_overlap() {
    for (task, hint, expected, description, weak_name, weak_description) in [
        (
            "设计产品落地页的布局配色和动效",
            "frontend layout typography motion",
            "gemini-frontend-design",
            "Design and build polished websites and web apps, covering layout, typography and motion",
            "x-post-writer",
            "发布产品动态并撰写社交内容",
        ),
        (
            "整理领域术语，补充词汇表和架构决策",
            "domain model glossary ADR",
            "domain-modeling",
            "Build a domain model, edit a glossary, and record an ADR",
            "humanizer-zh",
            "整理文字并改写中文内容",
        ),
        (
            "编写供助手执行的项目指引",
            "writing documents agents AGENTS.md",
            "writing-for-agents",
            "Writing documents for agents. Use when editing skills, AGENTS.md or CLAUDE.md",
            "patent-disclosure-skill",
            "编写发明说明和项目材料",
        ),
        (
            "从官方清单安装一个技能",
            "install Codex curated skill",
            "skill-installer",
            "Install Codex skills from a curated list or GitHub repo",
            "agent-skills-manager",
            "统一管理技能并维护共享目录",
        ),
    ] {
        let temp = fixture(&[(expected, description), (weak_name, weak_description)]);
        let weak_path = temp
            .path()
            .join("home/.codex/skills")
            .join(weak_name)
            .join("SKILL.md");
        let mut body = fs::read_to_string(&weak_path).unwrap();
        body.push_str(task);
        fs::write(weak_path, body).unwrap();
        assert!(run(&temp, &["scan"]).0);
        let (ok, loaded) = run(&temp, &["find", "--hint", hint, "--load", "--", task]);
        assert!(ok, "hint={hint}: {loaded}");
        assert_eq!(
            loaded["result"]["matches"][0]["name"], expected,
            "hint={hint}: {loaded}"
        );
        assert!(
            loaded["result"]["matches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|matched| matched["name"] == weak_name),
            "native candidate must remain inspectable: {loaded}"
        );
    }
}

#[test]
fn installer_stem_matches_precise_install_hint_over_multilingual_manager_overlap() {
    let temp = fixture(&[
        (
            "agent-skills-manager",
            "Manage skills across AI coding agents including Codex with a unified symlink architecture. Use when the user wants to install a new skill, migrate skills, or check skill distribution. Triggers include install skill, 安装 skill, and 统一管理 skill.",
        ),
        (
            "skill-installer",
            "Install Codex skills from a curated list or a GitHub repo path. Use when a user asks to install a curated skill or install a skill from another repo.",
        ),
    ]);
    for hint in [
        "install Codex curated skill",
        "installer Codex curated skill",
        "installation Codex curated skill",
        "installed Codex curated skill",
        "installing Codex curated skill",
    ] {
        let (ok, loaded) = run(
            &temp,
            &[
                "find",
                "--hint",
                hint,
                "--load",
                "--",
                "从官方清单安装一个技能",
            ],
        );
        assert!(ok, "hint={hint}: {loaded}");
        assert_eq!(
            loaded["result"]["matches"][0]["name"], "skill-installer",
            "hint={hint}: {loaded}"
        );
    }
}
