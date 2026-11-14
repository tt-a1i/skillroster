use super::*;

/// A cheap change detector, never a replacement for verified package hashes.
/// No Skill bytes are read and directory links are followed only inside the
/// same resolved roots approved by Scan. Incomplete walks cannot bless reuse.
pub(crate) fn filesystem_fingerprint(options: &ScanOptions) -> Option<String> {
    let home = normalized_root(&options.home);
    let mut roots = known_agent_roots(&home)
        .into_iter()
        .flat_map(|roots| {
            roots
                .skill_roots
                .into_iter()
                .map(move |path| (path, roots.agent == AgentKind::Hermes))
        })
        .collect::<Vec<_>>();
    roots.extend([
        (home.join(".agents_skills"), false),
        (home.join(".skillroster/library"), false),
    ]);
    roots.extend(
        options
            .managed_source_roots
            .iter()
            .cloned()
            .map(|path| (path, false)),
    );
    roots.extend(
        options
            .explicit_skill_roots
            .iter()
            .map(|root| (normalized_root(&root.path), root.agent == AgentKind::Hermes)),
    );
    roots.extend(
        options
            .explicit_source_roots
            .iter()
            .map(|path| (normalized_root(path), false)),
    );
    roots.extend(
        options
            .durable_read_roots
            .iter()
            .filter(|root| {
                crate::source_policy::capture_identity(&root.path)
                    .ok()
                    .as_ref()
                    == Some(&root.identity)
            })
            .map(|root| (root.path.clone(), false)),
    );
    roots.extend(
        codex_plugin_skill_roots(&home, &mut Vec::new())
            .into_iter()
            .map(|root| (root.path, false)),
    );
    roots.sort();
    roots.dedup();
    let approved = roots
        .iter()
        .filter_map(|(path, _)| fs::canonicalize(path).ok())
        .collect::<Vec<_>>();
    let mut walk = MetadataWalk {
        digest: Sha256::new(),
        approved,
        remaining: 100_000,
    };
    walk.digest.update(b"skillroster-filesystem-metadata-v1");
    walk.record(&home.join(".codex/config.toml"));
    for (root, hermes) in roots {
        walk.visit(&root, options.max_depth, None, hermes, &mut BTreeSet::new())?;
    }
    Some(format!("{:x}", walk.digest.finalize()))
}

// Missing roots need the same lexical normalization as Snapshot configuration:
// canonicalize cannot collapse `absent-parent/../missing` before it exists.
fn normalized_root(path: &Path) -> PathBuf {
    if let Ok(path) = fs::canonicalize(path) {
        return path;
    }
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

struct MetadataWalk {
    digest: Sha256,
    approved: Vec<PathBuf>,
    remaining: usize,
}

impl MetadataWalk {
    fn record(&mut self, path: &Path) -> Option<fs::Metadata> {
        self.digest.update(path.as_os_str().as_encoded_bytes());
        self.digest.update([0]);
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                self.digest.update(metadata.len().to_le_bytes());
                self.digest.update([
                    metadata.is_dir() as u8,
                    metadata.file_type().is_symlink() as u8,
                ]);
                if let Ok(time) = metadata
                    .modified()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).map_err(io::Error::other))
                {
                    self.digest.update(time.as_nanos().to_le_bytes());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    for value in [
                        metadata.dev(),
                        metadata.ino(),
                        metadata.mode() as u64,
                        metadata.ctime() as u64,
                        metadata.ctime_nsec() as u64,
                    ] {
                        self.digest.update(value.to_le_bytes());
                    }
                }
                if metadata.file_type().is_symlink() {
                    if let Ok(target) = fs::read_link(path) {
                        self.digest.update(target.as_os_str().as_encoded_bytes());
                    }
                }
                Some(metadata)
            }
            Err(error) => {
                self.digest.update(format!("{:?}", error.kind()).as_bytes());
                None
            }
        }
    }

    fn visit(
        &mut self,
        path: &Path,
        depth: usize,
        package_depth: Option<usize>,
        hermes: bool,
        ancestors: &mut BTreeSet<PathBuf>,
    ) -> Option<()> {
        self.remaining = self.remaining.checked_sub(1)?;
        let Some(metadata) = self.record(path) else {
            return Some(());
        };
        if !metadata.is_dir() && !metadata.file_type().is_symlink() {
            return Some(());
        }
        let Ok(physical) = fs::canonicalize(path) else {
            return Some(());
        };
        self.digest.update(physical.as_os_str().as_encoded_bytes());
        if !self.approved.iter().any(|root| physical.starts_with(root)) {
            return Some(());
        }
        if metadata.file_type().is_symlink() {
            self.record(&physical);
        }
        if !physical.is_dir() || !ancestors.insert(physical.clone()) {
            return Some(());
        }
        // Revalidate the logical binding before and after enumeration. A retarget
        // cannot become a newly approved source during this walk.
        let identity = crate::source_policy::capture_identity(&physical).ok()?;
        let result = self.visit_directory(path, &physical, depth, package_depth, hermes, ancestors);
        ancestors.remove(&physical);
        if fs::canonicalize(path).ok().as_ref() != Some(&physical)
            || crate::source_policy::capture_identity(&physical)
                .ok()
                .as_ref()
                != Some(&identity)
        {
            return None;
        }
        result
    }

    fn visit_directory(
        &mut self,
        logical: &Path,
        physical: &Path,
        depth: usize,
        package_depth: Option<usize>,
        hermes: bool,
        ancestors: &mut BTreeSet<PathBuf>,
    ) -> Option<()> {
        let entries = match fs::read_dir(physical) {
            Ok(entries) => {
                let entries = entries
                    .take(self.remaining.saturating_add(1))
                    .collect::<io::Result<Vec<_>>>()
                    .ok()?;
                if entries.len() > self.remaining {
                    return None;
                }
                entries
            }
            Err(error) => {
                self.digest.update(format!("{:?}", error.kind()).as_bytes());
                return Some(());
            }
        };
        let declares_skill = entries.iter().any(|entry| entry.file_name() == "SKILL.md");
        let package_depth = package_depth.or(declares_skill.then_some(0));
        let mut entries = entries;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            if package_depth.is_some() {
                if ignored_package_entry_name(&name) {
                    continue;
                }
            } else if ENTRYPOINT_DISCOVERY_EXCLUDED_DIRS
                .iter()
                .any(|excluded| name == *excluded && !(hermes && *excluded == "target"))
            {
                continue;
            }
            let child = logical.join(name);
            // Package support metadata is bounded independently from entrypoint
            // discovery. At the boundary record directory metadata, never content.
            let next_package = package_depth.map(|value| value + 1);
            if next_package.is_some_and(|value| value > MAX_SKILL_PACKAGE_DEPTH)
                || (package_depth.is_none() && depth == 0)
            {
                self.remaining = self.remaining.checked_sub(1)?;
                self.record(&child);
                continue;
            }
            self.visit(
                &child,
                depth.saturating_sub(1),
                next_package,
                hermes,
                ancestors,
            )?;
        }
        Some(())
    }
}
