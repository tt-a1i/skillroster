use super::*;

pub(crate) fn run_mcp_find_process(cli: Cli) -> Result<(ActionContext, Result<Output>)> {
    // An explicit Snapshot is an optimistic read boundary, never an auto-refresh request.
    let refresh =
        matches!(&cli.command, Some(Command::Find(args)) if args.require_snapshot.is_none());
    let context = ActionContext::from_process(&cli)?;
    let output = run_with_context(cli, &context, refresh);
    Ok((context, output))
}

pub(super) fn find_with_snapshot(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    roots: &[String],
    source_roots: &[PathBuf],
    args: &crate::cli::FindArgs,
    refresh: bool,
) -> Result<(Value, Vec<String>, Vec<SuggestedAction>)> {
    let (scanned, warnings) = if refresh {
        ensure_snapshot(store, home, state_dir, roots, source_roots)?
    } else {
        (false, vec![])
    };
    let (mut result, actions) = find_command(store, state_dir, args)?;
    if refresh {
        result["scan_performed"] = json!(scanned);
    }
    Ok((result, warnings, actions))
}

pub(super) fn ensure_snapshot(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    roots: &[String],
    source_roots: &[PathBuf],
) -> Result<(bool, Vec<String>)> {
    let refresh = match store.latest_scan_payload::<ScanResult>()? {
        None => true,
        Some((id, snapshot)) => {
            match require_loaded_snapshot_current(store, &id, &snapshot)
                .and_then(|()| require_content_identity(&snapshot))
            {
                Ok(()) => !snapshot.entrypoint_bytes_observed,
                Err(error)
                    if error.is::<SnapshotRescanRequired>()
                        || error.is::<SourceRootSnapshotRescanRequired>()
                        || error.is::<ContentIdentityRescanRequired>() =>
                {
                    true
                }
                Err(error) => return Err(error),
            }
        }
    };
    let warnings = if refresh {
        scan_command(
            store,
            home,
            state_dir,
            parse_explicit_roots(roots)?,
            parse_source_roots(source_roots)?,
            true,
        )?
        .1
    } else {
        Vec::new()
    };
    Ok((refresh, warnings))
}

pub(super) fn check_command(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    roots: &[String],
    source_roots: &[PathBuf],
) -> Result<(Value, Vec<String>, Vec<SuggestedAction>)> {
    let (scanned, warnings) = ensure_snapshot(store, home, state_dir, roots, source_roots)?;
    let mut result = report_command(store, state_dir, ReportRequest::Summary)?;
    let actions = report_actions(&result, ReportRequest::Summary);
    result["scan_performed"] = json!(scanned);
    Ok((result, warnings, actions))
}

