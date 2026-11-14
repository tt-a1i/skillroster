//! Explicit shared-root adoption. Ordinary Roster authority remains unchanged.
use super::*;
use std::fs;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    core_budget: usize,
    #[serde(default)]
    protected_skill_ids: BTreeSet<String>,
}

struct Package<'a> {
    placement: &'a scan::SkillPlacement,
    source: PathBuf,
    destination: PathBuf,
    fingerprint: String,
}

struct Derived {
    input: Value,
    identity: Value,
    anchors: Vec<PathBuf>,
    impact: Value,
    selection: SelectionEvidenceViews,
}

fn shared_root(scan: &ScanResult) -> Result<&scan::RootObservation> {
    scan.roots
        .iter()
        .find(|root| {
            root.agent == Some(AgentKind::Codex)
                && root.kind == RootKind::Skills
                && root.path.ends_with(".agents/skills")
                && root.status == scan::RootStatus::Included
        })
        .ok_or_else(|| {
            anyhow!("No observed .agents/skills shared root; run check before requesting adoption")
        })
}

/// A proposal is a separate, explicit choice; it never changes ordinary planning.
pub(super) fn guidance(scan: &ScanResult, scan_id: &ScanId) -> Option<Value> {
    let root = shared_root(scan).ok()?;
    let count = scan
        .placements
        .iter()
        .filter(|p| {
            p.root == root.path
                && p.mutation_scope == Some(scan::MutationScope::SharedReadOnly)
                && p.fingerprint_completeness == scan::FingerprintCompleteness::Complete
        })
        .count();
    (count > 0).then(|| {
        json!({
            "actionability": "review_required",
            "action": "adopt_shared_skills",
            "shared_root": root.path,
            "observed_shared_placement_count": count,
            "request_template": {"schema_version": 1, "scan_id": scan_id,
                "adopt_shared_skills": {"core_budget": 15, "protected_skill_ids": []}},
            "requires_explicit_selection": true,
            "confirmation_required_after_plan": true,
            "host_configuration_files_changed": false
        })
    })
}

pub(super) fn prepare(
    store: &StateStore,
    state_dir: &Path,
    mut input: Value,
    prefix: &[String],
) -> Result<Value> {
    let request: Request = serde_json::from_value(
        input
            .as_object_mut()
            .ok_or_else(|| anyhow!("Plan must be an object"))?
            .remove("adopt_shared_skills")
            .expect("explicit adoption request"),
    )?;
    if input
        .as_object()
        .unwrap()
        .keys()
        .any(|key| !matches!(key.as_str(), "schema_version" | "scan_id"))
    {
        bail!(
            "adopt_shared_skills cannot be mixed with other decisions or caller-authored operations"
        );
    }
    if input["schema_version"] != 1 {
        bail!("Plan schema_version must be 1");
    }
    let (scan_id, scan) = latest_scan(store)?;
    require_content_identity(&scan)?;
    if input["scan_id"] != scan_id.as_str() {
        bail!("Adoption must reference the current Snapshot {scan_id}");
    }
    let derived = derive(
        store,
        &scan_id,
        &scan,
        &fs::canonicalize(state_dir)?,
        &request,
    )?;
    prepare_plan(
        store,
        state_dir,
        derived.input,
        PlanOrigin::SharedAdoption,
        Some(json!({"shared_adoption": derived.identity,
            "selection_summary": derived.selection.summary,
            "selection_full": derived.selection.full,
            "selection_uncertainty": derived.selection.uncertainty,
            "impact": derived.impact,
            "anchors": derived.anchors})),
        prefix,
        None,
    )
}

pub(super) fn anchors(identity: Option<&Value>) -> Vec<PathBuf> {
    identity
        .and_then(|v| serde_json::from_value(v["anchors"].clone()).ok())
        .unwrap_or_default()
}

