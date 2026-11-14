use super::*;

pub(super) fn find_command(
    store: &StateStore,
    state_dir: &Path,
    args: &crate::cli::FindArgs,
) -> Result<(Value, Vec<SuggestedAction>)> {
    let task = args.task.as_str();
    let hints = args.hints.as_slice();
    let limit = usize::from(args.limit);
    let load = args.load;
    let variant_skill_id = args.variant_skill_id.as_deref();
    let variant_placement_id = args.variant_placement_id.as_deref();
    let required_snapshot_id = args.require_snapshot.as_deref();
    if (variant_skill_id.is_some() || variant_placement_id.is_some()) && !load {
        return Err(SkillLoadBlocked {
            reason: "variant_selector_requires_load",
            skill_id: variant_skill_id.unwrap_or_default().to_owned(),
            skill_name: "Top-1".into(),
            path: None,
            roster_state: "unknown".into(),
            mutation_scopes: Vec::new(),
            expected_digest: None,
            actual_digest: None,
            retry_argv: None,
            retrieval: None,
        }
        .into());
    }
    let retrieval_hints = normalize_retrieval_hints(hints);
    let (scan_id, scan) = latest_scan(store)?;
    if let Some(expected) = required_snapshot_id {
        let expected = ScanId::parse(expected.to_owned())?;
        if expected != scan_id {
            return Err(FindSnapshotChanged {
                expected_snapshot_id: expected,
                actual_snapshot_id: scan_id.clone(),
                retry_argv: find_action_argv(
                    task,
                    &retrieval_hints,
                    FindActionKind::InspectVariants { limit },
                    Some(&scan_id),
                ),
            }
            .into());
        }
    }
    require_content_identity(&scan)?;
    let retrieval_query = crate::query::RetrievalQuery::from_task_and_hints(
        task,
        retrieval_hints.iter().map(String::as_str),
    );
    let candidate_search_text = crate::query::candidate_search_text(retrieval_query.text());
    let mut candidate_ids = store
        .search_skill_ids_in_snapshot(&candidate_search_text, scan.skills.len(), &scan_id)?
        .into_iter()
        .map(|id| id.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    let mut routable_ids = scan
        .skills
        .iter()
        .map(|skill| skill.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut archived_ids = BTreeSet::new();
    for skill_id in routable_ids.clone() {
        let id = SkillId::parse(skill_id.clone())?;
        let states = store.roster_states_for_skill(&id)?;
        if !states.is_empty() && states.iter().all(|state| *state == RosterState::Archived) {
            routable_ids.remove(&skill_id);
            archived_ids.insert(skill_id);
        }
    }
    if crate::query::contains_cjk(retrieval_query.text()) {
        candidate_ids.extend(routable_ids.iter().cloned());
    }
    candidate_ids.retain(|skill_id| routable_ids.contains(skill_id));
    let pool_limit = if retrieval_hints.is_empty() {
        limit
    } else {
        usize::from(crate::cli::MAX_FIND_RESULTS)
    };
    let augmented_result = crate::query::find_matching_with_evidence(
        &scan,
        &retrieval_query,
        pool_limit,
        Some(&candidate_ids),
        Some(&routable_ids),
    );
    let task_exclusion_effects = augmented_result.task_exclusion_effects;
    let augmented_matches = augmented_result.matches;
    let ranking_strategy = if retrieval_hints.is_empty() {
        "single_lexical_channel"
    } else {
        "task_hint_reciprocal_rank_fusion"
    };
    let mut matches = if retrieval_hints.is_empty() {
        augmented_matches
    } else {
        let task_query =
            crate::query::RetrievalQuery::from_task_and_hints(task, std::iter::empty::<&str>());
        let task_matches = crate::query::find_matching(
            &scan,
            &task_query,
            pool_limit,
            Some(&candidate_ids),
            Some(&routable_ids),
        );
        crate::query::fuse_retrieval_channels(task_matches, augmented_matches, &task_query, limit)
    };
    let mut warnings = Vec::new();
    let mut observations = content_choice::FindReadObservations::default();
    let mut rescan_required = false;
    for found in &mut matches {
        let mut drifted_variants = 0_usize;
        if found.variants.is_empty() {
            let skill_id = SkillId::parse(found.skill_id.clone())?;
            found.roster_state = current_roster_state(&store.roster_states_for_skill(&skill_id)?);
            let current = current_readable_skill_paths_with_observations(
                &scan,
                state_dir,
                &found.skill_id,
                &mut observations,
            )?;
            found.paths = current.paths;
            drifted_variants += usize::from(current.drifted);
        } else {
            for variant in &mut found.variants {
                let skill_id = SkillId::parse(variant.skill_id.clone())?;
                variant.roster_state =
                    current_roster_state(&store.roster_states_for_skill(&skill_id)?);
                let current = current_readable_skill_paths_with_observations(
                    &scan,
                    state_dir,
                    &variant.skill_id,
                    &mut observations,
                )?;
                let group_paths = scan
                    .placements
                    .iter()
                    .filter(|placement| placement.skill_id == variant.skill_id)
                    .filter(|placement| {
                        scan.observed_skill_for_placement(placement)
                            .is_some_and(|skill| {
                                skill.name.trim().to_lowercase() == found.name.trim().to_lowercase()
                            })
                    })
                    .map(|placement| placement.entrypoint.display().to_string())
                    .collect::<BTreeSet<_>>();
                variant.paths = current
                    .paths
                    .into_iter()
                    .filter(|path| group_paths.contains(path))
                    .collect();
                drifted_variants += usize::from(current.drifted);
            }
            if let Some(representative) = found
                .variants
                .iter()
                .find(|variant| variant.skill_id == found.skill_id)
            {
                found.paths = representative.paths.clone();
                found.roster_state = representative.roster_state.clone();
            }
        }
        if drifted_variants > 0 {
            rescan_required = true;
            let variant_count = found.variants.len().max(1);
            warnings.push(if drifted_variants == variant_count {
                format!(
                    "{} has placement drift relative to the latest Snapshot identity or fingerprint; run skillroster scan",
                    found.name
                )
            } else {
                format!(
                    "{} has {drifted_variants} same-name variant(s) with path drift; run skillroster scan",
                    found.name
                )
            });
        }
    }
    let (mut actions, variant_rescan_required) = bind_variant_findings(
        store,
        state_dir,
        &scan_id,
        &scan,
        &routable_ids,
        &mut matches,
        &mut observations,
    )?;
    rescan_required |= variant_rescan_required;
    let cjk_hint_required = retrieval_hints.is_empty() && crate::query::contains_cjk(task);
    if load && matches.is_empty() {
        let archived_match = crate::query::find_matching(
            &scan,
            &retrieval_query,
            1,
            Some(&archived_ids),
            Some(&archived_ids),
        )
        .into_iter()
        .next();
        return Err(SkillLoadBlocked {
            reason: if archived_match.is_some() {
                "archived_skill_not_routable"
            } else {
                "no_routable_match"
            },
            skill_id: archived_match
                .as_ref()
                .map(|matched| matched.skill_id.clone())
                .unwrap_or_default(),
            skill_name: archived_match
                .as_ref()
                .map(|matched| matched.name.clone())
                .unwrap_or_else(|| "Top-1".into()),
            path: None,
            roster_state: archived_match
                .map(|_| "archived".to_owned())
                .unwrap_or_else(|| "unassigned".into()),
            mutation_scopes: Vec::new(),
            expected_digest: None,
            actual_digest: None,
            retry_argv: None,
            retrieval: Some(blocked_retrieval(
                &scan_id,
                task,
                &retrieval_hints,
                &matches,
            )),
        }
        .into());
    }
    // Selector validity is independent of relevance and cannot be repaired by
    // a better hint. Validate it before the evidence gate, without loading text.
    let selected_match = if load {
        matches
            .first()
            .map(|ranked| {
                select_top_match_variant(
                    store,
                    &scan,
                    &routable_ids,
                    ranked,
                    variant_skill_id,
                    variant_placement_id,
                )
            })
            .transpose()
            .map_err(|error| {
                attach_blocked_retrieval(error, &scan_id, task, &retrieval_hints, &matches)
            })?
    } else {
        None
    };
    if load {
        let weak_top_match = matches
            .first()
            .filter(|ranked| !crate::query::has_strong_verified_load_evidence(ranked));
        let weak_match_reason = weak_top_match.and_then(|ranked| {
            // A known unsafe unique placement must recover its Snapshot before
            // refining lexical relevance. This check only rejects; exact loading
            // still performs the fresh verifier after selection evidence passes.
            if ranked.variant_count == 1 {
                if let Some(reason) = find_placement_readiness_blockers(
                    &scan,
                    state_dir,
                    &ranked.skill_id,
                    &mut observations,
                )
                .first()
                {
                    return Some(*reason);
                }
            }
            find_selection_evidence_blocker(ranked, task, &retrieval_hints)
        });
        if let (Some(ranked), Some(reason)) = (weak_top_match, weak_match_reason) {
            return Err(SkillLoadBlocked {
                reason,
                skill_id: ranked.skill_id.clone(),
                skill_name: ranked.name.clone(),
                path: None,
                roster_state: ranked.roster_state.clone(),
                mutation_scopes: ranked
                    .mutation_scopes
                    .iter()
                    .map(|scope| scope.id().to_owned())
                    .collect(),
                expected_digest: None,
                actual_digest: None,
                retrieval: Some(blocked_retrieval(
                    &scan_id,
                    task,
                    &retrieval_hints,
                    &matches,
                )),
                retry_argv: Some(find_action_argv(
                    task,
                    &retrieval_hints,
                    FindActionKind::InspectVariants {
                        limit: limit.max(5),
                    },
                    Some(&scan_id),
                )),
            }
            .into());
        }
    }
    let loaded_skill = if load && !matches.is_empty() {
        let ranked = &matches[0];
        let ambiguity_retry_argv =
            (ranked.variant_count != 1 && variant_skill_id.is_none()).then(|| {
                find_action_argv(
                    task,
                    &retrieval_hints,
                    FindActionKind::InspectVariants { limit },
                    Some(&scan_id),
                )
            });
        let selected = selected_match
            .as_ref()
            .expect("nonempty load selection validated above");
        let retry_argv = find_action_argv(
            task,
            &retrieval_hints,
            FindActionKind::InspectVariants { limit },
            Some(&scan_id),
        );
        let choice = declared_content_choice(&scan, state_dir, &selected.skill_id, 10);
        if variant_placement_id.is_none() && choice["content_selection_required"] == true {
            return Err(error::DeclaredIdentityContentAmbiguous {
                skill_id: selected.skill_id.clone(),
                content_choice: choice,
                retrieval: Some(blocked_retrieval(
                    &scan_id,
                    task,
                    &retrieval_hints,
                    &matches,
                )),
                retry_argv,
            }
            .into());
        }
        let mut loaded = verified_skill_placement_load(
            &scan_id,
            &scan,
            state_dir,
            selected,
            ambiguity_retry_argv,
            variant_placement_id,
        )
        .map_err(|error| {
            attach_blocked_retrieval(error, &scan_id, task, &retrieval_hints, &matches)
        })?;
        if variant_skill_id.is_some() || variant_placement_id.is_some() {
            loaded["selection"]["ranking_evidence_scope"] = json!("ranked_capability_group");
            loaded["selection"]["variant_selection"] = json!({
                "mode": if variant_placement_id.is_some() { "explicit_placement_id" } else { "explicit_skill_id" },
                "requested_skill_id": variant_skill_id,
                "requested_placement_id": variant_placement_id,
                "ranked_capability_name": ranked.name,
                "ranked_group_representative_skill_id": ranked.skill_id,
                "ranked_variant_count": ranked.variant_count,
            });
        }
        Some(loaded)
    } else {
        None
    };
    for found in matches
        .iter()
        .take(3)
        .filter(|found| found.variant_count > 1)
    {
        let next = found.variant_finding.as_ref();
        warnings.push(match next.and_then(|reference| reference.finding_id.as_deref()) {
            Some(finding_id) => format!(
                "{} represents {} same-name Skill variants; inspect Finding {finding_id} before choosing content",
                found.name, found.variant_count
            ),
            None if matches!(
                next.map(|reference| reference.state),
                Some(crate::query::VariantFindingState::SourceConfirmationRequired)
            ) => format!(
                "{} represents {} same-name Skill variants behind unconfirmed source links; inspect the linked source-root Finding before choosing content",
                found.name, found.variant_count
            ),
            None if matches!(
                next.map(|reference| reference.state),
                Some(crate::query::VariantFindingState::IncompletePackage)
            ) => format!(
                "{} has same-name variants with incomplete package fingerprints; inspect inventory findings and per-variant availability before choosing content; rescan alone is not sufficient",
                found.name
            ),
            None if matches!(
                next.map(|reference| reference.state),
                Some(crate::query::VariantFindingState::ReportRequired)
            ) => format!(
                "{} represents {} same-name Skill variants; materialize the current Report before choosing content",
                found.name, found.variant_count
            ),
            None if matches!(
                next.map(|reference| reference.state),
                Some(crate::query::VariantFindingState::RescanRequired)
            ) => format!(
                "{} represents {} same-name Skill variants; refresh the drifted Snapshot before choosing content",
                found.name, found.variant_count
            ),
            _ => format!(
                "{} represents {} same-name Skill variants, but no matching current divergent-content Finding is available",
                found.name, found.variant_count
            ),
        });
    }
    if !load {
        if let Some(found) = matches.first().filter(|found| {
            found.variant_count > 1
                && !matches!(
                    found
                        .variant_finding
                        .as_ref()
                        .map(|reference| reference.state),
                    Some(crate::query::VariantFindingState::RescanRequired)
                        | Some(crate::query::VariantFindingState::SourceConfirmationRequired)
                )
        }) {
            actions.extend(explicit_variant_load_actions(
                task,
                &retrieval_hints,
                found,
                Some(&scan_id),
                &scan,
                state_dir,
                &mut observations,
            ));
        }
    }
    if cjk_hint_required {
        warnings.push(
            "Find is lexical and the task contains CJK text; retry with one concise English capability paraphrase via --hint if relevant Skills use English metadata"
                .into(),
        );
    }
    if matches.is_empty() && !cjk_hint_required {
        warnings.push(
            "No lexical Skill match was found; retry once with concrete capability, tool, or operation terms via --hint"
                .into(),
        );
    }
    warnings.sort();
    warnings.dedup();
    let mut result = json!({
        "snapshot_id": scan_id,
        "task": task,
        "task_exclusions": retrieval_query.excluded_task_phrases(),
        "task_exclusion_effects": task_exclusion_effects,
        "retrieval_hints": retrieval_hints,
        "ranking_strategy": ranking_strategy,
        "matches": matches,
        "rescan_required": rescan_required,
        "warnings": warnings,
        "files_changed": false
    });
    bind_find_path_availability(&scan, state_dir, &mut result, &mut observations);
    bind_find_variant_path_availability(&scan, state_dir, &mut result, &mut observations);
    bind_content_choices(
        store,
        state_dir,
        &scan_id,
        &scan,
        task,
        &retrieval_hints,
        &matches,
        !load,
        &mut result,
        &mut actions,
        &mut observations,
    )?;
    for (found, value) in matches
        .iter()
        .zip(result["matches"].as_array_mut().unwrap())
    {
        if found.variant_count > 1 {
            // Compare the complete routable group, even when the displayed
            // variant preview is truncated.
            let ids = scan
                .routing_skills()
                .filter(|skill| {
                    skill.name.trim().to_lowercase() == found.name.trim().to_lowercase()
                })
                .filter(|skill| routable_ids.contains(&skill.id))
                .map(|skill| skill.id.as_str())
                .collect::<BTreeSet<_>>();
            value["entrypoint_comparison"] = entrypoint_comparison(
                scan.placements
                    .iter()
                    .filter(|placement| ids.contains(placement.skill_id.as_str()))
                    .filter(|placement| {
                        scan.observed_skill_for_placement(placement)
                            .is_some_and(|skill| {
                                skill.name.trim().to_lowercase() == found.name.trim().to_lowercase()
                            })
                    }),
            );
        }
    }
    if !load {
        for (found, value) in matches
            .iter()
            .zip(result["matches"].as_array_mut().unwrap())
        {
            let evidence_reason = find_selection_evidence_blocker(found, task, &retrieval_hints);
            let mut reasons = Vec::new();
            if found.variant_count > 1 {
                reasons.push("same_name_variants_ambiguous");
            }
            if let Some(reason) = evidence_reason {
                reasons.push(reason);
            }
            if value["content_choice"]["content_selection_required"] == true {
                reasons.push("declared_identity_content_ambiguous");
            }
            if found.variant_count == 1 {
                reasons.extend(find_placement_readiness_blockers(
                    &scan,
                    state_dir,
                    &found.skill_id,
                    &mut observations,
                ));
            }
            if found.paths.is_empty() {
                reasons.push("current_path_unavailable");
            }
            value["load_readiness"] = json!({
                "state": if reasons.is_empty() { "verification_required" } else { "blocked" },
                "reason_codes": reasons,
                "direct_selection_evidence": crate::query::has_strong_verified_load_evidence(found),
                "verification_performed": false,
                "selection_guidance": "Ranking is not a capability decision. Inspect the candidate evidence, independently choose the intended capability, and author a faithful hint if direct selection evidence is missing; never derive an exact-name hint solely from Top-1. Exact content verification still runs only on --load."
            });
        }
    }
    if let Some(loaded_skill) = loaded_skill {
        result["loaded_skill"] = loaded_skill;
    }
    // Find derives current paths and may load content after its initial
    // Snapshot check. Re-freeze durable permissions before returning so a
    // persistent revoke or filesystem-identity drift observed during the read
    // discards those derived facts instead of returning them as authorized.
    require_loaded_snapshot_current(store, &scan_id, &scan)?;
    Ok((result, actions))
}

// Advisory evidence only: mirror the verifier's placement preference without
// returning content or treating these cached observations as authorization.
pub(super) fn find_placement_readiness_blockers(
    scan: &ScanResult,
    state_dir: &Path,
    skill_id: &str,
    observations: &mut content_choice::FindReadObservations,
) -> Vec<&'static str> {
    let placements = scan
        .placements
        .iter()
        .filter(|p| p.skill_id == skill_id)
        .collect::<Vec<_>>();
    let preferred = placements
        .iter()
        .copied()
        .filter(|p| {
            p.mutation_scope
                .is_some_and(|s| s != scan::MutationScope::UntrustedExternal)
                && p.fingerprint_completeness == scan::FingerprintCompleteness::Complete
                && p.entrypoint_digest.is_some()
        })
        .min_by_key(|p| (!p.default_exposed, &p.entrypoint));
    if let Some(placement) = preferred {
        let state = content_choice::placement_read_state_with_observations(
            scan,
            state_dir,
            placement,
            observations,
        );
        let exact_copy_available = state == "available"
            || placements.iter().any(|copy| {
                copy.content_digest == placement.content_digest
                    && copy.content_identity_digest == placement.content_identity_digest
                    && copy.entrypoint_digest == placement.entrypoint_digest
                    && content_choice::placement_read_state_with_observations(
                        scan,
                        state_dir,
                        copy,
                        observations,
                    ) == "available"
            });
        return if exact_copy_available {
            Vec::new()
        } else {
            vec![state]
        };
    }
    placements
        .into_iter()
        .map(|p| {
            content_choice::placement_read_state_with_observations(scan, state_dir, p, observations)
        })
        .filter(|state| *state != "available")
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn find_selection_evidence_blocker(
    ranked: &crate::query::FindMatch,
    task: &str,
    hints: &[String],
) -> Option<&'static str> {
    if crate::query::has_strong_verified_load_evidence(ranked) {
        None
    } else if hints.is_empty() && crate::query::contains_cjk(task) {
        Some("cjk_hint_required_for_weak_match")
    } else if !hints.is_empty() {
        Some("hint_direct_selection_evidence_required")
    } else {
        Some("direct_selection_evidence_required")
    }
}

#[derive(Clone, Copy)]
pub(super) enum FindActionKind<'a> {
    InspectVariants { limit: usize },
    LoadVariant { skill_id: &'a str },
    LoadPlacement { placement_id: &'a str },
}

pub(super) fn find_action_argv(
    task: &str,
    hints: &[String],
    kind: FindActionKind<'_>,
    required_snapshot_id: Option<&ScanId>,
) -> Vec<String> {
    let mut argv = vec!["find".to_owned()];
    for hint in hints {
        argv.extend(["--hint".to_owned(), hint.clone()]);
    }
    match kind {
        FindActionKind::InspectVariants { limit } => {
            argv.extend(["--limit".to_owned(), limit.to_string()]);
        }
        FindActionKind::LoadPlacement { placement_id } => argv.extend([
            "--load".to_owned(),
            "--limit".to_owned(),
            "1".to_owned(),
            "--variant-placement-id".to_owned(),
            placement_id.to_owned(),
        ]),
        FindActionKind::LoadVariant { skill_id } => argv.extend([
            "--load".to_owned(),
            "--limit".to_owned(),
            "1".to_owned(),
            "--variant-skill-id".to_owned(),
            skill_id.to_owned(),
        ]),
    }
    if let Some(snapshot_id) = required_snapshot_id {
        argv.extend(["--require-snapshot".to_owned(), snapshot_id.to_string()]);
    }
    argv.push("--json".to_owned());
    argv.extend(["--".to_owned(), task.to_owned()]);
    argv
}

pub(super) fn explicit_variant_load_actions(
    task: &str,
    hints: &[String],
    ranked: &crate::query::FindMatch,
    required_snapshot_id: Option<&ScanId>,
    scan: &ScanResult,
    state_dir: &Path,
    observations: &mut content_choice::FindReadObservations,
) -> Vec<SuggestedAction> {
    if find_selection_evidence_blocker(ranked, task, hints).is_some() {
        return Vec::new();
    }
    ranked
        .variants
        .iter()
        .filter(|variant| !variant.paths.is_empty())
        .filter(|variant| {
            find_placement_readiness_blockers(scan, state_dir, &variant.skill_id, observations)
                .is_empty()
        })
        .map(|variant| {
            let mut argv = vec!["skillroster".to_owned()];
            argv.extend(find_action_argv(
                task,
                hints,
                FindActionKind::LoadVariant {
                    skill_id: &variant.skill_id,
                },
                required_snapshot_id,
            ));
            SuggestedAction {
                action: "load_exact_variant_for_comparison".into(),
                description: "load_exact_variant_for_comparison".into(),
                argv,
                mutates: false,
                requires_confirmation: false,
                reason_code: "same_name_variant_explicit_read_available".into(),
            }
        })
        .collect()
}

pub(super) fn bind_find_variant_path_availability(
    scan: &ScanResult,
    state_dir: &Path,
    result: &mut Value,
    observations: &mut content_choice::FindReadObservations,
) {
    for found in result["matches"].as_array_mut().into_iter().flatten() {
        for variant in found["variants"].as_array_mut().into_iter().flatten() {
            let Some(skill_id) = variant["skill_id"].as_str() else {
                continue;
            };
            let mut reasons = BTreeSet::new();
            let mut placements = Vec::new();
            let mut available_count = 0;
            for placement in scan.placements.iter().filter(|p| p.skill_id == skill_id) {
                let reason = content_choice::placement_read_state_with_observations(
                    scan,
                    state_dir,
                    placement,
                    observations,
                );
                if reason == "available" {
                    available_count += 1;
                } else {
                    reasons.insert(reason);
                }
                placements.push(json!({
                    "placement_id": placement.id,
                    "read_state": reason,
                    "fingerprint_completeness": placement.fingerprint_completeness,
                    "entrypoint_read_error": placement.entrypoint_read_error,
                    "detail": placement.fingerprint_detail,
                }));
            }
            let count = placements.len();
            placements.truncate(20);
            variant["path_availability"] = json!({
                "state": if available_count > 0 { "available" } else { "unavailable" },
                "reason_codes": reasons,
                "available_placement_count": available_count,
                "placement_count": count,
                "placements": placements,
                "placements_truncated": count > 20,
            });
        }
    }
}

pub(super) fn bind_variant_findings(
    store: &StateStore,
    state_dir: &Path,
    scan_id: &ScanId,
    scan: &ScanResult,
    routable_ids: &BTreeSet<String>,
    matches: &mut [crate::query::FindMatch],
    observations: &mut content_choice::FindReadObservations,
) -> Result<(Vec<SuggestedAction>, bool)> {
    let report = store
        .latest_report()?
        .filter(|report| report.scan_id == *scan_id);
    let mut actions = Vec::new();
    let mut action_argvs = BTreeSet::new();
    let mut rescan_required = false;

    for (index, found) in matches
        .iter_mut()
        .enumerate()
        .filter(|(_, found)| found.variant_count > 1)
    {
        let capability_name = found.name.trim().to_lowercase();
        let variant_ids = scan
            .routing_skills()
            .filter(|skill| skill.name.trim().to_lowercase() == capability_name)
            .filter(|skill| routable_ids.contains(&skill.id))
            .map(|skill| skill.id.clone())
            .collect::<BTreeSet<_>>();
        let mut variant_drifted = false;
        let mut source_confirmation_variant_ids = BTreeSet::new();
        for skill_id in &variant_ids {
            let current = current_readable_skill_paths_with_observations(
                scan,
                state_dir,
                skill_id,
                observations,
            )?;
            variant_drifted |= current.drifted;
            if current.source_confirmation_required {
                source_confirmation_variant_ids.insert(skill_id.clone());
            }
        }
        rescan_required |= variant_drifted;
        let (reference, suggested) = if variant_drifted {
            let suggested = action(
                "refresh_drifted_snapshot",
                &["scan", "--summary", "--json"],
                false,
                false,
                "routable_variant_drift_detected",
            );
            (
                crate::query::VariantFindingReference {
                    state: crate::query::VariantFindingState::RescanRequired,
                    reason_code: crate::query::VariantFindingReason::RoutableVariantDriftDetected,
                    snapshot_id: scan_id.to_string(),
                    report_id: None,
                    finding_id: None,
                    resolution: None,
                    argv: suggested.argv.clone(),
                },
                suggested,
            )
        } else if scan.placements.iter().any(|placement| {
            variant_ids.contains(&placement.skill_id)
                && placement.fingerprint_completeness != scan::FingerprintCompleteness::Complete
                && placement.mutation_scope != Some(scan::MutationScope::UntrustedExternal)
        }) {
            let suggested = action(
                "inspect_incomplete_variant_inventory",
                &["report", "--findings", "--category", "inventory", "--json"],
                false,
                false,
                "incomplete_variant_fingerprints",
            );
            (
                crate::query::VariantFindingReference {
                    state: crate::query::VariantFindingState::IncompletePackage,
                    reason_code: crate::query::VariantFindingReason::IncompleteVariantFingerprints,
                    snapshot_id: scan_id.to_string(),
                    report_id: report.as_ref().map(|report| report.id.to_string()),
                    finding_id: None,
                    resolution: Some("inspect_incomplete_package_before_content_choice".into()),
                    argv: suggested.argv.clone(),
                },
                suggested,
            )
        } else if let Some(report) = report.as_ref() {
            if !variant_ids.is_empty() && source_confirmation_variant_ids == variant_ids {
                if let Some(finding_id) = matching_escaping_link_finding_id(
                    store,
                    report,
                    &source_confirmation_variant_ids,
                )? {
                    let suggested = action(
                        "inspect_source_confirmation_finding",
                        &["report", "--finding", &finding_id, "--json"],
                        false,
                        false,
                        "untrusted_same_name_variants_require_source_confirmation",
                    );
                    (
                        crate::query::VariantFindingReference {
                            state: crate::query::VariantFindingState::SourceConfirmationRequired,
                            reason_code: crate::query::VariantFindingReason::UntrustedVariantsRequireSourceConfirmation,
                            snapshot_id: scan_id.to_string(),
                            report_id: Some(report.id.to_string()),
                            finding_id: Some(finding_id),
                            resolution: Some("confirm_trusted_source_roots".into()),
                            argv: suggested.argv.clone(),
                        },
                        suggested,
                    )
                } else {
                    let suggested = action(
                        "inspect_layout_findings",
                        &["report", "--findings", "--category", "layout", "--json"],
                        false,
                        false,
                        "matching_escaping_link_finding_missing",
                    );
                    (
                        crate::query::VariantFindingReference {
                            state: crate::query::VariantFindingState::FindingUnavailable,
                            reason_code: crate::query::VariantFindingReason::MatchingEscapingLinkFindingMissing,
                            snapshot_id: scan_id.to_string(),
                            report_id: Some(report.id.to_string()),
                            finding_id: None,
                            resolution: None,
                            argv: suggested.argv.clone(),
                        },
                        suggested,
                    )
                }
            } else if let Some(finding_id) =
                matching_variant_finding_id(store, report, &variant_ids)?
            {
                let suggested = action(
                    "inspect_variant_finding",
                    &["report", "--finding", &finding_id, "--json"],
                    false,
                    false,
                    "same_snapshot_variant_finding_available",
                );
                (
                    crate::query::VariantFindingReference {
                        state: crate::query::VariantFindingState::Available,
                        reason_code:
                            crate::query::VariantFindingReason::SameSnapshotVariantSetMatched,
                        snapshot_id: scan_id.to_string(),
                        report_id: Some(report.id.to_string()),
                        finding_id: Some(finding_id),
                        resolution: Some("choose_same_name_variant".into()),
                        argv: suggested.argv.clone(),
                    },
                    suggested,
                )
            } else {
                let suggested = action(
                    "inspect_layout_findings",
                    &["report", "--findings", "--category", "layout", "--json"],
                    false,
                    false,
                    "matching_divergent_content_finding_missing",
                );
                (
                    crate::query::VariantFindingReference {
                        state: crate::query::VariantFindingState::FindingUnavailable,
                        reason_code: crate::query::VariantFindingReason::MatchingDivergentContentFindingMissing,
                        snapshot_id: scan_id.to_string(),
                        report_id: Some(report.id.to_string()),
                        finding_id: None,
                        resolution: None,
                        argv: suggested.argv.clone(),
                    },
                    suggested,
                )
            }
        } else {
            let suggested = action(
                "materialize_report",
                &["report", "--summary", "--json"],
                false,
                false,
                "current_snapshot_report_missing",
            );
            (
                crate::query::VariantFindingReference {
                    state: crate::query::VariantFindingState::ReportRequired,
                    reason_code: crate::query::VariantFindingReason::CurrentSnapshotReportMissing,
                    snapshot_id: scan_id.to_string(),
                    report_id: None,
                    finding_id: None,
                    resolution: None,
                    argv: suggested.argv.clone(),
                },
                suggested,
            )
        };
        found.variant_finding = Some(reference);
        if index < 3 && action_argvs.insert(suggested.argv.clone()) {
            actions.push(suggested);
        }
    }
    Ok((actions, rescan_required))
}

pub(super) fn matching_variant_finding_id(
    store: &StateStore,
    report: &ReportRecord,
    variant_ids: &BTreeSet<String>,
) -> Result<Option<String>> {
    matching_report_finding_id(
        store,
        report,
        crate::query::SAME_NAME_DIVERGENT_FINDING_KIND,
        |affected| affected == variant_ids,
    )
}

pub(super) fn matching_escaping_link_finding_id(
    store: &StateStore,
    report: &ReportRecord,
    untrusted_variant_ids: &BTreeSet<String>,
) -> Result<Option<String>> {
    matching_report_finding_id(
        store,
        report,
        crate::source_policy::ESCAPING_LINK_FINDING_KIND,
        |affected| untrusted_variant_ids.is_subset(affected),
    )
}

pub(super) fn matching_escaping_link_finding_ids(
    store: &StateStore,
    report: &ReportRecord,
    untrusted_variant_ids: &BTreeSet<String>,
) -> Result<Vec<String>> {
    if untrusted_variant_ids.is_empty() {
        return Ok(Vec::new());
    }
    matching_report_finding_ids(
        store,
        report,
        crate::source_policy::ESCAPING_LINK_FINDING_KIND,
        |affected| untrusted_variant_ids.is_subset(affected),
    )
}

pub(super) fn matching_report_finding_id(
    store: &StateStore,
    report: &ReportRecord,
    kind: &str,
    affected_ids_match: impl Fn(&BTreeSet<String>) -> bool,
) -> Result<Option<String>> {
    let matches = matching_report_finding_ids(store, report, kind, affected_ids_match)?;
    Ok(matches.into_iter().next())
}

pub(super) fn matching_report_finding_ids(
    store: &StateStore,
    report: &ReportRecord,
    kind: &str,
    affected_ids_match: impl Fn(&BTreeSet<String>) -> bool,
) -> Result<Vec<String>> {
    let mut matches = BTreeSet::new();
    for finding in report.summary["findings"].as_array().into_iter().flatten() {
        if finding["kind"] != kind {
            continue;
        }
        let Some(affected) = finding_skill_ids(finding) else {
            continue;
        };
        if !affected_ids_match(&affected) {
            continue;
        }
        let Some(raw_id) = finding["id"].as_str() else {
            continue;
        };
        let Ok(id) = FindingId::parse(raw_id.to_owned()) else {
            continue;
        };
        let Some(stored) = store.get_finding(&id)? else {
            continue;
        };
        let Some(stored_skill_ids) = finding_skill_ids(&stored.details) else {
            continue;
        };
        if stored.report_id == report.id
            && stored.category == FindingCategory::Layout
            && stored.details["kind"] == kind
            && affected_ids_match(&stored_skill_ids)
        {
            matches.insert(id.to_string());
        }
    }
    Ok(matches.into_iter().collect())
}

pub(super) fn finding_skill_ids(finding: &Value) -> Option<BTreeSet<String>> {
    finding["affected_skill_ids"]
        .as_array()?
        .iter()
        .map(|id| id.as_str().map(str::to_owned))
        .collect()
}

pub(super) fn normalize_retrieval_hints(hints: &[String]) -> Vec<String> {
    let mut normalized = hints
        .iter()
        .map(|hint| hint.trim())
        .filter(|hint| !hint.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn blocked_retrieval(
    scan_id: &ScanId,
    task: &str,
    hints: &[String],
    matches: &[crate::query::FindMatch],
) -> Value {
    json!({
        "snapshot_id": scan_id,
        "task": task,
        "retrieval_hints": hints,
        "matches": matches.iter().take(10).collect::<Vec<_>>(),
        "returned_match_count": matches.len(),
        "matches_truncated": matches.len() > 10,
        "loaded_skill": null,
    })
}

fn attach_blocked_retrieval(
    mut error: anyhow::Error,
    scan_id: &ScanId,
    task: &str,
    hints: &[String],
    matches: &[crate::query::FindMatch],
) -> anyhow::Error {
    if let Some(blocked) = error.downcast_mut::<SkillLoadBlocked>() {
        blocked.retrieval = Some(blocked_retrieval(scan_id, task, hints, matches));
    }
    error
}
