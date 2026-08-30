//! Deep local policy for durable, exact source-root read permissions.
//!
//! This module owns `source-root confirm`, `source-root inspect`,
//! `source-root revoke`, and scan-time freezing. SQLite persistence and
//! platform-specific filesystem identity stay internal and locally
//! substitutable; the rest of the crate sees only typed local-policy facts.
//!
//! A permission binds one exact observed canonical source directory to one
//! current completed Snapshot/Finding. It restores factual scanning only: it
//! never adds Agent exposure, marks a source safe, authorizes governance,
//! or enters Plan/Apply/Receipt semantics.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::FindingRecord;
use crate::scan::{LinkStatus, ScanResult};
use crate::sqlite::{StateStore, StorageResult};

/// Stable Finding title a confirmation may be bound to.
pub const ESCAPING_LINK_FINDING_TITLE: &str = "Skill links escape an approved root";
/// Stable Finding kind used by Agent-facing references and validation.
pub const ESCAPING_LINK_FINDING_KIND: &str = "escaping_link_source_confirmation";

static NEXT_PERMISSION_ID: AtomicU64 = AtomicU64::new(0);

fn opaque_permission_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_PERMISSION_ID.fetch_add(1, Ordering::Relaxed) as u128;
    let process = std::process::id() as u128;
    format!("sroot_{:032x}", nanos ^ (process << 64) ^ sequence)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourcePermissionId(String);

impl SourcePermissionId {
    pub fn new() -> Self {
        Self(opaque_permission_id())
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, crate::model::InvalidId> {
        let value = value.into();
        if value.starts_with("sroot_") && value.len() > "sroot_".len() {
            Ok(Self(value))
        } else {
            Err(crate::model::InvalidId::new("sroot", value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SourcePermissionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SourcePermissionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Stable filesystem identity captured when a permission is granted.
///
/// POSIX uses device + inode plus the inode change time as a conservative reuse
/// guard. Windows pairs volume + file index with the object's creation time for
/// the same purpose. These fields reduce common object-reuse false negatives;
/// their precision and guarantees still depend on the filesystem. A platform
/// without these facts reports `Unavailable`, and drift detection then degrades
/// to exact resolved-path equality.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RootIdentity {
    Posix {
        dev: u64,
        ino: u64,
        ctime: i64,
        ctime_nsec: i64,
    },
    Windows {
        volume_serial: u32,
        file_index_high: u32,
        file_index_low: u32,
        creation_time_high: u32,
        creation_time_low: u32,
    },
    Unavailable,
}

#[cfg(unix)]
pub(crate) fn capture_identity(path: &Path) -> io::Result<RootIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path)?;
    Ok(RootIdentity::Posix {
        dev: metadata.dev(),
        ino: metadata.ino(),
        ctime: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec(),
    })
}

#[cfg(windows)]
pub(crate) fn capture_identity(path: &Path) -> io::Result<RootIdentity> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
        OPEN_EXISTING,
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut information) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(RootIdentity::Windows {
        volume_serial: information.dwVolumeSerialNumber,
        file_index_high: information.nFileIndexHigh,
        file_index_low: information.nFileIndexLow,
        creation_time_high: information.ftCreationTime.dwHighDateTime,
        creation_time_low: information.ftCreationTime.dwLowDateTime,
    })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn capture_identity(_path: &Path) -> io::Result<RootIdentity> {
    Ok(RootIdentity::Unavailable)
}

pub(crate) fn identity_matches_exact(path: &Path, identity: &RootIdentity) -> bool {
    fs::canonicalize(path).ok().as_deref() == Some(path)
        && capture_identity(path).is_ok_and(|current| &current == identity)
}

/// One durable local read permission for one exact canonical source directory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRootPermission {
    pub id: SourcePermissionId,
    /// Exact canonical source directory the user confirmed, as observed.
    pub path: PathBuf,
    /// Escaping-link Finding the confirmation was bound to.
    pub finding_id: String,
    /// Completed Snapshot that Finding belonged to.
    pub snapshot_id: String,
    pub identity: RootIdentity,
    pub granted_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<i64>,
}

/// Typed freeze state of one granted root at scan time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRootState {
    /// Resolved to the granted canonical path with the granted identity.
    Active,
    /// The granted path no longer exists.
    Missing,
    /// The granted path exists but could not be resolved or identified.
    Inaccessible,
    /// The granted canonical path resolves to the same path but the
    /// filesystem object at it has a different identity.
    Replaced,
    /// The granted canonical path now resolves to a different path.
    Retargeted,
}

