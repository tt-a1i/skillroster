use super::*;

pub(super) fn preview(value: &Value, lines: &mut Vec<String>, width: usize) {
    fact(lines, "Receipt", text(value, "receipt_id"));
    fact(lines, "Plan", text(value, "plan_id"));
    fact(lines, "Recorded operations", text(value, "operation_count"));
    fact(lines, "Recorded paths", text(value, "changed_path_count"));
    if value["affected"].is_object() {
        fact_items(
            lines,
            "Affected",
            vec![
                format!("{} Agents", text(&value["affected"], "agent_count")),
                format!("{} Skills", text(&value["affected"], "skill_count")),
            ],
            width,
        );
    }
    let groups = &value["original_operation_groups"];
    let items = [
        ("create_directory", "created directories"),
        ("create_symlink", "created links"),
        ("remove_symlink", "removed links"),
        ("write_file", "new files"),
        ("replace_file", "replaced files"),
        ("copy", "copied packages"),
        ("move_recoverable", "moved packages"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        groups[key]
            .as_u64()
            .filter(|count| *count > 0)
            .map(|count| format!("{count} {label}"))
    })
    .collect();
    fact_items(lines, "Reverse recorded", items, width);
    lines.push(String::new());
    lines.push("Only this Receipt's changes are in scope.".into());
    lines.push("Drift is refused; unrelated files stay outside Undo.".into());
    lines.push(format!(
        "Exact original operations: plan --show {} --json",
        text(value, "plan_id")
    ));
    lines.push("Preview only · no files changed".into());
}