pub(super) fn scan_command(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    explicit: Vec<ExplicitSkillRoot>,
    source_roots: Vec<PathBuf>,
    summary: bool,
) -> Result<(Value, Vec<String>)> {
    let started = Utc::now().timestamp();
    let id = ScanId::new();
    // Freeze durable read permissions once before discovery. Only exact roots
    // whose path and stable filesystem identity still match enter the Scan;
    // drift is local to that permission and persists as typed Snapshot facts.
    let frozen_permissions = crate::source_policy::freeze_active_roots(store)?;
    let durable_read_roots = frozen_permissions
        .iter()
        .filter(|root| root.state == crate::source_policy::SourceRootState::Active)
        .filter_map(|root| {
            root.resolved_path
                .clone()
                .map(|path| crate::scan::DurableReadRoot {
                    permission_id: root.permission.id.as_str().to_owned(),
                    path,
                    identity: root.permission.identity.clone(),
                })
        })
        .collect::<Vec<_>>();
    store.save_scan(&ScanRun {
        id: id.clone(),
        started_at: started,
        completed_at: None,
        status: ScanStatus::Running,
        coverage_notes: vec![],
    })?;
    let mut options = ScanOptions::for_home(home);
    options.explicit_skill_roots = explicit;
    options.explicit_source_roots = source_roots;
    options.durable_read_roots = durable_read_roots;
    options.managed_source_roots = vec![state_dir.join("library")];
    options.excluded_session_agents = store
        .evidence_exclusions()?
        .iter()
        .map(|agent| parse_agent_kind(agent))
        .collect::<Result<_>>()?;
    let mut result = match scan::scan(&options) {
        Ok(result) => result,
        Err(error) => {
            store.save_scan(&ScanRun {
                id,
                started_at: started,
                completed_at: Some(Utc::now().timestamp()),
                status: ScanStatus::Failed,
                coverage_notes: vec![error.to_string()],
            })?;
            return Err(error.into());
        }
    };
    let post_scan_permissions = match crate::source_policy::freeze_active_roots(store) {
        Ok(permissions) => permissions,
        Err(error) => {
            store.save_scan(&ScanRun {
                id,
                started_at: started,
                completed_at: Some(Utc::now().timestamp()),
                status: ScanStatus::Failed,
                coverage_notes: vec![error.to_string()],
            })?;
            return Err(error.into());
        }
    };
    let policy_facts = conservative_source_policy_facts(
        &frozen_permissions,
        &post_scan_permissions,
        &result.durable_read_drifted_permission_ids,
    );
    for fact in policy_facts
        .iter()
        .filter(|fact| fact.state != crate::source_policy::SourceRootState::Active)
    {
        result.warnings.push(format!(
            "source-root permission {} is {} and was excluded: {}",
            fact.permission_id,
            serde_json::to_value(fact.state)?
                .as_str()
                .unwrap_or("drifted"),
            fact.drift_reason.as_deref().unwrap_or("identity drift")
        ));
    }
    result.source_root_policy = policy_facts;
    store.begin_scan_snapshot()?;
    let persistence = (|| -> Result<()> {
        persist_index(store, &id, &result)?;
        store.save_scan_payload(&id, &result)?;
        store.save_scan(&ScanRun {
            id: id.clone(),
            started_at: started,
            completed_at: Some(Utc::now().timestamp()),
            status: ScanStatus::Completed,
            coverage_notes: result.warnings.clone(),
        })?;
        store.commit_scan_snapshot()?;
        Ok(())
    })();
    if let Err(error) = persistence {
        let rollback_error = store.rollback_scan_snapshot().err();
        store.save_scan(&ScanRun {
            id,
            started_at: started,
            completed_at: Some(Utc::now().timestamp()),
            status: ScanStatus::Failed,
            coverage_notes: vec![error.to_string()],
        })?;
        if let Some(rollback_error) = rollback_error {
            bail!("Scan persistence failed: {error}; rollback failed: {rollback_error}");
        }
        return Err(error);
    }
    let agents_checked = result
        .roots
        .iter()
        .filter_map(|root| root.agent)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let warnings = compact_scan_warnings(result.warnings.clone());
    let public_result = if summary {
        scan_summary_value(&id, agents_checked, &result)?
    } else {
        let roots = result
            .roots
            .iter()
            .map(|root| {
                let mut value = json!(root);
                value["agent"] = json!(root.agent.map(AgentKind::id));
                value
            })
            .collect::<Vec<_>>();
        let coverage = result
            .coverage
            .iter()
            .map(session_coverage_details)
            .collect::<Result<Vec<_>>>()?;
        json!({
            "snapshot_id": &id,
            "agents_checked": agents_checked,
            "skill_count": result.skills.len(),
            "placement_count": result.placements.len(),
            "default_exposure": result.placements.iter().filter(|p| p.default_exposed).count(),
            "exposed_metadata_footprint": crate::footprint::exposed(&result),
            "native_visibility": result.native_visibility.as_ref().map(|visibility| visibility.summary()),
            "codex_visibility": result.codex_visibility.as_ref().map(|visibility| visibility.summary()),
            "hermes_visibility": result.hermes_visibility.as_ref().map(|visibility| visibility.summary()),
            "roots": roots,
            "coverage": coverage,
            "source_root_policy": {
                "scope": "exact_local_read_only",
                "content_endorsed": false,
                "evidence_quality_changed": false,
                "governance_authorized": false,
                "permissions": result.source_root_policy,
            },
            "files_changed": false
        })
    };
    Ok((public_result, warnings))
}
