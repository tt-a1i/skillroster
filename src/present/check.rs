use super::*;

pub(super) fn render(value: &Value, lines: &mut Vec<String>, width: usize) {
    fact(
        lines,
        "Snapshot",
        if value["scan_performed"] == true {
            "scanned now"
        } else {
            "reused"
        },
    );
    fact(lines, "Independent Skills", text(value, "skill_count"));
    fact(lines, "Placements", text(value, "placement_count"));
    fact(lines, "Default exposure", text(value, "default_exposure"));
    metadata_footprint(value, lines, width);
    let coverage = &value["session_coverage"];
    fact_items(
        lines,
        "Usage evidence",
        vec![
            format!(
                "{} Agents observed",
                text(value, "observed_use_agent_count")
            ),
            format!(
                "{}/{} complete",
                text(coverage, "complete_agents"),
                text(coverage, "supported_agents")
            ),
            format!("{} sampled", text(coverage, "sampled_agents")),
        ],
        width,
    );
    fact_items(
        lines,
        "Coverage limits",
        vec![
            format!("{} limited", text(coverage, "limited_agents")),
            format!("{} missing", text(coverage, "missing_root_agents")),
            format!("{} inaccessible", text(coverage, "inaccessible_agents")),
        ],
        width,
    );
    let roots = &value["overview"]["skill_roots"];
    fact_items(
        lines,
        "Skill roots",
        vec![
            format!("{} included", text(roots, "included")),
            format!("{} inaccessible", text(roots, "inaccessible")),
            format!("{} bounded", text(roots, "bounded")),
        ],
        width,
    );
    lines.push("Scope: local files; session catalog unobserved.".into());
    lines.push("Missing usage is unknown, not unused.".into());
    lines.push(String::new());
    let findings = value["findings"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let count = value["overview"]["review_finding_count"]
        .as_u64()
        .unwrap_or(0);
    if findings.is_empty() {
        lines.push(if value["overview"]["state"] == "inventory_empty" {
            "No Skills found in checked roots.".into()
        } else {
            "No review items. Keep the current setup.".into()
        });
    } else {
        lines.push(format!(
            "Review Findings · {} of {count}",
            findings.len().min(3)
        ));
        for finding in findings.iter().take(3) {
            let prefix = format!("  {:<7} ", text(finding, "severity"));
            let title = if finding["kind"] == "large_default_roster" {
                finding["affected_agents"]
                    .as_array()
                    .filter(|agents| agents.len() == 1)
                    .and_then(|agents| agents[0].as_str())
                    .map(|agent| format!("Large {} Roster needs review", core_agent_label(agent)))
                    .unwrap_or_else(|| text(finding, "title"))
            } else {
                text(finding, "title")
            };
            let suffix = match finding["actionability"].as_str() {
                Some("blocked") => " · blocked",
                Some("plan_available") => " · plan available",
                _ => "",
            };
            lines.push(format!(
                "{prefix}{}{suffix}",
                middle_truncate(
                    &title,
                    width.saturating_sub(display_width(&prefix) + display_width(suffix))
                )
            ));
        }
        if findings
            .first()
            .is_some_and(|f| f["kind"] == "escaping_link_source_confirmation")
        {
            lines.push("Review exact external paths before permitting reads.".into());
        }
    }
    if value["shared_adoption"].is_object() {
        lines.push("Shared Skills: explicit adoption preview in report --full.".into());
    }
    lines.push(format!("Report: {}", text(value, "report_id")));
    lines.extend(summary(
        "Read-only · no Agent files changed",
        if findings.is_empty() {
            "Next: keep this setup; use report --findings for context."
        } else {
            "Next: review IDs and evidence with report --findings."
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn check_keeps_metrics_scope_and_three_findings_in_a_compact_plain_view() {
        let value = json!({
            "scan_performed":true, "skill_count":60, "placement_count":60, "default_exposure":60,
            "exposed_metadata_footprint":{"exposed_metadata_bytes":68700, "exposed_frontmatter_bytes":4380,
                "metadata_unknown_placement_count":0, "frontmatter_unknown_placement_count":0},
            "observed_use_agent_count":0,
            "session_coverage":{"sampled_agents":0,"complete_agents":0,"supported_agents":8,
                "limited_agents":0,"missing_root_agents":8,"inaccessible_agents":0},
            "overview":{"state":"review_available", "review_finding_count":27,
                "skill_roots":{"included":1,"inaccessible":0,"bounded":0}},
            "findings":[{"severity":"medium", "title":"Large default Rosters need review", "actionability":"blocked"},
                {"severity":"low", "title":"Skill provenance is unknown"},
                {"severity":"low", "title":"Semantic overlap candidate"}],
            "report_id":"report_01K00000000000000000000000"
        });
        for width in [60, 80, 120] {
            let output = super::super::render(
                "check",
                &value,
                RenderOptions {
                    width,
                    styled: false,
                },
            );
            assert!(output.lines().count() <= 30, "{output}");
            assert!(
                output.lines().all(|line| display_width(line) <= width),
                "{width}: {output}"
            );
            for expected in [
                "scanned now",
                "67.1 KiB",
                "4.3 KiB",
                "3 of 27",
                "session catalog unobserved",
                "not unused",
                "Read-only",
                "report --findings",
                "blocked",
            ] {
                assert!(output.contains(expected), "{expected}: {output}");
            }
            assert!(!output.contains('\u{1b}'));
        }
    }
}
