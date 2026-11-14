use super::*;

pub(super) fn require_human_confirmation(question: &str, consequence: &str) -> Result<bool> {
    (|| {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            bail!("human mutation requires an interactive terminal; Agent callers must use --json only after explicit user confirmation");
        }
        eprintln!("{question}\n{consequence}\n\nType confirm to continue:");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        Ok(answer.trim() == "confirm")
    })().map_err(error::before_mutation)
}

pub(super) fn human_plan_preview(store: &StateStore, id: &str) -> Result<String> {
    (|| {
        let id = PlanId::parse(id.to_string())?;
        let record = store
            .get_plan(&id)?
            .ok_or_else(|| anyhow!("Plan {id} does not exist"))?;
        let prepared: PreparedPlan = serde_json::from_value(record.input["prepared"].clone())?;
        if let Some(mut summary) = record.input.get("summary").cloned() {
            summary["state"] = json!(record.status);
            return Ok(crate::present::human("plan", &summary));
        }
        let risk = plan_risk(&prepared);
        Ok(crate::present::human(
            "plan",
            &json!({
                "plan_id": prepared.id, "operations": prepared.operations,
                "roster_changes": prepared.roster_changes, "risk": risk,
                "reversible": true, "canonical_deletion_count": 0,
                "blocked_preconditions": [], "state": record.status, "files_changed": false
            }),
        ))
    })()
    .map_err(error::before_mutation)
}

pub(super) fn human_undo_preview(store: &StateStore, id: &str) -> Result<String> {
    (|| {
        let id = ReceiptId::parse(id.to_string())?;
        let record = store.get_receipt(&id)?.ok_or_else(|| anyhow!("Receipt {id} does not exist"))?;
        if record.status != ReceiptStatus::Applied || store.reverse_receipt_for(&id)?.is_some() {
            bail!("Receipt {id} is not undoable");
        }
        let plan = store.get_plan(&record.plan_id)?.ok_or_else(|| anyhow!("Plan {} does not exist", record.plan_id))?;
        let prepared: PreparedPlan = serde_json::from_value(plan.input["prepared"].clone())?;
        Ok(crate::present::human("undo-preview", &json!({
            "receipt_id": record.id, "plan_id": record.plan_id,
            "operation_count": record.operation_results.len(),
            "changed_path_count": record.verification["change_receipt"]["changed_paths"].as_array().map_or(0, Vec::len),
            "original_operation_groups": operation_groups(&prepared.operations),
            "affected": plan.input["summary"]["affected"],
            "files_changed": false,
        })))
    })().map_err(error::before_mutation)
}