/// Re-derive exactly the trusted decision before both persistence and Apply.
/// Only this path may bypass ordinary shared-read-only overlap checks.
pub(super) fn validate(
    store: &StateStore,
    plan: &PreparedPlan,
    scan: &ScanResult,
    identity: Option<&Value>,
) -> Result<bool> {
    let Some(identity) = identity.filter(|v| v.get("shared_adoption").is_some()) else {
        return Ok(false);
    };
    let request: Request = serde_json::from_value(identity["shared_adoption"]["request"].clone())?;
    let scan_id = ScanId::parse(plan.scan_id.clone())?;
    let derived = derive(store, &scan_id, scan, &plan.state_dir, &request)?;
    if derived.identity != identity["shared_adoption"]
        || derived.anchors != anchors(Some(identity))
        || derived.impact != identity["impact"]
        || derived.selection.summary != identity["selection_summary"]
        || derived.selection.full != identity["selection_full"]
        || json!(derived.selection.uncertainty) != identity["selection_uncertainty"]
    {
        bail!(
            "plan_drifted: shared root entries, aliases or adoption selection changed; rescan and prepare a new Plan"
        );
    }
    let expected = change::prepare(
        &serde_json::to_string(&derived.input)?,
        &PrepareContext {
            approved_roots: adoption_roots(scan, &plan.state_dir, &derived.anchors),
            state_dir: plan.state_dir.clone(),
            operation_policy: OperationPolicy::SharedAdoption,
        },
    )?;
    if expected.digest != plan.digest || expected.approved_roots != plan.approved_roots {
        bail!("plan_drifted: shared adoption operations differ from their exact validated scope");
    }
    validate_library_content_evidence(plan, scan)?;
    Ok(true)
}

pub(super) fn adoption_roots(scan: &ScanResult, state: &Path, anchors: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = approved_roots(scan);
    roots.extend_from_slice(anchors);
    roots.extend([state.join("library"), state.join("plan-backups")]);
    roots
}

