use super::*;

const DECLARED_CONTENT_FINDING: &str = "declared_identity_divergent_content";

pub(super) fn declared_content_choice(
    scan: &ScanResult,
    state_dir: &Path,
    skill_id: &str,
    limit: usize,
) -> Value {
    let mut groups = BTreeMap::<Option<&str>, Vec<&scan::SkillPlacement>>::new();
    for placement in scan.placements.iter().filter(|p| p.skill_id == skill_id) {
        groups
            .entry(placement.content_identity_digest.as_deref())
            .or_default()
            .push(placement);
    }
    let count = groups.len();
    let known_count = groups.keys().filter(|digest| digest.is_some()).count();
    let unknown_count = groups.get(&None).map_or(0, Vec::len);
    let groups = groups.into_iter().take(limit).map(|(digest, mut placements)| {
        placements.sort_by(|left, right| (!left.default_exposed, &left.entrypoint, &left.id).cmp(&(!right.default_exposed, &right.entrypoint, &right.id)));
        let placement_count = placements.len();
        let observed_name = placements.first().and_then(|p| scan.observed_skill_for_placement(p)).map(|s| &s.name);
        let items = placements.into_iter().take(limit).map(|placement| {
            let observed = scan.observed_skill_for_placement(placement);
            json!({
                "placement_id": placement.id,
                "path": placement.entrypoint,
                "name": observed.map(|s| &s.name),
                "content_identity_digest": placement.content_identity_digest,
                "package_fingerprint": placement.content_digest,
                "entrypoint_digest": placement.entrypoint_digest,
                "fingerprint_completeness": placement.fingerprint_completeness,
                "mutation_scope": placement.mutation_scope,
                "owned_by_agent": placement.owned_by_agent,
                "agent": placement.agent.map(AgentKind::id),
                "provider": placement.provider,
                "read_state": placement_read_state(scan, state_dir, placement),
            })
        }).collect::<Vec<_>>();
        json!({ "content_identity_digest": digest, "name": observed_name,
            "placement_count": placement_count, "placements_truncated": items.len() < placement_count,
            "placements": items })
    }).collect::<Vec<_>>();
    json!({
        "decision": "choose_declared_identity_content",
        "skill_id": skill_id,
        "state": if known_count > 1 { "selection_required" } else if unknown_count > 0 { "content_identity_incomplete" } else { "routing_equivalent" },
        "content_selection_required": known_count > 1,
        "routing_equivalent": known_count == 1 && unknown_count == 0,
        "content_group_count": count,
        "unknown_content_placement_count": unknown_count,
        "groups_truncated": groups.len() < count,
        "groups": groups,
    })
}

/// Validate each frozen placement independently. A healthy copy must not hide
/// drift in a different content choice under the same declared identity.
pub(super) fn placement_read_state(
    scan: &ScanResult,
    state_dir: &Path,
    placement: &scan::SkillPlacement,
) -> &'static str {
    if placement
        .mutation_scope
        .is_none_or(|s| s == scan::MutationScope::UntrustedExternal)
    {
        return "untrusted_external_source";
    }
    if placement.fingerprint_completeness != scan::FingerprintCompleteness::Complete {
        return "package_fingerprint_incomplete";
    }
    if placement.content_identity_digest.is_none()
        || scan.observed_skill_for_placement(placement).is_none()
    {
        return "placement_routing_identity_missing";
    }
    let Some(expected_entrypoint) = placement.entrypoint_digest.as_deref() else {
        return "legacy_snapshot_requires_rescan";
    };
    let Ok(path) = std::fs::canonicalize(&placement.entrypoint) else {
        return "entrypoint_unreadable";
    };
    let allowed = scan
        .roots
        .iter()
        .filter(|r| r.kind == RootKind::Skills && r.status == scan::RootStatus::Included)
        .filter_map(|r| std::fs::canonicalize(&r.path).ok())
        .chain(std::fs::canonicalize(state_dir.join("library")).ok())
        .any(|root| path.starts_with(root));
    if !path.is_file() || !allowed {
        return "entrypoint_escapes_approved_roots";
    }
    if placement.validated_physical_directory().is_err()
        || path.parent() != placement.physical_directory.as_deref()
    {
        return "physical_source_drift";
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_AGENT_LOADED_SKILL_BYTES) {
        return "entrypoint_exceeds_content_limit";
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return "entrypoint_unreadable";
    };
    if bytes.len() as u64 > MAX_AGENT_LOADED_SKILL_BYTES {
        return "entrypoint_exceeds_content_limit";
    }
    if content_digest(&bytes) != expected_entrypoint {
        return "entrypoint_content_drift";
    }
    if std::str::from_utf8(&bytes).is_err() {
        return "entrypoint_not_utf8";
    }
    match scan::inspect_skill_identity(&path) {
        Ok((id, digest)) if id == placement.skill_id && digest == placement.content_digest => {
            "available"
        }
        Ok(_) => "package_identity_drift",
        Err(_) => "package_identity_unreadable",
    }
}

