use super::*;

pub(crate) fn run_mcp_find_process(cli: Cli) -> Result<(ActionContext, Result<Output>)> {
    // MCP reads validate freshness, but an explicit Snapshot never authorizes refresh.
    let refresh = matches!(&cli.command, Some(Command::Find(_)));
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
    let (scanned, warnings) = if refresh && args.require_snapshot.is_some() {
        if let Some((id, snapshot)) = store.latest_scan_payload::<ScanResult>()? {
            if args.require_snapshot.as_deref() != Some(id.as_str()) {
                let (result, actions) = find_command(store, state_dir, args)?;
                return Ok((result, vec![], actions));
            }
            require_loaded_snapshot_current(store, &id, &snapshot)?;
            require_content_identity(&snapshot)?;
            let explicit = parse_explicit_roots(roots)?;
            let sources = parse_source_roots(source_roots)?;
            let configuration = snapshot_configuration(home, &explicit, &sources)?;
            let options = freshness_options(store, home, state_dir, explicit, sources)?;
            require_filesystem_snapshot(&snapshot, &configuration, &options)?;
        }
        (false, vec![])
    } else if refresh {
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
    let explicit = parse_explicit_roots(roots)?;
    let source_roots = parse_source_roots(source_roots)?;
    let configuration = snapshot_configuration(home, &explicit, &source_roots)?;
    let options = freshness_options(
        store,
        home,
        state_dir,
        explicit.clone(),
        source_roots.clone(),
    )?;
    let refresh = match store.latest_scan_payload::<ScanResult>()? {
        None => true,
        Some((id, snapshot)) => {
            match require_loaded_snapshot_current(store, &id, &snapshot)
                .and_then(|()| require_content_identity(&snapshot))
            {
                Ok(()) => {
                    !snapshot.entrypoint_bytes_observed
                        || require_filesystem_snapshot(&snapshot, &configuration, &options).is_err()
                }
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
        scan_command(store, home, state_dir, explicit, source_roots, true)?.1
    } else {
        Vec::new()
    };
    Ok((refresh, warnings))
}

fn freshness_options(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    explicit: Vec<ExplicitSkillRoot>,
    source_roots: Vec<PathBuf>,
) -> Result<ScanOptions> {
    let mut options = ScanOptions::for_home(home);
    options.explicit_skill_roots = explicit;
    options.explicit_source_roots = source_roots;
    options.managed_source_roots = vec![state_dir.join("library")];
    options.durable_read_roots = crate::source_policy::freeze_active_roots(store)?
        .into_iter()
        .filter(|root| root.state == crate::source_policy::SourceRootState::Active)
        .filter_map(|root| {
            root.resolved_path.map(|path| crate::scan::DurableReadRoot {
                permission_id: root.permission.id.as_str().to_owned(),
                path,
                identity: root.permission.identity,
            })
        })
        .collect();
    Ok(options)
}

fn require_filesystem_snapshot(
    snapshot: &ScanResult,
    configuration: &crate::scan::SnapshotConfiguration,
    options: &ScanOptions,
) -> Result<()> {
    let reason = if snapshot.filesystem_fingerprint.is_none() {
        Some("filesystem_snapshot_fingerprint_missing")
    } else if snapshot.snapshot_configuration.as_ref() != Some(configuration)
        || snapshot.filesystem_fingerprint != scan::filesystem_fingerprint(options)
    {
        Some("filesystem_snapshot_changed")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(ContentIdentityRescanRequired { reason }.into());
    }
    Ok(())
}

fn snapshot_configuration(
    home: &Path,
    explicit: &[ExplicitSkillRoot],
    source_roots: &[PathBuf],
) -> Result<crate::scan::SnapshotConfiguration> {
    let normalize = |path: &Path| -> std::io::Result<PathBuf> {
        std::fs::canonicalize(path).or_else(|_| {
            let mut normalized = PathBuf::new();
            for component in std::path::absolute(path)?.components() {
                match component {
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    other => normalized.push(other.as_os_str()),
                }
            }
            Ok(normalized)
        })
    };
    let mut explicit_skill_roots = explicit
        .iter()
        .map(|root| {
            Ok(ExplicitSkillRoot {
                agent: root.agent,
                path: normalize(&root.path)?,
            })
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    explicit_skill_roots.sort_by(|a, b| (a.agent, &a.path).cmp(&(b.agent, &b.path)));
    explicit_skill_roots.dedup();
    let mut explicit_source_roots = source_roots
        .iter()
        .map(|path| normalize(path))
        .collect::<std::io::Result<Vec<_>>>()?;
    explicit_source_roots.sort();
    explicit_source_roots.dedup();
    Ok(crate::scan::SnapshotConfiguration {
        home: normalize(home)?,
        explicit_skill_roots,
        explicit_source_roots,
    })
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
    result["project_local_skill_roots"] =
        json!(project_local_skill_roots(home, roots, source_roots)?);
    Ok((result, warnings, actions))
}

fn project_local_skill_roots(
    home: &Path,
    explicit: &[String],
    sources: &[PathBuf],
) -> Result<Vec<Value>> {
    let selected = parse_explicit_roots(explicit)?
        .into_iter()
        .map(|root| root.path)
        .chain(sources.iter().cloned())
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .collect::<std::collections::BTreeSet<_>>();
    let cwd = std::env::current_dir()?;
    if std::fs::canonicalize(&cwd).ok() == std::fs::canonicalize(home).ok() {
        return Ok(Vec::new());
    }
    let mut roots = Vec::new();
    for (agent, relative) in [
        ("codex", ".agents/skills"),
        ("claude-code", ".claude/skills"),
    ] {
        let path = cwd.join(relative);
        // A hint observes only the exact directory path. Do not inspect package
        // contents or traverse a linked project configuration directory.
        let parent = path.parent().expect("fixed project Skill path");
        if std::fs::symlink_metadata(parent)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            && std::fs::symlink_metadata(&path)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            if std::fs::canonicalize(&path)
                .ok()
                .is_some_and(|path| selected.contains(&path))
            {
                continue;
            }
            roots.push(json!({
                "agent": agent, "path": path, "included_in_snapshot": false,
                "contents_read": false, "explicit_selection_required": true,
                "note": "Project Skill directory observed only. Choose --root explicitly to include it as an Agent placement root; linked source contents require separate read approval.",
                "argv": ["skillroster", "--root", format!("{agent}={}", path.display()), "check", "--json"]
            }));
        }
    }
    Ok(roots)
}

pub(super) fn scan_command(
    store: &StateStore,
    home: &Path,
    state_dir: &Path,
    explicit: Vec<ExplicitSkillRoot>,
    source_roots: Vec<PathBuf>,
    summary: bool,
) -> Result<(Value, Vec<String>)> {
    let configuration = snapshot_configuration(home, &explicit, &source_roots)?;
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
    result.snapshot_configuration = Some(configuration);
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
