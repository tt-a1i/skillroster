use super::*;

pub(super) fn plan_from_finding_or_shared(
    store: &StateStore,
    state_dir: &Path,
    args: &crate::cli::PlanArgs,
    argv_prefix: &[String],
) -> Result<Value> {
    let budget = args
        .core_limit
        .unwrap_or(if args.finding.is_some() { 50 } else { 15 });
    let input = if let Some(id) = &args.finding {
        json!({"schema_version": 1, "finding_roster_changes": [{
            "finding_id": id, "core_budget": budget, "protected_skill_ids": []
        }]})
    } else {
        let (scan_id, _): (ScanId, ScanResult) = latest_scan(store)?;
        json!({"schema_version": 1, "scan_id": scan_id,
            "adopt_shared_skills": {"core_budget": budget, "protected_skill_ids": []}})
    };
    prepare_plan(
        store,
        state_dir,
        input,
        PlanOrigin::Agent,
        None,
        argv_prefix,
        None,
    )
}

#[derive(Clone, Copy)]
struct FindingRosterPlanningOptions {
    full: bool,
    decision_facts_only: bool,
}

pub(super) fn finding_roster_planning_decision(
    store: &StateStore,
    finding: &FindingRecord,
    scan_id: &ScanId,
    latest_scan_id: &ScanId,
    scan: &ScanResult,
    state_dir: &Path,
) -> Result<Option<Value>> {
    // Probe the same planning gates used by detail output without resolving
    // report-linked continuation data before the new Report is persisted.
    finding_roster_planning_impl(
        store,
        finding,
        scan_id,
        latest_scan_id,
        scan,
        state_dir,
        FindingRosterPlanningOptions {
            full: false,
            decision_facts_only: true,
        },
    )
}

pub(super) fn finding_roster_planning(
    store: &StateStore,
    finding: &FindingRecord,
    scan_id: &ScanId,
    latest_scan_id: &ScanId,
    scan: &ScanResult,
    state_dir: &Path,
    full: bool,
) -> Result<Option<Value>> {
    finding_roster_planning_impl(
        store,
        finding,
        scan_id,
        latest_scan_id,
        scan,
        state_dir,
        FindingRosterPlanningOptions {
            full,
            decision_facts_only: false,
        },
    )
}