pub(super) fn select_top_match_variant(
    store: &StateStore,
    scan: &ScanResult,
    routable_ids: &BTreeSet<String>,
    ranked: &crate::query::FindMatch,
    requested_skill_id: Option<&str>,
    requested_placement_id: Option<&str>,
) -> Result<crate::query::FindMatch> {
    let blocked = |reason| SkillLoadBlocked {
        reason,
        skill_id: requested_skill_id.unwrap_or(&ranked.skill_id).to_owned(),
        skill_name: ranked.name.clone(),
        path: None,
        roster_state: "unknown".into(),
        mutation_scopes: Vec::new(),
        expected_digest: None,
        actual_digest: None,
        retry_argv: None,
    };
    if requested_skill_id.is_none() && requested_placement_id.is_none() {
        return Ok(ranked.clone());
    }
    let placement = if let Some(id) = requested_placement_id {
        Some(
            scan.placements
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| blocked("variant_placement_not_in_top_match"))?,
        )
    } else {
        None
    };
    if let (Some(skill_id), Some(placement)) = (requested_skill_id, placement)
        && skill_id != placement.skill_id
    {
        return Err(blocked("variant_selectors_disagree").into());
    }
    let skill_id = requested_skill_id
        .or_else(|| placement.map(|p| p.skill_id.as_str()))
        .unwrap_or(&ranked.skill_id);
    if ranked.variant_count <= 1 && requested_placement_id.is_none() {
        return Err(blocked("variant_selector_requires_ambiguous_top_match").into());
    }
    let member = skill_id == ranked.skill_id
        || scan.routing_skills().any(|skill| {
            skill.id == skill_id
                && skill.name.trim().to_lowercase() == ranked.name.trim().to_lowercase()
        });
    if !member || !routable_ids.contains(skill_id) {
        return Err(blocked(if placement.is_some() {
            "variant_placement_not_in_top_match"
        } else {
            "variant_not_in_top_match"
        })
        .into());
    }
    let observed = placement
        .and_then(|p| scan.observed_skill_for_placement(p))
        .or_else(|| {
            scan.routing_skills().find(|s| {
                s.id == skill_id
                    && s.name.trim().to_lowercase() == ranked.name.trim().to_lowercase()
            })
        })
        .ok_or_else(|| blocked("placement_routing_identity_missing"))?;
    let mut selected = ranked.clone();
    selected.skill_id = skill_id.to_owned();
    selected.name = observed.name.clone();
    selected.source = observed.metadata.source.clone();
    selected.roster_state = current_roster_state(
        &store.roster_states_for_skill(&SkillId::parse(skill_id.to_owned())?)?,
    );
    selected.paths = scan
        .placements
        .iter()
        .filter(|p| p.skill_id == skill_id)
        .map(|p| p.entrypoint.display().to_string())
        .collect();
    let placements = scan
        .placements
        .iter()
        .filter(|p| p.skill_id == skill_id)
        .filter(|p| placement.is_none_or(|selected| p.id == selected.id))
        .collect::<Vec<_>>();
    selected.agents = placements
        .iter()
        .filter_map(|p| p.agent.map(|a| a.id().to_owned()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    selected.providers = placements
        .iter()
        .filter_map(|p| p.provider.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    selected.governable = placements.iter().any(|p| p.is_mutable());
    let authority = placement_authority_summary(&placements);
    selected.owned_by_agent = authority.owned_by_agent;
    selected.mutation_scopes = authority.mutation_scopes;
    selected.variant_skill_ids.clear();
    selected.variants.clear();
    selected.variant_count = 1;
    selected.variants_truncated = false;
    selected.variant_finding = None;
    Ok(selected)
}

pub(super) fn add_declared_content_resolution(
    object: &mut serde_json::Map<String, Value>,
    scan: &ScanResult,
    state_dir: &Path,
    scan_id: &ScanId,
    full: bool,
) -> Result<()> {
    if object.get("kind").and_then(Value::as_str) != Some(DECLARED_CONTENT_FINDING) {
        return Ok(());
    }
    let Some(skill_id) = object
        .get("affected_skill_ids")
        .and_then(Value::as_array)
        .and_then(|ids| ids.first())
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    let choice = declared_content_choice(
        scan,
        state_dir,
        skill_id,
        if full { usize::MAX } else { 10 },
    );
    let mut resolution = json!({
        "decision": "choose_declared_identity_content", "automatic_change_supported": false,
        "routing_equivalent": choice["routing_equivalent"],
        "content_selection_required": choice["content_selection_required"],
        "content_choice": choice,
        "next_step": "compare_observed_content_without_changing_governance",
    });
    for group in resolution["content_choice"]["groups"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        for placement in group["placements"].as_array_mut().into_iter().flatten() {
            if placement["read_state"] == "available" {
                let name = placement["name"].as_str().unwrap_or_default();
                let id = placement["placement_id"].as_str().unwrap_or_default();
                let mut argv = vec!["skillroster".to_owned()];
                argv.extend(find_action_argv(
                    name,
                    &[],
                    FindActionKind::LoadPlacement { placement_id: id },
                    Some(scan_id),
                ));
                placement["load_argv"] = json!(argv);
            }
        }
    }
    object.insert("resolution".into(), resolution);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn bind_content_choices(
    store: &StateStore,
    state_dir: &Path,
    scan_id: &ScanId,
    scan: &ScanResult,
    task: &str,
    hints: &[String],
    matches: &[crate::query::FindMatch],
    offer_load_actions: bool,
    result: &mut Value,
    actions: &mut Vec<SuggestedAction>,
) -> Result<()> {
    let report = store.latest_report()?.filter(|r| r.scan_id == *scan_id);
    for (index, matched) in matches.iter().enumerate() {
        let mut choice = declared_content_choice(scan, state_dir, &matched.skill_id, 10);
        let mut inspect = None;
        if choice["content_selection_required"] == true {
            let expected_ids = scan
                .placements
                .iter()
                .filter(|p| p.skill_id == matched.skill_id)
                .map(|p| p.id.as_str())
                .collect::<BTreeSet<_>>();
            let finding_id = if let Some(report) = report.as_ref() {
                let mut found = None;
                for finding in report.summary["findings"].as_array().into_iter().flatten() {
                    if finding["kind"] != DECLARED_CONTENT_FINDING
                        || finding_skill_ids(finding)
                            != Some(BTreeSet::from([matched.skill_id.clone()]))
                    {
                        continue;
                    }
                    let Some(id) = finding["id"].as_str() else {
                        continue;
                    };
                    let Some(stored) = store.get_finding(&FindingId::parse(id.to_owned())?)? else {
                        continue;
                    };
                    let actual_ids = stored.details["affected_placement_ids"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<BTreeSet<_>>();
                    if stored.report_id == report.id
                        && stored.details["kind"] == DECLARED_CONTENT_FINDING
                        && actual_ids == expected_ids
                    {
                        found = Some(id.to_owned());
                        break;
                    }
                }
                found
            } else {
                None
            };
            let (state, reason_code, suggested) = if let Some(id) = finding_id.as_deref() {
                (
                    crate::query::VariantFindingState::Available,
                    crate::query::VariantFindingReason::SameSnapshotVariantSetMatched,
                    action(
                        "inspect_declared_identity_content",
                        &["report", "--finding", id, "--json"],
                        false,
                        false,
                        "same_snapshot_variant_finding_available",
                    ),
                )
            } else if report.is_none() {
                (
                    crate::query::VariantFindingState::ReportRequired,
                    crate::query::VariantFindingReason::CurrentSnapshotReportMissing,
                    action(
                        "materialize_report",
                        &["report", "--summary", "--json"],
                        false,
                        false,
                        "current_snapshot_report_missing",
                    ),
                )
            } else {
                (
                    crate::query::VariantFindingState::FindingUnavailable,
                    crate::query::VariantFindingReason::MatchingDivergentContentFindingMissing,
                    action(
                        "inspect_layout_findings",
                        &["report", "--findings", "--category", "layout", "--json"],
                        false,
                        false,
                        "matching_divergent_content_finding_missing",
                    ),
                )
            };
            inspect = Some(json!(crate::query::VariantFindingReference {
                state,
                reason_code,
                snapshot_id: scan_id.to_string(),
                report_id: report.as_ref().map(|r| r.id.to_string()),
                finding_id,
                resolution: Some("choose_declared_identity_content".into()),
                argv: suggested.argv.clone(),
            }));
            if index < 3 && !actions.iter().any(|a| a.argv == suggested.argv) {
                actions.push(suggested);
            }
        }
        if let Some(inspect) = inspect {
            choice["inspection"] = inspect;
        }
        // Only the Top-1 group authorizes exact selection. Lower-ranked choices
        // remain useful facts without an action that would falsely promise load.
        if index == 0 && offer_load_actions {
            let mut top_ids = BTreeSet::from([matched.skill_id.as_str()]);
            top_ids.extend(matched.variants.iter().map(|v| v.skill_id.as_str()));
            for skill_id in top_ids {
                let identity_choice = declared_content_choice(scan, state_dir, skill_id, 10);
                for group in identity_choice["groups"].as_array().into_iter().flatten() {
                    if let Some(placement) = group["placements"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|p| p["read_state"] == "available")
                    {
                        let id = placement["placement_id"].as_str().unwrap_or_default();
                        let mut argv = vec!["skillroster".to_owned()];
                        argv.extend(find_action_argv(
                            task,
                            hints,
                            FindActionKind::LoadPlacement { placement_id: id },
                            Some(scan_id),
                        ));
                        actions.push(SuggestedAction {
                            action: "load_exact_content_for_comparison".into(),
                            description: "load_exact_content_for_comparison".into(),
                            argv,
                            mutates: false,
                            requires_confirmation: false,
                            reason_code: "declared_identity_content_explicit_read_available".into(),
                        });
                    }
                }
            }
        }
        result["matches"][index]["content_choice"] = choice;
        for variant in result["matches"][index]["variants"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            if let Some(skill_id) = variant["skill_id"].as_str() {
                variant["content_choice"] = declared_content_choice(scan, state_dir, skill_id, 10);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn content_choices_preserve_drift_on_one_copy_when_another_is_readable() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let root = home.join(".codex/skills");
        for name in ["research", "deploy"] {
            let package = root.join(name);
            std::fs::create_dir_all(&package).unwrap();
            std::fs::write(package.join("SKILL.md"), format!("---\nname: {name}\ndescription: Perform {name}\nsource: example/shared\nversion: 1.0\n---\n{name} instructions\n")).unwrap();
        }
        let mut options = ScanOptions::for_home(home);
        options.include_session_evidence = false;
        let scan = scan::scan(&options).unwrap();
        let skill_id = &scan.skills[0].id;
        std::fs::write(root.join("research/SKILL.md"), "drifted").unwrap();
        let choice = declared_content_choice(&scan, temp.path(), skill_id, 10);
        assert_eq!(choice["content_group_count"], 2);
        assert_eq!(choice["content_selection_required"], true);
        let states = choice["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["placements"].as_array().unwrap())
            .map(|p| {
                (
                    p["name"].as_str().unwrap(),
                    p["read_state"].as_str().unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(states["research"], "entrypoint_content_drift");
        assert_eq!(states["deploy"], "available");
        let current = current_readable_skill_paths(&scan, temp.path(), skill_id).unwrap();
        assert!(current.drifted);
        assert_eq!(current.paths.len(), 1);
    }

    #[test]
    fn exact_identity_contract_rejects_legacy_observations_and_missing_placement_digest() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let package = home.join(".codex/skills/research");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("SKILL.md"),
            "---\nname: research\ndescription: Research\n---\nInstructions\n",
        )
        .unwrap();
        let mut options = ScanOptions::for_home(home);
        options.include_session_evidence = false;
        let mut scan = scan::scan(&options).unwrap();
        assert!(require_content_identity(&scan).is_ok());
        scan.skill_content_variants = None;
        assert_eq!(
            require_content_identity(&scan)
                .unwrap_err()
                .downcast_ref::<ContentIdentityRescanRequired>()
                .unwrap()
                .reason,
            "legacy_content_observations_require_rescan"
        );
        scan.skill_content_variants = Some(Vec::new());
        scan.placements[0].content_identity_digest = None;
        assert_eq!(
            require_content_identity(&scan)
                .unwrap_err()
                .downcast_ref::<ContentIdentityRescanRequired>()
                .unwrap()
                .reason,
            "placement_routing_identity_missing"
        );
    }
    #[cfg(unix)]
    #[test]
    fn exact_load_keeps_physical_binding_when_a_retarget_stays_inside_approved_roots() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let root = home.join(".codex/skills");
        let package = root.join("research");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("SKILL.md"),
            "---\nname: research\ndescription: Research\n---\nPRIVATE_INSTRUCTIONS\n",
        )
        .unwrap();
        let mut options = ScanOptions::for_home(home);
        options.include_session_evidence = false;
        let scan = scan::scan(&options).unwrap();
        let query = crate::query::RetrievalQuery::from_task_and_hints(
            "research",
            std::iter::empty::<&str>(),
        );
        let matched = crate::query::find_matching(&scan, &query, 1, None, None).remove(0);
        let placement = &scan.placements[0];
        let moved = root.join("moved");
        std::fs::rename(&package, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &package).unwrap();
        let error = verified_skill_placement_load(
            &ScanId::parse("scan_physical-binding").unwrap(),
            &scan,
            temp.path(),
            &matched,
            None,
            Some(&placement.id),
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<SkillLoadBlocked>().unwrap().reason,
            "physical_source_drift"
        );
        assert_eq!(
            placement_read_state(&scan, temp.path(), placement),
            "physical_source_drift"
        );
    }
}