/// Persisted typed drift/policy fact carried by one Snapshot payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRootPolicyFact {
    pub permission_id: String,
    pub granted_path: PathBuf,
    pub granted_at: i64,
    pub state: SourceRootState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift_reason: Option<String>,
}

/// One frozen (resolved) view of a persisted active permission.
#[derive(Clone, Debug)]
pub struct FrozenSourceRoot {
    pub permission: SourceRootPermission,
    pub state: SourceRootState,
    pub resolved_path: Option<PathBuf>,
    pub drift_reason: Option<String>,
}

/// Typed `source-root` policy errors with stable codes for JSON envelopes.
#[derive(Debug)]
pub enum SourceRootPolicyError {
    FindingNotFound {
        finding_id: String,
    },
    NotEscapingLinkFinding {
        finding_id: String,
    },
    FindingSnapshotNotCurrent {
        finding_id: String,
        finding_snapshot: String,
        current_snapshot: String,
    },
    PathNotResolvable {
        path: PathBuf,
        reason: String,
    },
    PathNotObservedTarget {
        path: PathBuf,
    },
    PathNotDirectory {
        path: PathBuf,
    },
    PermissionNotFound {
        permission_id: String,
    },
    PermissionAlreadyRevoked {
        permission_id: String,
    },
    ActivePermissionIdentityDrift {
        permission_id: String,
        path: PathBuf,
    },
}

impl SourceRootPolicyError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::FindingNotFound { .. } => "source_root_finding_not_found",
            Self::FindingSnapshotNotCurrent { .. } => "source_root_finding_stale",
            Self::NotEscapingLinkFinding { .. } => "source_root_finding_not_escaping",
            Self::PathNotResolvable { .. } => "source_root_path_not_resolvable",
            Self::PathNotObservedTarget { .. } => "source_root_path_not_observed",
            Self::PathNotDirectory { .. } => "source_root_path_not_directory",
            Self::PermissionNotFound { .. } => "source_root_permission_not_found",
            Self::PermissionAlreadyRevoked { .. } => "source_root_permission_already_revoked",
            Self::ActivePermissionIdentityDrift { .. } => "source_root_permission_identity_drift",
        }
    }
}

impl fmt::Display for SourceRootPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FindingNotFound { finding_id } => {
                write!(formatter, "Finding {finding_id} does not exist")
            }
            Self::FindingSnapshotNotCurrent {
                finding_id,
                finding_snapshot,
                current_snapshot,
            } => write!(
                formatter,
                "Finding {finding_id} belongs to Snapshot {finding_snapshot}, but the current completed Snapshot is {current_snapshot}; rescan and confirm against the current Finding"
            ),
            Self::NotEscapingLinkFinding { finding_id } => write!(
                formatter,
                "Finding {finding_id} is not a '{ESCAPING_LINK_FINDING_TITLE}' Finding and cannot confirm a source root"
            ),
            Self::PathNotResolvable { path, reason } => write!(
                formatter,
                "cannot resolve source root {}: {reason}",
                path.display()
            ),
            Self::PathNotObservedTarget { path } => write!(
                formatter,
                "{} was not observed as an exact escaping link target in the bound Snapshot; confirm only an exact observed target",
                path.display()
            ),
            Self::PathNotDirectory { path } => write!(
                formatter,
                "confirmed source root {} is not a directory",
                path.display()
            ),
            Self::PermissionNotFound { permission_id } => {
                write!(
                    formatter,
                    "source-root permission {permission_id} does not exist"
                )
            }
            Self::PermissionAlreadyRevoked { permission_id } => write!(
                formatter,
                "source-root permission {permission_id} is already revoked"
            ),
            Self::ActivePermissionIdentityDrift {
                permission_id,
                path,
            } => write!(
                formatter,
                "source-root permission {permission_id} no longer identifies {}; revoke it before confirming the current observed directory",
                path.display()
            ),
        }
    }
}