fn derive(
    store: &StateStore,
    scan_id: &ScanId,
    scan: &ScanResult,
    state: &Path,
    request: &Request,
) -> Result<Derived> {
    let shared = shared_root(scan)?;
    if !shared.discovery_complete {
        bail!("Shared root discovery is incomplete; rescan before adoption");
    }
    let home = shared
        .path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("Invalid shared root"))?;
    let physical = fs::canonicalize(&shared.path)?;
    let mut packages = BTreeMap::<PathBuf, Package<'_>>::new();
    for placement in scan.placements.iter().filter(|p| p.root == shared.path) {
        let Some(source) = &placement.physical_directory else {
            continue;
        };
        let Ok(relative) = source.strip_prefix(&physical) else {
            continue;
        };
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| c.as_os_str().to_str().is_none_or(|s| s.starts_with('.')))
            || source.components().any(|c| {
                matches!(
                    c.as_os_str().to_str(),
                    Some("plugins" | "cache" | "node_modules")
                )
            })
            || placement.provider.is_some()
            || placement.mutation_scope != Some(scan::MutationScope::SharedReadOnly)
            || placement.fingerprint_completeness != scan::FingerprintCompleteness::Complete
            || placement.entrypoint_read_error.is_some()
            || placement.link_status == scan::LinkStatus::EscapesRoot
        {
            continue;
        }
        // Only real ordinary package directories. An approved root alias is not
        // permission to move a package reached through another directory link.
        let mut cursor = physical.clone();
        let mut ordinary = true;
        for component in relative.components() {
            cursor.push(component);
            ordinary &= fs::symlink_metadata(&cursor)
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
        }
        if !ordinary
            || fs::canonicalize(&placement.directory).ok().as_ref() != Some(source)
            || !fs::symlink_metadata(&placement.entrypoint)
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
            || scan
                .placements
                .iter()
                .any(|p| p.physical_directory.as_ref() == Some(source) && p.provider.is_some())
        {
            continue;
        }
        placement.validated_physical_directory()?;
        let suffix = hex::encode(Sha256::digest(
            scan::unicode_identity_path(relative)?.as_bytes(),
        ));
        let destination =
            state
                .join("library")
                .join(format!("{}-{}", placement.skill_id, &suffix[..12]));
        if fs::symlink_metadata(&destination).is_ok() {
            bail!(
                "Library destination {} already exists; rescan before adoption",
                destination.display()
            );
        }
        packages.insert(
            source.clone(),
            Package {
                placement,
                source: source.clone(),
                destination,
                fingerprint: change::fingerprint(source)?,
            },
        );
    }
    if packages.is_empty() {
        bail!(
            "No complete ordinary shared packages are eligible for adoption; hidden, provider, external and incomplete packages stay in place"
        );
    }
    if scan.placements.iter().any(|p| {
        p.physical_directory.as_ref().is_some_and(|path| {
            packages
                .keys()
                .any(|source| path != source && path.starts_with(source))
        })
    }) {
        bail!(
            "A nested observed consumer would move with a shared package; review its exact path before adoption"
        );
    }
    if packages.keys().any(|path| {
        packages
            .keys()
            .any(|other| path != other && path.starts_with(other))
    }) {
        bail!("Nested Skill packages overlap; preserve them and review before shared adoption");
    }
    let mut aliases = BTreeMap::<PathBuf, AgentKind>::new();
    let mut agents = BTreeSet::from([AgentKind::Codex]);
    let native_roots = harness::known_agent_roots(home)
        .into_iter()
        .flat_map(|roots| roots.skill_roots)
        .collect::<BTreeSet<_>>();
    for root in scan.roots.iter().filter(|r| {
        r.kind == RootKind::Skills
            && r.status == scan::RootStatus::Included
            && r.path != shared.path
            && r.agent.is_some()
            && native_roots.contains(&r.path)
    }) {
        if fs::canonicalize(&root.path).ok().as_ref() == Some(&physical) {
            let agent = root
                .agent
                .ok_or_else(|| anyhow!("Shared root {} has no known Agent", root.path.display()))?;
            if !root.discovery_complete
                || !fs::symlink_metadata(&root.path)?.file_type().is_symlink()
            {
                bail!(
                    "Shared consumer {} is not a complete replaceable root alias",
                    root.path.display()
                );
            }
            aliases.insert(root.path.clone(), agent);
            agents.insert(agent);
        }
    }
    let destinations = harness::known_agent_roots(home)
        .into_iter()
        .flat_map(|roots| {
            roots
                .skill_roots
                .into_iter()
                .map(move |path| (roots.agent, path))
        })
        .filter(|(agent, path)| agents.contains(agent) && *path != shared.path)
        .fold(BTreeMap::new(), |mut map, (agent, path)| {
            map.entry(agent).or_insert(path);
            map
        });
    if agents.iter().any(|agent| !destinations.contains_key(agent)) {
        bail!("A shared consumer has no supported independent Skill root");
    }
    let mut projected = scan.clone();
    for placement in &mut projected.placements {
        if placement
            .physical_directory
            .as_ref()
            .is_some_and(|path| packages.contains_key(path))
            && (placement.root == shared.path
                || aliases
                    .keys()
                    .any(|alias| placement.directory.starts_with(alias)))
            && agents.contains(&placement.agent.unwrap_or(AgentKind::Codex))
        {
            placement.governable = true;
            placement.mutation_scope = Some(scan::MutationScope::Mutable);
        } else if placement.agent.is_some_and(|agent| agents.contains(&agent)) {
            // Independent exposures are outside this action. They remain Core
            // and consume budget even when another workflow could edit them.
            placement.governable = false;
            placement.mutation_scope = Some(scan::MutationScope::DurableReadOnly);
        }
    }
    let recommendation = crate::roster_recommendation::recommend_independent_rosters(
        &projected,
        &declared_core_pairs(store, scan)?,
        &crate::roster_recommendation::RecommendationRequest {
            core_budget: request.core_budget,
            protected_skill_ids: request.protected_skill_ids.clone(),
        },
        &agents,
    )?;
    let selection = roster_selection_evidence(&recommendation);
    let core = recommendation
        .changes
        .iter()
        .filter(|c| c.state == "core")
        .map(|c| {
            (
                harness_agent(&c.agent).expect("validated Agent"),
                c.skill_id.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut operations = Vec::new();
    let mut directories = BTreeSet::new();
    let mut anchors = BTreeSet::new();
    for root in [state.join("library"), state.join("plan-backups")] {
        add_directory(&mut operations, &mut directories, &root, false)?;
    }
    for package in packages.values() {
        operations.push(json!({"kind": "move_recoverable", "source": package.source,
            "target": package.destination, "expected_fingerprint": package.fingerprint}));
    }
    for alias in aliases.keys() {
        let parent = alias.parent().expect("absolute root parent");
        anchors.insert(parent.to_path_buf());
        let digest = hex::encode(Sha256::digest(
            scan::unicode_identity_path(alias)?.as_bytes(),
        ));
        operations.push(json!({"kind": "move_recoverable", "source": alias,
            "target": state.join("plan-backups").join(format!("shared-{}-{}", scan_id, &digest[..12])),
            "expected_fingerprint": change::fingerprint(alias)?}));
        add_directory(&mut operations, &mut directories, alias, true)?;
    }
    let mut after = scan.clone();
    after.placements.retain(|p| {
        !(p.physical_directory
            .as_ref()
            .is_some_and(|path| packages.contains_key(path))
            && (p.root == shared.path
                || aliases.keys().any(|alias| p.directory.starts_with(alias))))
    });
    let mut links = BTreeMap::<PathBuf, (PathBuf, String)>::new();
    for (alias, agent) in &aliases {
        // Retain excluded Skill entry links (including .system and untrusted
        // external entries) without opening their targets or moving originals.
        for p in scan.placements.iter().filter(|p| {
            p.agent == Some(*agent)
                && p.directory.starts_with(alias)
                && !p
                    .physical_directory
                    .as_ref()
                    .is_some_and(|path| packages.contains_key(path))
        }) {
            let relative = p.directory.strip_prefix(alias)?;
            let mut source = physical.clone();
            let mut target = alias.clone();
            for component in relative.components() {
                source.push(component);
                target.push(component);
                // A directory link can contain unobserved external children.
                // Retain its first entry; never inspect through that boundary.
                if fs::symlink_metadata(&source)?.file_type().is_symlink() {
                    break;
                }
            }
            if links.keys().any(|target| p.directory.starts_with(target)) {
                continue;
            }
            links.insert(target, (source.clone(), change::fingerprint(&source)?));
        }
    }
    for (agent, root) in &destinations {
        let root = root.clone();
        if !aliases.contains_key(&root)
            && fs::symlink_metadata(&root).is_ok_and(|m| m.file_type().is_symlink())
        {
            bail!(
                "Independent root {} points elsewhere; shared adoption will not replace it",
                root.display()
            );
        }
        let parent = nearest_existing_parent(&root)?;
        anchors.insert(parent);
        add_directory(
            &mut operations,
            &mut directories,
            &root,
            aliases.contains_key(&root),
        )?;
        for package in packages.values().filter(|package| {
            core.contains(&(*agent, package.placement.skill_id.as_str()))
                && scan.placements.iter().any(|p| {
                    p.agent == Some(*agent)
                        && p.default_exposed
                        && p.physical_directory.as_ref() == Some(&package.source)
                })
        }) {
            // An existing independent exposure already satisfies this Core.
            if after.placements.iter().any(|p| {
                p.agent == Some(*agent)
                    && p.skill_id == package.placement.skill_id
                    && p.content_digest == package.placement.content_digest
                    && p.default_exposed
            }) {
                continue;
            }
            let relative = package.source.strip_prefix(&physical)?;
            let target = root.join(relative);
            if !aliases.contains_key(&root) && fs::symlink_metadata(&target).is_ok() {
                bail!(
                    "Core destination {} already exists; no local item will be overwritten",
                    target.display()
                );
            }
            validate_core_visibility(scan, *agent, &root, &target, package)?;
            links.insert(
                target.clone(),
                (package.destination.clone(), package.fingerprint.clone()),
            );
            let mut p = package.placement.clone();
            p.agent = Some(*agent);
            p.directory = target.clone();
            p.root = root.clone();
            p.entrypoint = target.join("SKILL.md");
            p.default_exposed = true;
            after.placements.push(p);
        }
    }
    // Other observed consumers keep their exposure through exact link retargets.
    for p in scan.placements.iter().filter(|p| {
        p.agent.is_some()
            && p.root != shared.path
            && !aliases.keys().any(|alias| p.directory.starts_with(alias))
    }) {
        let Some(package) = p
            .physical_directory
            .as_ref()
            .and_then(|path| packages.get(path))
        else {
            continue;
        };
        let target = scan::physical_entry_path(&p.directory);
        if !fs::symlink_metadata(&target)?.file_type().is_symlink() {
            bail!(
                "Dependent consumer {} is not an independently retargetable link",
                target.display()
            );
        }
        if matches!(p.agent, Some(AgentKind::Codex | AgentKind::Hermes)) {
            validate_library_native_visibility(
                scan,
                &p.skill_id,
                &package.destination,
                &package.source,
            )?;
        }
        let digest = hex::encode(Sha256::digest(
            scan::unicode_identity_path(&target)?.as_bytes(),
        ));
        operations.push(json!({"kind": "move_recoverable", "source": target,
            "target": state.join("plan-backups").join(format!("consumer-{}-{}", scan_id, &digest[..12])),
            "expected_fingerprint": change::fingerprint(&target)?}));
        links.insert(
            target,
            (package.destination.clone(), package.fingerprint.clone()),
        );
    }
    for (target, (source, fingerprint)) in &links {
        if !cfg!(unix) && fingerprint.starts_with("symlink:") {
            bail!(
                "Shared adoption preserving external link entries is currently supported on Unix; no Ready Plan or Agent changes were created"
            );
        }
        add_directory(
            &mut operations,
            &mut directories,
            target.parent().expect("link parent"),
            aliases.keys().any(|alias| target.starts_with(alias)),
        )?;
        operations.push(
            json!({"kind": "create_symlink", "target": target, "source": source,
            "expected_fingerprint": "missing", "expected_source_fingerprint": fingerprint}),
        );
    }
    let roster_changes = recommendation
        .changes
        .iter()
        .filter(|c| {
            packages
                .values()
                .any(|p| p.placement.skill_id == c.skill_id)
        })
        .collect::<Vec<_>>();
    let mut library_changes = BTreeMap::new();
    for package in packages.values() {
        let action = library_changes.entry(&package.placement.skill_id).or_insert_with(|| json!({
            "skill_id": package.placement.skill_id, "canonical_placement_id": package.placement.id,
            "placement_ids": [], "requested_state": "hosted",
            "canonical_path": package.source, "library_path": package.destination
        }));
        action["placement_ids"]
            .as_array_mut()
            .expect("placement IDs")
            .push(json!(package.placement.id));
    }
    let governed = roster_changes
        .iter()
        .map(|c| c.skill_id.as_str())
        .chain(packages.values().map(|p| p.placement.skill_id.as_str()))
        .collect::<BTreeSet<_>>();
    let evidence = scan
        .skills
        .iter()
        .filter(|s| governed.contains(s.id.as_str()))
        .map(|s| {
            evidence_id(scan_id, &format!("digest:{}", s.content_digest)).map(|id| id.to_string())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let mut listings = BTreeMap::new();
    for package in packages.values() {
        let mut parent = package.source.parent();
        while let Some(path) = parent.filter(|path| path.starts_with(&physical)) {
            if !listings.contains_key(path) {
                listings.insert(path.to_path_buf(), directory_listing(path)?);
            }
            if path == physical {
                break;
            }
            parent = path.parent();
        }
    }
    let alias_bindings = aliases
        .keys()
        .map(|path| Ok(json!({"path": path, "fingerprint": change::fingerprint(path)?})))
        .collect::<Result<Vec<_>>>()?;
    let affected_agents = scan
        .placements
        .iter()
        .filter(|p| {
            p.physical_directory
                .as_ref()
                .is_some_and(|path| packages.contains_key(path))
        })
        .filter_map(|p| p.agent)
        .chain(agents.iter().copied())
        .map(|agent| agent.id())
        .collect::<BTreeSet<_>>();
    let exclusions = scan
        .placements
        .iter()
        .filter(|p| {
            p.root == shared.path
                && !p
                    .physical_directory
                    .as_ref()
                    .is_some_and(|path| packages.contains_key(path))
        })
        .map(|p| {
            json!({"skill_id": p.skill_id, "path": p.directory,
            "reason": "preserved_outside_adoption_scope", "mutation_scope": p.mutation_scope,
            "fingerprint_completeness": p.fingerprint_completeness})
        })
        .collect::<Vec<_>>();
    let impact = json!({"kind": "shared_adoption", "shared_root": shared.path, "hosted_package_count": packages.len(),
        "separated_agent_roots": aliases.keys().collect::<Vec<_>>(), "affected_agents": affected_agents,
        "exclusions": exclusions,
        "preserved_entries": "Hidden, provider, external and incomplete packages remain at their original locations; excluded Skill links remain exposed.",
        "consumer_boundary": "All observed consumers are included. Unobserved consumers of the shared root may lose package paths; review the exact moved paths before Apply.",
        "host_configuration_files_changed": false,
        "before_default_exposure": scan.placements.iter().filter(|p| p.default_exposed).count(),
        "after_default_exposure": after.placements.iter().filter(|p| p.default_exposed).count(),
        "exposed_metadata_footprint": {"before": crate::footprint::exposed(scan), "after": crate::footprint::exposed(&after)}});
    Ok(Derived {
        input: json!({"schema_version": 1, "scan_id": scan_id, "evidence_ids": evidence,
        "operations": operations, "roster_changes": roster_changes, "library_changes": library_changes.into_values().collect::<Vec<_>>() }),
        identity: json!({"request": request, "shared_root": shared.path, "physical_root": physical,
            "shared_root_fingerprint": if fs::symlink_metadata(&shared.path)?.file_type().is_symlink() { Some(change::fingerprint(&shared.path)?) } else { None },
            "directory_entries": listings, "alias_bindings": alias_bindings}),
        anchors: anchors.into_iter().collect(),
        impact,
        selection,
    })
}

fn nearest_existing_parent(path: &Path) -> Result<PathBuf> {
    let mut parent = path.parent().ok_or_else(|| anyhow!("Invalid root"))?;
    while !parent.exists() {
        parent = parent.parent().ok_or_else(|| anyhow!("No root anchor"))?;
    }
    Ok(fs::canonicalize(parent)?)
}

fn add_directory(
    operations: &mut Vec<Value>,
    added: &mut BTreeSet<PathBuf>,
    path: &Path,
    projected: bool,
) -> Result<()> {
    if added.contains(path) {
        return Ok(());
    }
    if !projected && fs::symlink_metadata(path).is_ok() {
        if !fs::symlink_metadata(path)?.is_dir() {
            bail!("Directory {} is not an ordinary directory", path.display());
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        add_directory(
            operations,
            added,
            parent,
            projected
                && !fs::symlink_metadata(parent)
                    .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()),
        )?;
    }
    added.insert(path.to_path_buf());
    operations.push(
        json!({"kind": "create_directory", "target": path, "expected_fingerprint": "missing"}),
    );
    Ok(())
}

fn directory_listing(path: &Path) -> Result<Vec<Value>> {
    let mut entries = fs::read_dir(path)?
        .take(4097)
        .collect::<std::io::Result<Vec<_>>>()?;
    if entries.len() > 4096 {
        bail!(
            "Shared root entry listing exceeds 4096 entries; narrow the shared library before adoption"
        );
    }
    entries.sort_by_key(|entry| entry.file_name());
    entries.into_iter().map(|entry| {
        let kind = entry.file_type()?;
        Ok(json!({"name": entry.file_name().to_str().ok_or_else(|| anyhow!("Non-Unicode shared entry"))?,
            "kind": if kind.is_symlink() { "symlink" } else if kind.is_dir() { "directory" } else { "file" },
            "link": if kind.is_symlink() { Some(fs::read_link(entry.path())?) } else { None }}))
    }).collect()
}

fn validate_core_visibility(
    scan: &ScanResult,
    agent: AgentKind,
    root: &Path,
    target: &Path,
    package: &Package<'_>,
) -> Result<()> {
    use crate::claude_visibility::CatalogVisibility;
    let source = package.placement;
    let visibility = match agent {
        AgentKind::Codex => scan.codex_visibility.as_ref().map(|v| {
            let projected = v.visibility_for_retargeted_entrypoint(
                &source.entrypoint,
                &package.destination.join("SKILL.md"),
            );
            if projected != CatalogVisibility::On {
                projected
            } else {
                v.visibility_for_core_target_entrypoint(
                    &target.join("SKILL.md"),
                    &source.entrypoint,
                )
            }
        }),
        AgentKind::Hermes => scan.hermes_visibility.as_ref().map(|v| {
            v.visibility_for_core_target_entrypoint(&target.join("SKILL.md"), &source.entrypoint)
        }),
        AgentKind::ClaudeCode => scan.native_visibility.as_ref().map(|v| {
            let invocation = scan
                .skills
                .iter()
                .find(|skill| skill.id == source.skill_id)
                .map(|skill| skill.metadata.model_invocation)
                .unwrap_or(crate::claude_visibility::ModelInvocation::Unknown);
            v.skill_visibility(
                root,
                target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(""),
                invocation,
            )
        }),
        _ => Some(CatalogVisibility::On),
    };
    if visibility != Some(CatalogVisibility::On) {
        bail!(
            "Core target {} has disabled or unknown native visibility; keep it unchanged and review host settings",
            target.display()
        );
    }
    Ok(())
}