fn finding_roster_planning_impl(
    store: &StateStore,
    finding: &FindingRecord,
    scan_id: &ScanId,
    latest_scan_id: &ScanId,
    scan: &ScanResult,
    state_dir: &Path,
    options: FindingRosterPlanningOptions,
) -> Result<Option<Value>> {
    let FindingRosterPlanningOptions {
        full,
        decision_facts_only,
    } = options;
    if !crate::roster_recommendation::is_large_roster_finding(finding) {
        return Ok(None);
    }
    if scan_id != latest_scan_id {
        return Ok(Some(json!({
            "supported": false,
            "reason": "stale_finding",
            "snapshot_id": scan_id,
            "latest_snapshot_id": latest_scan_id
        })));
    }
    let declared_core = declared_core_pairs(store, scan)?;
    let recommendation = match crate::roster_recommendation::recommend(
        finding,
        scan,
        &declared_core,
        &crate::roster_recommendation::RecommendationRequest {
            core_budget: crate::roster_recommendation::MAX_CORE_BUDGET,
            protected_skill_ids: BTreeSet::new(),
        },
    ) {
        Ok(recommendation) => recommendation,
        Err(error) => {
            if let Some(required) =
                error.downcast_ref::<crate::roster_recommendation::MinimumCoreBudgetRequired>()
            {
                return Ok(Some(json!({
                    "supported": false,
                    "reason": "minimum_core_budget_required",
                    "agent": required.agent.id(),
                    "minimum_core_budget": required.minimum_core_budget,
                    "core_budget": required.core_budget,
                    "maximum_core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
                    "files_changed": false,
                    "state_files_changed": false
                })));
            }
            if let Some(required) = error.downcast_ref::<
                crate::roster_recommendation::PhysicalMutationIdentityRescanRequired,
            >() {
                return Ok(Some(json!({
                    "supported": false,
                    "reason": "physical_mutation_identity_rescan_required",
                    "reason_code": "snapshot_missing_physical_mutation_identity",
                    "snapshot_id": scan_id,
                    "placement_id": required.placement_id,
                    "files_changed": false,
                    "state_files_changed": false,
                    "next_action": "scan",
                    "action": physical_identity_rescan_action()
                })));
            }
            return Ok(Some(json!({
                "supported": false,
                "reason": "automatic_roster_selection_unavailable",
                "detail": error.to_string(),
                "snapshot_id": scan_id
            })));
        }
    };
    let selection_evidence = roster_selection_evidence(&recommendation);
    let supported =
        crate::roster_plan::exclude_unpreservable_demotions(scan, recommendation.changes.clone())?;
    let agents = recommendation
        .agents
        .iter()
        .map(|agent| {
            let unchanged_blocked_count = supported
                .exclusions
                .iter()
                .filter(|exclusion| exclusion.agent == agent.agent.id())
                .count();
            let preview = agent
                .core_selections
                .iter()
                .take(5)
                .map(|selection| {
                    json!({
                        "skill_id": selection.skill_id,
                        "name": selection.name,
                        "reason": selection.reason,
                        "evidence_scope": selection.evidence_scope,
                        "evidence_agents": selection.evidence_agents
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "agent": agent.agent.id(),
                "before_default_exposure": agent.before_default_exposure,
                "unique_skill_count": agent.unique_skill_count,
                "proposed_core_count": agent.core_count,
                "proposed_on_demand_count": agent.on_demand_count.saturating_sub(unchanged_blocked_count),
                "unchanged_blocked_count": unchanged_blocked_count,
                "positive_signal_count": agent.positive_signal_count,
                "direct_signal_count": agent.direct_signal_count,
                "cross_agent_signal_count": agent.cross_agent_signal_count,
                "fallback_core_count": agent.fallback_core_count,
                "core_preview": preview,
                "core_preview_truncated": agent.core_selections.len() > 5
            })
        })
        .collect::<Vec<_>>();
    let blocked_change_count = supported.exclusions.len();
    if decision_facts_only && blocked_change_count > 0 {
        return Ok(Some(json!({"supported": false})));
    }
    let blocked_changes = supported
        .exclusions
        .iter()
        .take(5)
        .map(crate::roster_plan::blocked_change_json)
        .collect::<Vec<_>>();
    if blocked_change_count > 0 {
        let blocked_skills = blocked_skill_planning(&supported.exclusions, full);
        let blocked_reasons = supported
            .exclusions
            .iter()
            .map(|exclusion| exclusion.reason)
            .collect::<BTreeSet<_>>();
        let package_variants_only = blocked_reasons.len() == 1
            && blocked_reasons
                .contains("multiple_package_fingerprints_require_explicit_preservation");
        let source_dependency = supported
            .exclusions
            .iter()
            .any(|exclusion| exclusion.reason == "non_agent_source_link_depends_on_removal");
        let blocked_skill_ids = supported
            .exclusions
            .iter()
            .map(|exclusion| exclusion.skill_id.as_str())
            .collect::<BTreeSet<_>>();
        let observed_link_targets = scan
            .placements
            .iter()
            .filter(|placement| blocked_skill_ids.contains(placement.skill_id.as_str()))
            .filter_map(|placement| placement.link_target.as_ref())
            .map(|path| path.display().to_string())
            .collect::<BTreeSet<_>>();
        if source_dependency {
            let protect_choice = blocked_core_protection_choice(
                store,
                finding,
                scan,
                &declared_core,
                &blocked_skills,
                state_dir,
                full,
            )?;
            let protection_available = protect_choice["available"].as_bool() == Some(true);
            let source_choice = json!({
                "choice": "preserve_or_retarget_dependent_sources",
                "requires_confirmation": true,
                "dependent_source_paths": blocked_skills.displayed_dependent_source_paths,
                "dependent_source_paths_complete": !blocked_skills.truncated,
                "next": if blocked_skills.truncated {
                    "open the full Finding before changing any dependent source link"
                } else {
                    "after user-approved manual preservation or retargeting, rescan and reopen the new large-Roster Finding; another independent blocker may still fail closed"
                }
            });
            return Ok(Some(json!({
                "supported": false,
                "reason": "source_dependency_blocks_roster_change",
                "decision": "resolve_source_dependency",
                "automatic_change_supported": false,
                "snapshot_id": scan_id,
                "request_field": "finding_roster_changes",
                "default_core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
                "absence_of_usage_evidence": "not_negative_evidence",
                "explicit_only_or_archive_decision_implied": false,
                "agent_count": agents.len(),
                "agents": agents,
                "blocked_change_count": blocked_change_count,
                "blocked_reasons": blocked_reasons,
                "blocked_changes": blocked_changes,
                "blocked_changes_truncated": blocked_change_count > 5,
                "blocked_skill_count": blocked_skills.count,
                "blocked_skills": blocked_skills.items,
                "blocked_skills_truncated": blocked_skills.truncated,
                "dependent_link_targets": observed_link_targets,
                "resolution_choices": [protect_choice, source_choice],
                "after_resolution": {
                    "next": if protection_available {
                        "choose either explicit Core protection or user-approved source-link preservation, then retry from fresh evidence; another independent blocker may still fail closed"
                    } else {
                        "use user-approved source-link preservation, then retry from fresh evidence; another independent blocker may still fail closed"
                    }
                }
            })));
        }
        let mutation_scopes = supported
            .exclusions
            .iter()
            .flat_map(|exclusion| match exclusion.safety_blocker.as_ref() {
                Some(crate::roster_plan::RosterSafetyBlocker::ProviderManaged { .. }) => {
                    vec!["provider_read_only".to_owned()]
                }
                Some(crate::roster_plan::RosterSafetyBlocker::MutationScopeReadOnly {
                    mutation_scopes,
                    ..
                }) => mutation_scopes.clone(),
                _ => Vec::new(),
            })
            .collect::<BTreeSet<_>>();
        let requires_more_than_read_confirmation = mutation_scopes.is_empty()
            || mutation_scopes
                .iter()
                .any(|scope| scope != "untrusted_external");
        if requires_more_than_read_confirmation {
            let protect_choice = blocked_core_protection_choice(
                store,
                finding,
                scan,
                &declared_core,
                &blocked_skills,
                state_dir,
                full,
            )?;
            let mut planning = json!({
                "supported": false,
                "reason": if package_variants_only {
                    "package_fingerprint_variants_block_roster_change"
                } else {
                    "mutation_scope_blocks_roster_change"
                },
                "decision": if package_variants_only {
                    "review_package_fingerprint_variants"
                } else {
                    "choose_mutable_placements_or_keep_unchanged"
                },
                "decision_code": if package_variants_only {
                    "protect_package_variants_or_keep_unchanged"
                } else {
                    "protect_read_only_skills_or_keep_unchanged"
                },
                "automatic_change_supported": false,
                "snapshot_id": scan_id,
                "request_field": "finding_roster_changes",
                "default_core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
                "absence_of_usage_evidence": "not_negative_evidence",
                "explicit_only_or_archive_decision_implied": false,
                "agent_count": agents.len(),
                "agents": agents,
                "blocked_change_count": blocked_change_count,
                "blocked_reasons": blocked_reasons,
                "blocked_changes": blocked_changes,
                "blocked_changes_truncated": blocked_change_count > 5,
                "blocked_skill_count": blocked_skills.count,
                "blocked_skills": blocked_skills.items,
                "blocked_skills_truncated": blocked_skills.truncated,
                "resolution_choices": [protect_choice],
                "next": if package_variants_only {
                    "These Skill identities have different package fingerprints. Keep every package unchanged through the typed Core-protection choice, or review the packages before proposing another change; read permission does not resolve the content differences."
                } else {
                    "Read permissions do not make read-only placements mutable. Review all blocked reasons and keep these Skills unchanged as Core through the typed protection choice. Rescan only when scope facts are missing."
                }
            });
            if !package_variants_only {
                planning["mutation_scopes"] = json!(mutation_scopes);
            }
            return Ok(Some(planning));
        }
        let untrusted_external_skill_ids = supported
            .exclusions
            .iter()
            .filter(|exclusion| is_untrusted_external_blocker(exclusion))
            .map(|exclusion| exclusion.skill_id.clone())
            .collect::<BTreeSet<_>>();
        let source_confirmation_finding = source_confirmation_finding_reference(
            store,
            finding,
            scan_id,
            &untrusted_external_skill_ids,
        )?;
        return Ok(Some(json!({
            "supported": false,
            "reason": "trusted_canonical_sources_required",
            "decision": "confirm_trusted_source_roots",
            "decision_semantics": "confirm_exact_local_read_permission",
            "permission_scope": "exact_local_read_only",
            "content_endorsed": false,
            "evidence_quality_changed": false,
            "governance_authorized": false,
            "plan_apply_authorized": false,
            "automatic_change_supported": false,
            "snapshot_id": scan_id,
            "request_field": "finding_roster_changes",
            "default_core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
            "absence_of_usage_evidence": "not_negative_evidence",
            "explicit_only_or_archive_decision_implied": false,
            "agent_count": agents.len(),
            "agents": agents,
            "blocked_change_count": blocked_change_count,
            "blocked_reasons": blocked_reasons,
            "blocked_changes": blocked_changes,
            "blocked_changes_truncated": blocked_change_count > 5,
            "blocked_skill_count": blocked_skills.count,
            "blocked_skills": blocked_skills.items,
            "blocked_skills_truncated": blocked_skills.truncated,
            "observed_link_targets": observed_link_targets,
            "source_confirmation_finding": source_confirmation_finding,
            "after_confirmation": {
                "repeatable_option": "--source-root",
                "value": "absolute canonical source directory",
                "next": if blocked_reasons.contains("multiple_package_fingerprints_require_explicit_preservation") {
                    "rescan and reopen the new large-Roster Finding; read confirmation does not resolve the independent package-preservation blockers"
                } else {
                    "rescan and reopen the new large-Roster Finding"
                }
            }
        })));
    }
    if let Err(error) = crate::roster_plan::derive(scan, state_dir, &supported.changes) {
        if let Some(conflict) =
            error.downcast_ref::<crate::roster_plan::RosterLibraryTargetClaimConflict>()
        {
            if decision_facts_only {
                return Ok(Some(json!({"supported": false})));
            }
            let conflict_skill_ids = conflict
                .claimants
                .iter()
                .map(|claimant| claimant.skill_id.clone())
                .collect::<BTreeSet<_>>();
            let protected_validation = validated_core_protection(
                finding,
                scan,
                &declared_core,
                state_dir,
                conflict_skill_ids.clone(),
                vec![conflict.clone()],
            );
            let validated = protected_validation.as_ref().ok();
            let protected_skill_ids = validated
                .map(|validated| validated.protected_skill_ids.clone())
                .unwrap_or(conflict_skill_ids)
                .into_iter()
                .collect::<Vec<_>>();
            let conflicts = validated
                .map(|validated| validated.library_target_conflicts.as_slice())
                .unwrap_or_else(|| std::slice::from_ref(conflict));
            let disclosure = library_target_claim_disclosure(store, finding, conflicts, full)?;
            let protected_ids_complete = (full || protected_skill_ids.len() <= 10)
                && disclosure["claimants_complete"].as_bool() == Some(true);
            let protection_available = protected_validation.is_ok() && protected_ids_complete;
            let conflict_kind = if conflicts.len() == 1 {
                if conflicts[0].has_one_logical_name() {
                    "same_name_library_target_conflict"
                } else {
                    "library_target_name_normalization_conflict"
                }
            } else {
                "library_target_claim_conflicts"
            };
            let conflict_reason_code = if conflicts.len() == 1 {
                if conflicts[0].has_one_logical_name() {
                    "same_name_variants_require_explicit_preservation"
                } else {
                    "normalized_names_claim_one_library_target"
                }
            } else {
                "multiple_library_targets_require_explicit_preservation"
            };
            let unavailable_detail = protected_validation.as_ref().err().map(ToString::to_string);
            let mut protection_choice = json!({
                "choice": "protect_library_target_claimants_as_core",
                "requires_confirmation": true,
                "available": protection_available,
                "protected_skill_ids": if protected_ids_complete { json!(protected_skill_ids) } else { Value::Null },
                "protected_skill_ids_complete": protected_ids_complete,
                "plan_request_template_available": protection_available,
                "unavailable_detail": unavailable_detail,
                "next": if protection_available {
                    "after user confirmation, submit the exact Plan request to preserve every conflicting variant as Core"
                } else if !protected_ids_complete {
                    "open the full Finding before constructing a complete protected-Skill Plan request"
                } else {
                    "the production Recommendation or Plan preconditions reject this Core protection set"
                }
            });
            if protection_available {
                protection_choice["plan_request_template"] = json!({
                    "schema_version": 1,
                    "finding_roster_changes": [{
                        "finding_id": finding.id,
                        "core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
                        "protected_skill_ids": protected_skill_ids
                    }]
                });
            }
            return Ok(Some(json!({
                "supported": false,
                "reason": conflict_kind,
                "reason_code": conflict_reason_code,
                "automatic_change_supported": false,
                "snapshot_id": scan_id,
                "request_field": "finding_roster_changes",
                "library_target_claims": disclosure,
                "resolution_choices": [protection_choice],
                "files_changed": false,
                "state_files_changed": false
            })));
        }
        // A rejected candidate Plan is a Finding blocker, not a failure to
        // read the Report. Keep the same decision in summary and detail views.
        return Ok(Some(json!({
            "supported": false,
            "reason": "roster_plan_preconditions_failed",
            "detail": error.to_string(),
            "snapshot_id": scan_id,
            "files_changed": false,
            "state_files_changed": false
        })));
    }
    if decision_facts_only {
        return Ok(Some(json!({
            "supported": true,
            "reason": "large_default_roster",
            "snapshot_id": scan_id,
            "request_template": {
                "schema_version": 1,
                "finding_roster_changes": [{
                    "finding_id": finding.id,
                    "core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
                    "protected_skill_ids": []
                }]
            }
        })));
    }
    Ok(Some(json!({
        "supported": true,
        "decision": "right_size_default_rosters",
        "snapshot_id": scan_id,
        "request_field": "finding_roster_changes",
        "default_core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
        "allowed_core_budget": {
            "minimum": recommendation.agents.iter().map(|agent| agent.core_selections.iter().filter(|selection| selection.evidence_scope == "forced").count()).max().unwrap_or(1).max(1),
            "maximum": crate::roster_recommendation::MAX_CORE_BUDGET,
            "unit": "skills_per_affected_agent"
        },
        "selection_policy": [
            "immutable_placement_retained",
            "native_visibility_unknown_retained",
            "protected_by_request",
            "declared_core",
            "skillroster_bootstrap",
            "target_agent_usage_evidence",
            "cross_agent_same_skill_usage_evidence",
            "stable_fallback"
        ],
        "absence_of_usage_evidence": "not_negative_evidence",
        "automatic_target_states": ["core", "on_demand"],
        "explicit_only_or_archive_decision_implied": false,
        "selection_evidence": selection_evidence.summary,
        "uncertainty": selection_evidence.uncertainty,
        "blocked_change_count": blocked_change_count,
        "blocked_changes": blocked_changes,
        "blocked_changes_truncated": blocked_change_count > 5,
        "agent_count": agents.len(),
        "agents": agents,
        "plan_request_template": {
            "schema_version": 1,
            "finding_roster_changes": [{
                "finding_id": finding.id,
                "core_budget": crate::roster_recommendation::MAX_CORE_BUDGET,
                "protected_skill_ids": []
            }]
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_core_coverage_exception_never_covers_package_operations_or_library_changes() {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("home");
        let package = home.join(".codex/skills/retained");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("SKILL.md"),
            "---\nname: retained\n---\nfixture\n",
        )
        .unwrap();
        let mut options = ScanOptions::for_home(&home);
        options.include_session_evidence = false;
        let mut scan = scan::scan(&options).unwrap();
        let placement = scan.placements.first_mut().unwrap();
        placement.fingerprint_completeness = scan::FingerprintCompleteness::Unreadable;
        placement.physical_directory = Some(temp.path().join("physical-package"));
        let placement = placement.clone();
        let unrelated = temp.path().join("unrelated");
        let mut plan = PreparedPlan {
            id: PlanId::new().to_string(),
            scan_id: ScanId::new().to_string(),
            evidence_ids: vec![],
            digest: "sha256:fixture".into(),
            operations: vec![Operation::CreateDirectory {
                target: unrelated.clone(),
                expected_fingerprint: "missing".into(),
            }],
            roster_changes: vec![change::RosterChange {
                agent: "codex".into(),
                skill_id: placement.skill_id.clone(),
                state: "core".into(),
            }],
            source_updates: vec![],
            library_changes: vec![],
            path_projections: vec![],
            approved_roots: vec![home],
            state_dir: temp.path().join("state"),
        };
        assert!(validate_governance_fingerprint_completeness(&plan, &scan).is_ok());
        for affected in [
            placement.directory.clone(),
            placement.entrypoint.clone(),
            placement.root.clone(),
            placement.physical_directory.clone().unwrap(),
        ] {
            for operation in [
                Operation::CreateDirectory {
                    target: affected.clone(),
                    expected_fingerprint: "missing".into(),
                },
                Operation::Copy {
                    source: affected,
                    target: unrelated.clone(),
                    expected_fingerprint: "missing".into(),
                },
            ] {
                plan.operations = vec![operation];
                assert!(validate_governance_fingerprint_completeness(&plan, &scan).is_err());
            }
        }
        plan.operations = vec![Operation::CreateDirectory {
            target: unrelated,
            expected_fingerprint: "missing".into(),
        }];
        plan.library_changes = vec![change::LibraryChangeAction {
            skill_id: placement.skill_id,
            canonical_placement_id: placement.id.clone(),
            placement_ids: vec![placement.id],
            requested_state: "managed".into(),
            canonical_path: placement.directory,
            library_path: None,
        }];
        assert!(validate_governance_fingerprint_completeness(&plan, &scan).is_err());
    }
}