impl std::error::Error for SourceRootPolicyError {}

/// Outcome of one confirmation request.
pub struct ConfirmOutcome {
    pub permission: SourceRootPermission,
    /// True when an identical active durable permission already existed; the
    /// request then changed nothing.
    pub already_permitted: bool,
}

/// Confirmation binds to the current completed Snapshot, the titled
/// escaping-link Finding, and one exact canonical target observed by that
/// Snapshot. No parent, sibling, descendant, alias, wildcard, or
/// Agent-specific exception is inferred.
pub fn confirm_source_root(
    store: &StateStore,
    finding: &FindingRecord,
    snapshot: &ScanResult,
    requested: &Path,
) -> anyhow::Result<ConfirmOutcome> {
    if !crate::query::stored_finding_is(
        finding,
        crate::query::FindingKind::EscapingLinkSourceConfirmation,
    ) {
        return Err(SourceRootPolicyError::NotEscapingLinkFinding {
            finding_id: finding.id.as_str().to_owned(),
        }
        .into());
    }
    let report = store.get_report(&finding.report_id)?.ok_or_else(|| {
        SourceRootPolicyError::FindingNotFound {
            finding_id: finding.id.as_str().to_owned(),
        }
    })?;
    let current =
        store
            .latest_completed_scan()?
            .ok_or_else(|| SourceRootPolicyError::FindingNotFound {
                finding_id: finding.id.as_str().to_owned(),
            })?;
    if report.scan_id != current.id {
        return Err(SourceRootPolicyError::FindingSnapshotNotCurrent {
            finding_id: finding.id.as_str().to_owned(),
            finding_snapshot: report.scan_id.as_str().to_owned(),
            current_snapshot: current.id.as_str().to_owned(),
        }
        .into());
    }
    let requested_exact = lexical_normalize(requested);
    let observed = observed_exact_targets(snapshot);
    if !observed.contains(&requested_exact) {
        return Err(SourceRootPolicyError::PathNotObservedTarget {
            path: requested_exact,
        }
        .into());
    }
    let requested = fs::canonicalize(&requested_exact).map_err(|error| {
        SourceRootPolicyError::PathNotResolvable {
            path: requested_exact,
            reason: error.to_string(),
        }
    })?;
    let metadata =
        fs::metadata(&requested).map_err(|error| SourceRootPolicyError::PathNotResolvable {
            path: requested.clone(),
            reason: error.to_string(),
        })?;
    if !metadata.is_dir() {
        return Err(SourceRootPolicyError::PathNotDirectory { path: requested }.into());
    }
    let identity =
        capture_identity(&requested).map_err(|error| SourceRootPolicyError::PathNotResolvable {
            path: requested.clone(),
            reason: error.to_string(),
        })?;
    let existing = store
        .list_source_root_permissions()?
        .into_iter()
        .find(|permission| permission.revoked_at.is_none() && permission.path == requested);
    if existing
        .as_ref()
        .is_some_and(|permission| permission.identity == identity)
    {
        return Ok(ConfirmOutcome {
            permission: existing.expect("checked above"),
            already_permitted: true,
        });
    }
    if let Some(existing) = existing {
        return Err(SourceRootPolicyError::ActivePermissionIdentityDrift {
            permission_id: existing.id.to_string(),
            path: requested,
        }
        .into());
    }
    let permission = SourceRootPermission {
        id: SourcePermissionId::new(),
        path: requested,
        finding_id: finding.id.as_str().to_owned(),
        snapshot_id: report.scan_id.as_str().to_owned(),
        identity,
        granted_at: Utc::now().timestamp(),
        revoked_at: None,
    };
    store.save_source_root_permission(&permission)?;
    Ok(ConfirmOutcome {
        permission,
        already_permitted: false,
    })
}

/// Every persisted permission (active and revoked), newest first. Auditable
/// approval/revocation record; the CLI never deletes rows for a revoke.
pub fn inspect_permissions(store: &StateStore) -> StorageResult<Vec<SourceRootPermission>> {
    let mut permissions = store.list_source_root_permissions()?;
    permissions.sort_by(|left, right| {
        right
            .granted_at
            .cmp(&left.granted_at)
            .then_with(|| left.id.as_str().cmp(right.id.as_str()))
    });
    Ok(permissions)
}

/// Revoke one durable permission by exact ID. A revoked permission stays in
/// the auditable record with its revocation time.
pub fn revoke_permission(
    store: &StateStore,
    permission_id: &str,
) -> anyhow::Result<SourceRootPermission> {
    let id = SourcePermissionId::parse(permission_id.to_owned())?;
    let Some(mut permission) = store.get_source_root_permission(&id)? else {
        return Err(SourceRootPolicyError::PermissionNotFound {
            permission_id: permission_id.to_owned(),
        }
        .into());
    };
    if permission.revoked_at.is_some() {
        return Err(SourceRootPolicyError::PermissionAlreadyRevoked {
            permission_id: permission_id.to_owned(),
        }
        .into());
    }
    permission.revoked_at = Some(Utc::now().timestamp());
    store.save_source_root_permission(&permission)?;
    Ok(permission)
}

/// Resolve and freeze every active permitted root exactly once, before any
/// discovery. A drifted root fails closed on its own and never makes an
/// unrelated valid root unreadable.
pub fn freeze_active_roots(store: &StateStore) -> StorageResult<Vec<FrozenSourceRoot>> {
    let mut frozen = store
        .list_source_root_permissions()?
        .into_iter()
        .filter(|permission| permission.revoked_at.is_none())
        .map(|permission| freeze_permission(&permission))
        .collect::<Vec<_>>();
    frozen.sort_by(|left, right| left.permission.path.cmp(&right.permission.path));
    Ok(frozen)
}

/// Freeze one permission against the live filesystem.
///
/// Identity-blind roots (identity `Unavailable`) degrade to exact
/// resolved-path equality: missing, inaccessible, and retargeted drift are
/// still detected; silent replacement at the same canonical path is not.
pub fn freeze_permission(permission: &SourceRootPermission) -> FrozenSourceRoot {
    let granted = &permission.path;
    let resolved = match fs::canonicalize(granted) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return FrozenSourceRoot {
                permission: permission.clone(),
                state: SourceRootState::Missing,
                resolved_path: None,
                drift_reason: Some(format!("granted path no longer exists: {}", error)),
            };
        }
        Err(error) => {
            return FrozenSourceRoot {
                permission: permission.clone(),
                state: SourceRootState::Inaccessible,
                resolved_path: None,
                drift_reason: Some(format!("granted path cannot be resolved: {error}")),
            };
        }
    };
    let identity_matches = match capture_identity(&resolved) {
        Ok(current) => current == permission.identity,
        Err(error) => {
            return FrozenSourceRoot {
                permission: permission.clone(),
                state: SourceRootState::Inaccessible,
                resolved_path: Some(resolved),
                drift_reason: Some(format!("filesystem identity cannot be read: {error}")),
            };
        }
    };
    let (state, drift_reason) = classify_resolution(&resolved, granted, identity_matches);
    FrozenSourceRoot {
        permission: permission.clone(),
        state,
        resolved_path: Some(resolved),
        drift_reason,
    }
}

/// Pure typed drift classification. `resolved` is the current canonical
/// resolution of `granted`; `identity_matches` is the comparison against the
/// granted identity (declared impossible when the platform cannot compare).
fn classify_resolution(
    resolved: &Path,
    granted: &Path,
    identity_matches: bool,
) -> (SourceRootState, Option<String>) {
    if resolved != granted {
        (
            SourceRootState::Retargeted,
            Some("granted canonical path now resolves to a different directory".into()),
        )
    } else if !identity_matches {
        (
            SourceRootState::Replaced,
            Some("the filesystem object at the granted path has a different identity".into()),
        )
    } else {
        (SourceRootState::Active, None)
    }
}

/// One persisted, frozen policy fact for the scan payload and terminal views.
pub fn fact_from_frozen(frozen: &FrozenSourceRoot) -> SourceRootPolicyFact {
    SourceRootPolicyFact {
        permission_id: frozen.permission.id.as_str().to_owned(),
        granted_path: frozen.permission.path.clone(),
        granted_at: frozen.permission.granted_at,
        state: frozen.state,
        resolved_path: frozen.resolved_path.clone(),
        drift_reason: frozen.drift_reason.clone(),
    }
}

/// Exact lexical link targets observed by one Snapshot's escaping Finding.
/// Confirmation compares this spelling before resolving the filesystem so an
/// unobserved alias cannot borrow another target's confirmation.
fn observed_exact_targets(snapshot: &ScanResult) -> BTreeSet<PathBuf> {
    snapshot
        .placements
        .iter()
        .filter(|placement| placement.link_status == LinkStatus::EscapesRoot)
        .filter_map(|placement| placement.link_target.as_ref())
        .map(|target| lexical_normalize(target))
        .collect()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
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

pub fn identity_json(identity: &RootIdentity) -> Value {
    match identity {
        RootIdentity::Posix {
            dev,
            ino,
            ctime,
            ctime_nsec,
        } => json!({
            "kind": "posix",
            "dev": dev,
            "ino": ino,
            "ctime": ctime,
            "ctime_nsec": ctime_nsec
        }),
        RootIdentity::Windows {
            volume_serial,
            file_index_high,
            file_index_low,
            creation_time_high,
            creation_time_low,
        } => json!({
            "kind": "windows",
            "volume_serial": volume_serial,
            "file_index_high": file_index_high,
            "file_index_low": file_index_low,
            "creation_time_high": creation_time_high,
            "creation_time_low": creation_time_low
        }),
        RootIdentity::Unavailable => json!({ "kind": "unavailable" }),
    }
}

/// One permission as a bounded JSON fact. `current_state` is the live freeze
/// state when one was computed (inspect), otherwise the stored lifecycle
/// state.
pub fn permission_json(
    permission: &SourceRootPermission,
    current_state: Option<SourceRootState>,
) -> Value {
    let state = if permission.revoked_at.is_some() {
        "revoked".to_owned()
    } else {
        current_state
            .and_then(|state| {
                serde_json::to_value(state)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
            })
            .unwrap_or_else(|| "active".to_owned())
    };
    json!({
        "permission_id": permission.id,
        "path": permission.path,
        "finding_id": permission.finding_id,
        "snapshot_id": permission.snapshot_id,
        "granted_at": permission.granted_at,
        "revoked_at": permission.revoked_at,
        "state": state,
        "identity": identity_json(&permission.identity),
    })
}

/// Bounded local-policy view for `source-root inspect`, `status`, and
/// lifecycle inspection. `with_current_state` additionally resolves active
/// permissions against the live filesystem.
pub fn policy_value(
    store: &StateStore,
    with_current_state: bool,
    limit: usize,
    offset: usize,
) -> StorageResult<Value> {
    let frozen = if with_current_state {
        Some(freeze_active_roots(store)?)
    } else {
        None
    };
    policy_value_with_page(store, frozen.as_deref(), Some((limit, offset)))
}

/// Build the same bounded policy view from roots already frozen by readiness
/// validation. Status uses this to avoid a second filesystem walk while it
/// holds the shared state lock.
pub fn policy_value_from_frozen(
    store: &StateStore,
    frozen: &[FrozenSourceRoot],
    limit: usize,
    offset: usize,
) -> StorageResult<Value> {
    policy_value_with_page(store, Some(frozen), Some((limit, offset)))
}

/// Complete local export view. This is written to the user-selected lifecycle
/// export file rather than returned inline in the Agent JSON envelope.
pub fn policy_export_value(store: &StateStore) -> StorageResult<Value> {
    policy_value_with_page(store, None, None)
}

fn policy_value_with_page(
    store: &StateStore,
    frozen: Option<&[FrozenSourceRoot]>,
    page: Option<(usize, usize)>,
) -> StorageResult<Value> {
    let permissions = inspect_permissions(store)?;
    let frozen_by_id = frozen
        .map(|frozen| {
            frozen
                .iter()
                .map(|fact| (fact.permission.id.as_str(), fact))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let items = permissions
        .iter()
        .skip(page.map_or(0, |(_, offset)| offset))
        .take(page.map_or(usize::MAX, |(limit, _)| limit))
        .map(|permission| {
            let current_state = frozen_by_id
                .get(permission.id.as_str())
                .map(|frozen| frozen.state);
            let mut value = permission_json(permission, current_state);
            if let Some(frozen) = frozen_by_id.get(permission.id.as_str()) {
                value["resolved_path"] = json!(frozen.resolved_path);
                value["drift_reason"] = json!(frozen.drift_reason);
            }
            value
        })
        .collect::<Vec<_>>();
    let revoked_count = permissions
        .iter()
        .filter(|permission| permission.revoked_at.is_some())
        .count();
    let drifted_count = frozen_by_id
        .values()
        .filter(|frozen| frozen.state != SourceRootState::Active)
        .count();
    let active_count = permissions.len() - revoked_count - drifted_count;
    let offset = page.map_or(0, |(_, offset)| offset);
    let next_offset = (offset + items.len() < permissions.len()).then_some(offset + items.len());
    Ok(json!({
        "permission_count": permissions.len(),
        "permissions_returned": items.len(),
        "permissions_truncated": items.len() < permissions.len(),
        "offset": offset,
        "next_offset": next_offset,
        "active_count": active_count,
        "drifted_count": drifted_count,
        "revoked_count": revoked_count,
        "permissions": items,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permission(path: PathBuf, identity: RootIdentity) -> SourceRootPermission {
        SourceRootPermission {
            id: SourcePermissionId::new(),
            path,
            finding_id: "finding_x".into(),
            snapshot_id: "scan_x".into(),
            identity,
            granted_at: 1,
            revoked_at: None,
        }
    }

    #[test]
    fn permission_ids_are_prefixed_and_parse_round_trips() {
        let id = SourcePermissionId::new();
        assert!(id.as_str().starts_with("sroot_"));
        assert_eq!(SourcePermissionId::parse(id.to_string()).unwrap(), id);
        assert!(SourcePermissionId::parse("finding_x").is_err());
    }

    #[test]
    fn classification_of_resolution_states_is_exact() {
        let granted = Path::new("/sources/shared");
        assert_eq!(
            classify_resolution(granted, granted, true),
            (SourceRootState::Active, None)
        );
        let (state, reason) = classify_resolution(granted, granted, false);
        assert_eq!(state, SourceRootState::Replaced);
        assert!(reason.is_some());
        let moved = Path::new("/sources/elsewhere");
        let (state, reason) = classify_resolution(moved, granted, true);
        assert_eq!(state, SourceRootState::Retargeted);
        assert!(reason.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn freeze_reports_missing_and_retargeted_roots_without_touching_other_roots() {
        let temp = tempfile::tempdir().unwrap();
        let granted_input = temp.path().join("shared");
        fs::create_dir(&granted_input).unwrap();
        let granted = fs::canonicalize(&granted_input).unwrap();
        let identity = capture_identity(&granted).unwrap();
        let active = permission(granted.clone(), identity.clone());

        let frozen = freeze_permission(&active);
        assert_eq!(frozen.state, SourceRootState::Active);
        assert_eq!(frozen.resolved_path.as_deref(), Some(granted.as_path()));

        let retargeted = permission(granted.clone(), identity.clone());
        fs::remove_dir(&granted).unwrap();
        let replacement = temp.path().join("replacement");
        fs::create_dir(&replacement).unwrap();
        std::os::unix::fs::symlink(&replacement, &granted).unwrap();
        let frozen = freeze_permission(&retargeted);
        assert_eq!(frozen.state, SourceRootState::Retargeted);
        assert_eq!(
            frozen.resolved_path.as_deref(),
            Some(fs::canonicalize(&replacement).unwrap().as_path())
        );

        let missing = permission(temp.path().join("absent"), identity);
        let frozen = freeze_permission(&missing);
        assert_eq!(frozen.state, SourceRootState::Missing);
    }

    #[test]
    fn policy_value_reports_active_revoked_and_drift_counts() {
        let store = StateStore::open_in_memory().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let granted_input = temp.path().join("shared");
        fs::create_dir(&granted_input).unwrap();
        let granted = fs::canonicalize(&granted_input).unwrap();
        let identity = capture_identity(&granted).unwrap();
        let mut permission = permission(granted.clone(), identity);
        store.save_source_root_permission(&permission).unwrap();
        permission.revoked_at = Some(2);
        store.save_source_root_permission(&permission).unwrap();

        let value = policy_value(&store, true, 100, 0).unwrap();
        assert_eq!(value["permission_count"], 1);
        assert_eq!(value["active_count"], 0);
        assert_eq!(value["revoked_count"], 1);
        assert_eq!(value["permissions"][0]["state"], "revoked");
    }

    #[test]
    fn policy_value_pages_bounded_permission_records() {
        let store = StateStore::open_in_memory().unwrap();
        let temp = tempfile::tempdir().unwrap();
        for name in ["one", "two"] {
            let input = temp.path().join(name);
            fs::create_dir(&input).unwrap();
            let path = fs::canonicalize(input).unwrap();
            store
                .save_source_root_permission(&permission(
                    path.clone(),
                    capture_identity(&path).unwrap(),
                ))
                .unwrap();
        }

        let first = policy_value(&store, true, 1, 0).unwrap();
        assert_eq!(first["permission_count"], 2);
        assert_eq!(first["permissions_returned"], 1);
        assert_eq!(first["offset"], 0);
        assert_eq!(first["next_offset"], 1);
        let second = policy_value(&store, true, 1, 1).unwrap();
        assert_eq!(second["permissions_returned"], 1);
        assert_eq!(second["offset"], 1);
        assert!(second["next_offset"].is_null());
    }

    #[cfg(unix)]
    #[test]
    fn freeze_rejects_a_replaced_directory_at_the_same_path() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("shared");
        fs::create_dir(&input).unwrap();
        let granted = fs::canonicalize(&input).unwrap();
        let approved = permission(granted.clone(), capture_identity(&granted).unwrap());

        fs::remove_dir(&granted).unwrap();
        fs::create_dir(&granted).unwrap();

        let frozen = freeze_permission(&approved);
        assert_eq!(frozen.state, SourceRootState::Replaced);
        assert_eq!(frozen.resolved_path.as_deref(), Some(granted.as_path()));

        let store = StateStore::open_in_memory().unwrap();
        store.save_source_root_permission(&approved).unwrap();
        let value = policy_value(&store, true, 100, 0).unwrap();
        assert_eq!(value["active_count"], 0);
        assert_eq!(value["drifted_count"], 1);
        assert_eq!(value["revoked_count"], 0);
    }

    #[cfg(windows)]
    #[test]
    fn windows_identity_capture_rejects_a_replaced_directory() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("shared");
        fs::create_dir(&input).unwrap();
        let granted = fs::canonicalize(&input).unwrap();
        let approved = permission(granted.clone(), capture_identity(&granted).unwrap());

        fs::remove_dir(&granted).unwrap();
        fs::create_dir(&granted).unwrap();

        let frozen = freeze_permission(&approved);
        assert_eq!(frozen.state, SourceRootState::Replaced);
        assert_eq!(frozen.resolved_path.as_deref(), Some(granted.as_path()));
    }
}
