use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::model::{
    AgentId, AgentRecord, EvidenceId, EvidenceRecord, FindingId, FindingRecord, PlanId, PlanRecord,
    PlanStatus, ReceiptId, ReceiptRecord, ReceiptStatus, ReportId, ReportRecord, RootRecord,
    RosterEntry, ScanId, ScanRun, SkillId, SkillRecord, UsageEvent,
};
use crate::source_policy::{RootIdentity, SourcePermissionId, SourceRootPermission};
use crate::state_security;

const SCHEMA_VERSION: i64 = 12;
const REPORT_EXISTS_FOR_SCAN_SQL: &str = "SELECT EXISTS(
        SELECT 1 FROM reports r
        JOIN scan_payloads p ON p.scan_id = r.scan_id
        WHERE r.scan_id = ?1 AND r.scan_payload_updated_at = p.updated_at
    )";

fn sqlite_nofollow_path(path: &Path) -> StorageResult<PathBuf> {
    let parent = path.parent().ok_or_else(|| {
        StorageError::InvalidData(format!(
            "state database path has no parent: {}",
            path.display()
        ))
    })?;
    let name = path.file_name().ok_or_else(|| {
        StorageError::InvalidData(format!(
            "state database path has no file name: {}",
            path.display()
        ))
    })?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let parent = parent.canonicalize().map_err(|error| {
        StorageError::InvalidData(format!(
            "cannot resolve state database directory {}: {error}",
            parent.display()
        ))
    })?;
    Ok(parent.join(name))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LifecycleCounts {
    pub evidence_rows: u64,
    pub raw_usage_rows: u64,
    pub monthly_usage_rows: u64,
    pub oldest_raw_usage_at: Option<i64>,
    pub plans: u64,
    pub receipts: u64,
    pub evidence_exclusions: u64,
    pub source_root_permissions: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotInvalidationKind {
    AppliedReceipt,
    UndoReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotInvalidation {
    pub receipt_id: ReceiptId,
    pub kind: SnapshotInvalidationKind,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PurgeCounts {
    pub aggregated_raw_usage_rows: u64,
    pub deleted_raw_usage_rows: u64,
    pub deleted_evidence_rows: u64,
    pub deleted_payload_usage_summaries: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlanReceiptPurgeCounts {
    pub plans: u64,
    pub receipts: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryReceiptSaveOutcome {
    pub inserted: bool,
    pub plan_transitioned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceBaseline {
    pub source: String,
    pub revision: String,
    pub entrypoint_digest: String,
    pub first_observed_scan_id: ScanId,
    pub first_observed_at: i64,
    pub trusted_digest: Option<String>,
    pub trusted_by_receipt_id: Option<ReceiptId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedSourceBaseline {
    pub source: String,
    pub revision: String,
    pub digest: String,
    pub scan_id: ScanId,
    pub observed_at: i64,
}

#[derive(Debug)]
pub enum StorageError {
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    InvalidData(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => write!(formatter, "SQLite error: {error}"),
            Self::Json(error) => write!(formatter, "stored JSON is invalid: {error}"),
            Self::InvalidData(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sql(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::InvalidData(_) => None,
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub type StorageResult<T> = Result<T, StorageError>;
type Migration = for<'a> fn(&Transaction<'a>) -> StorageResult<()>;

const MIGRATIONS: [(i64, Migration); SCHEMA_VERSION as usize] = [
    (1, migration_v1),
    (2, migration_v2),
    (3, migration_v3),
    (4, migration_v4),
    (5, migration_v5),
    (6, migration_v6),
    (7, migration_v7),
    (8, migration_v8),
    (9, migration_v9),
    (10, migration_v10),
    (11, migration_v11),
    (12, migration_v12),
];

fn validate_migrations(migrations: &[(i64, Migration)], schema_version: i64) -> StorageResult<()> {
    if i64::try_from(migrations.len()) != Ok(schema_version) {
        return Err(StorageError::InvalidData(format!(
            "migration registry has {} entries for schema {schema_version}",
            migrations.len()
        )));
    }
    for (index, (target, _)) in migrations.iter().enumerate() {
        let expected = i64::try_from(index)
            .map_err(|_| StorageError::InvalidData("migration index is too large".into()))?
            + 1;
        if *target != expected {
            return Err(StorageError::InvalidData(format!(
                "migration registry expected target {expected}, found {target}"
            )));
        }
    }
    Ok(())
}

pub struct StateStore {
    connection: Connection,
}

impl StateStore {
    pub fn requires_migration(path: impl AsRef<Path>) -> StorageResult<bool> {
        let path = path.as_ref();
        match std::fs::symlink_metadata(path) {
            Ok(_) => state_security::secure_file(path).map_err(|error| {
                StorageError::InvalidData(format!(
                    "cannot secure state database {}: {error}",
                    path.display()
                ))
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => {
                return Err(StorageError::InvalidData(format!(
                    "cannot inspect state database {}: {error}",
                    path.display()
                )));
            }
        }
        let connection = Connection::open_with_flags(
            sqlite_nofollow_path(path)?,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        Ok(version != SCHEMA_VERSION)
    }

    /// Opens a SQLite savepoint around normalized Snapshot persistence. The
    /// initial `running` row is intentionally written before this boundary so
    /// a failed attempt can be retained as `failed` after rollback.
    pub fn begin_scan_snapshot(&self) -> StorageResult<()> {
        self.connection
            .execute_batch("SAVEPOINT skillroster_scan")?;
        Ok(())
    }

    pub fn commit_scan_snapshot(&self) -> StorageResult<()> {
        self.connection.execute_batch("RELEASE skillroster_scan")?;
        Ok(())
    }

    pub fn rollback_scan_snapshot(&self) -> StorageResult<()> {
        self.connection
            .execute_batch("ROLLBACK TO skillroster_scan; RELEASE skillroster_scan")?;
        Ok(())
    }

    pub fn open(path: impl AsRef<Path>) -> StorageResult<Self> {
        if let Some(parent) = path.as_ref().parent() {
            state_security::prepare_state_root(parent).map_err(|error| {
                StorageError::InvalidData(format!(
                    "cannot secure state directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        let path = path.as_ref();
        state_security::prepare_private_file(path).map_err(|error| {
            StorageError::InvalidData(format!(
                "cannot prepare state database {}: {error}",
                path.display()
            ))
        })?;
        let connection = Connection::open_with_flags(
            sqlite_nofollow_path(path)?,
            OpenFlags::default() | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        let store = Self::initialize(connection)?;
        for sidecar in [path.with_extension("db-wal"), path.with_extension("db-shm")] {
            state_security::secure_optional_file(&sidecar).map_err(|error| {
                StorageError::InvalidData(format!(
                    "cannot secure state database sidecar {}: {error}",
                    sidecar.display()
                ))
            })?;
        }
        Ok(store)
    }

    pub fn open_in_memory() -> StorageResult<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> StorageResult<Self> {
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "busy_timeout", 5_000_i64)?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> StorageResult<()> {
        let mut current: i64 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current > SCHEMA_VERSION {
            return Err(StorageError::InvalidData(format!(
                "database schema {current} is newer than supported schema {SCHEMA_VERSION}"
            )));
        }
        if current < 0 {
            return Err(StorageError::InvalidData(format!(
                "database schema {current} cannot be negative"
            )));
        }
        validate_migrations(&MIGRATIONS, SCHEMA_VERSION)?;
        for (target, migration) in MIGRATIONS {
            if current + 1 != target {
                continue;
            }
            self.run_migration(target, migration)?;
            current = target;
        }
        if current != SCHEMA_VERSION {
            return Err(StorageError::InvalidData(format!(
                "migration stopped at schema {current}, expected {SCHEMA_VERSION}"
            )));
        }
        Ok(())
    }

    fn run_migration(&mut self, target: i64, migration: Migration) -> StorageResult<()> {
        let transaction = self.connection.transaction()?;
        migration(&transaction)?;
        transaction.pragma_update(None, "user_version", target)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn schema_version(&self) -> StorageResult<i64> {
        Ok(self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    /// Stores one exact local source-root read permission. Immutable approval
    /// facts cannot be replaced; the only allowed update is setting revoked_at.
    pub fn save_source_root_permission(
        &self,
        permission: &SourceRootPermission,
    ) -> StorageResult<()> {
        let path = permission.path.to_str().ok_or_else(|| {
            StorageError::InvalidData("source-root permission path must be valid Unicode".into())
        })?;
        let identity = json(&permission.identity)?;
        if let Some(existing) = self.get_source_root_permission(&permission.id)? {
            if existing.path != permission.path
                || existing.finding_id != permission.finding_id
                || existing.snapshot_id != permission.snapshot_id
                || existing.identity != permission.identity
                || existing.granted_at != permission.granted_at
                || existing.revoked_at.is_some()
                || permission.revoked_at.is_none()
            {
                return Err(StorageError::InvalidData(format!(
                    "source-root permission {} immutable approval facts changed",
                    permission.id
                )));
            }
            self.connection.execute(
                "UPDATE source_root_permissions SET revoked_at = ?1
                 WHERE id = ?2 AND revoked_at IS NULL",
                params![permission.revoked_at, permission.id.as_str()],
            )?;
            return Ok(());
        }
        self.connection.execute(
            "INSERT INTO source_root_permissions
                (id, canonical_path, finding_id, snapshot_id, identity_json, granted_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                permission.id.as_str(),
                path,
                permission.finding_id,
                permission.snapshot_id,
                identity,
                permission.granted_at,
                permission.revoked_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_source_root_permission(
        &self,
        id: &SourcePermissionId,
    ) -> StorageResult<Option<SourceRootPermission>> {
        self.connection
            .query_row(
                "SELECT canonical_path, finding_id, snapshot_id, identity_json,
                        granted_at, revoked_at
                 FROM source_root_permissions WHERE id = ?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                    ))
                },
            )
            .optional()?
            .map(
                |(path, finding_id, snapshot_id, identity, granted_at, revoked_at)| {
                    Ok(SourceRootPermission {
                        id: id.clone(),
                        path: path.into(),
                        finding_id,
                        snapshot_id,
                        identity: from_json::<RootIdentity>(&identity)?,
                        granted_at,
                        revoked_at,
                    })
                },
            )
            .transpose()
    }

    pub fn list_source_root_permissions(&self) -> StorageResult<Vec<SourceRootPermission>> {
        let mut statement = self.connection.prepare(
            "SELECT id, canonical_path, finding_id, snapshot_id, identity_json,
                    granted_at, revoked_at
             FROM source_root_permissions ORDER BY granted_at DESC, id",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })?
            .map(|row| {
                let (id, path, finding_id, snapshot_id, identity, granted_at, revoked_at) = row?;
                Ok(SourceRootPermission {
                    id: SourcePermissionId::parse(id).map_err(invalid_id)?,
                    path: path.into(),
                    finding_id,
                    snapshot_id,
                    identity: from_json::<RootIdentity>(&identity)?,
                    granted_at,
                    revoked_at,
                })
            })
            .collect()
    }

    pub fn save_scan(&self, scan: &ScanRun) -> StorageResult<()> {
        self.connection.execute(
            "INSERT INTO scans (id, started_at, completed_at, status, coverage_notes_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                completed_at = excluded.completed_at,
                status = excluded.status,
                coverage_notes_json = excluded.coverage_notes_json",
            params![
                scan.id.as_str(),
                scan.started_at,
                scan.completed_at,
                enum_text(&scan.status)?,
                json(&scan.coverage_notes)?,
            ],
        )?;
        Ok(())
    }

    pub fn latest_completed_scan(&self) -> StorageResult<Option<ScanRun>> {
        let stored = self
            .connection
            .query_row(
                "SELECT id, started_at, completed_at, status, coverage_notes_json
                 FROM scans WHERE status = 'completed'
                 ORDER BY completed_at DESC, started_at DESC, rowid DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(|(id, started_at, completed_at, status, notes)| {
                Ok(ScanRun {
                    id: ScanId::parse(id).map_err(invalid_id)?,
                    started_at,
                    completed_at,
                    status: enum_from_text(&status)?,
                    coverage_notes: from_json(&notes)?,
                })
            })
            .transpose()
    }

    /// Persists normalized scan facts only; raw session text is never stored.
    pub fn save_scan_payload<T: Serialize>(&self, id: &ScanId, payload: &T) -> StorageResult<()> {
        self.connection.execute(
            "INSERT INTO scan_payloads (scan_id, payload_json, updated_at)
             VALUES (?1, ?2, CAST(strftime('%s', 'now') AS INTEGER))
             ON CONFLICT(scan_id) DO UPDATE SET payload_json = excluded.payload_json,
                 updated_at = MAX(scan_payloads.updated_at + 1, excluded.updated_at)",
            params![id.as_str(), json(payload)?],
        )?;
        Ok(())
    }

    pub fn latest_scan_payload<T: DeserializeOwned>(&self) -> StorageResult<Option<(ScanId, T)>> {
        let stored = self
            .connection
            .query_row(
                "SELECT p.scan_id, p.payload_json FROM scan_payloads p
                 JOIN scans s ON s.id = p.scan_id
                 WHERE s.status = 'completed'
                 ORDER BY s.completed_at DESC, s.started_at DESC, s.rowid DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        stored
            .map(|(id, payload)| Ok((ScanId::parse(id).map_err(invalid_id)?, from_json(&payload)?)))
            .transpose()
    }

    pub fn snapshot_invalidating_receipt(
        &self,
        scan_id: &ScanId,
    ) -> StorageResult<Option<SnapshotInvalidation>> {
        self.connection
            .query_row(
                "SELECT r.id, r.reverses_receipt_id IS NOT NULL FROM snapshot_invalidations i
                 JOIN receipts r ON r.id = i.receipt_id
                 WHERE i.snapshot_id = ?1
                   AND ((r.reverses_receipt_id IS NULL AND r.status = 'applied')
                     OR (r.reverses_receipt_id IS NOT NULL AND r.status = 'undone'))
                 ORDER BY i.created_at DESC, i.rowid DESC
                 LIMIT 1",
                [scan_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?
            .map(|(id, is_undo)| {
                Ok(SnapshotInvalidation {
                    receipt_id: ReceiptId::parse(id).map_err(invalid_id)?,
                    kind: if is_undo {
                        SnapshotInvalidationKind::UndoReceipt
                    } else {
                        SnapshotInvalidationKind::AppliedReceipt
                    },
                })
            })
            .transpose()
    }

    pub fn scan_payload<T: DeserializeOwned>(&self, id: &ScanId) -> StorageResult<Option<T>> {
        self.connection
            .query_row(
                "SELECT payload_json FROM scan_payloads WHERE scan_id = ?1",
                [id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|payload| from_json(&payload))
            .transpose()
    }

    pub fn latest_report(&self) -> StorageResult<Option<ReportRecord>> {
        let stored = self
            .connection
            .query_row(
                "SELECT r.id, r.scan_id, r.created_at, r.summary_json FROM reports r
                 JOIN scan_payloads p ON p.scan_id = r.scan_id
                 WHERE r.scan_payload_updated_at = p.updated_at
                 ORDER BY r.created_at DESC, r.rowid DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(|(id, scan_id, created_at, summary)| {
                Ok(ReportRecord {
                    id: ReportId::parse(id).map_err(invalid_id)?,
                    scan_id: ScanId::parse(scan_id).map_err(invalid_id)?,
                    created_at,
                    summary: from_json(&summary)?,
                })
            })
            .transpose()
    }

    pub fn report_exists_for_scan(&self, id: &ScanId) -> StorageResult<bool> {
        Ok(self
            .connection
            .query_row(REPORT_EXISTS_FOR_SCAN_SQL, [id.as_str()], |row| row.get(0))?)
    }

    pub fn save_agent(&self, agent: &AgentRecord) -> StorageResult<AgentId> {
        let kind = enum_text(&agent.kind)?;
        if let Some(id) = self
            .connection
            .query_row("SELECT id FROM agents WHERE kind = ?1", [&kind], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
        {
            self.connection.execute(
                "UPDATE agents SET display_name = ?1 WHERE id = ?2",
                params![agent.display_name, id],
            )?;
            return AgentId::parse(id).map_err(invalid_id);
        }
        self.connection.execute(
            "INSERT INTO agents (id, kind, display_name) VALUES (?1, ?2, ?3)",
            params![agent.id.as_str(), kind, agent.display_name],
        )?;
        Ok(agent.id.clone())
    }

    pub fn agent_id(&self, kind: &crate::model::AgentKind) -> StorageResult<Option<AgentId>> {
        let kind = enum_text(kind)?;
        self.connection
            .query_row("SELECT id FROM agents WHERE kind = ?1", [kind], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .map(|id| AgentId::parse(id).map_err(invalid_id))
            .transpose()
    }

    pub fn save_root(&self, root: &RootRecord) -> StorageResult<()> {
        self.connection.execute(
            "INSERT INTO roots (id, scan_id, agent_id, path, kind, status, explicit, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET status = excluded.status, detail = excluded.detail",
            params![
                root.id.as_str(),
                root.scan_id.as_str(),
                root.agent_id.as_ref().map(AgentId::as_str),
                root.path,
                root.kind,
                enum_text(&root.status)?,
                root.explicit,
                root.detail,
            ],
        )?;
        Ok(())
    }

    /// Inserts a logical Skill or refreshes the existing identity while preserving its stable ID.
    pub fn save_skill(&self, skill: &SkillRecord) -> StorageResult<SkillId> {
        if let Some(id) = self
            .connection
            .query_row(
                "SELECT id FROM skills WHERE identity_key = ?1",
                [&skill.identity_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            self.connection.execute(
                "UPDATE skills SET name = ?1, description = ?2, declared_source = ?3,
                    declared_revision = ?4, content_digest = ?5, digest_version = ?6,
                    canonical_path = ?7 WHERE id = ?8",
                params![
                    skill.name,
                    skill.description,
                    skill.declared_source,
                    skill.declared_revision,
                    skill.content_digest,
                    skill.digest_version,
                    skill.canonical_path,
                    id,
                ],
            )?;
            return SkillId::parse(id).map_err(invalid_id);
        }
        if let Some(existing_identity) = self
            .connection
            .query_row(
                "SELECT identity_key FROM skills WHERE id = ?1",
                [skill.id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let expected_weak_identity = format!("incomplete-snapshot-skill:{}", skill.id.as_str());
            if skill.identity_key == expected_weak_identity {
                return Ok(skill.id.clone());
            }
            return Err(StorageError::InvalidData(format!(
                "Skill {} conflicts with retained identity {existing_identity}",
                skill.id
            )));
        }
        self.connection.execute(
            "INSERT INTO skills
                (id, identity_key, name, description, declared_source, declared_revision,
                 content_digest, digest_version, governance_state, canonical_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                skill.id.as_str(),
                skill.identity_key,
                skill.name,
                skill.description,
                skill.declared_source,
                skill.declared_revision,
                skill.content_digest,
                skill.digest_version,
                enum_text(&skill.governance_state)?,
                skill.canonical_path,
            ],
        )?;
        Ok(skill.id.clone())
    }

    /// Records the first observed entrypoint digest for a declared source revision.
    /// A later Scan may observe local edits, but it must never rewrite this baseline.
    pub fn record_source_baseline(&self, baseline: &SourceBaseline) -> StorageResult<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO source_baselines
                (source, revision, entrypoint_digest, first_observed_scan_id, first_observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                baseline.source,
                baseline.revision,
                baseline.entrypoint_digest,
                baseline.first_observed_scan_id.as_str(),
                baseline.first_observed_at,
            ],
        )?;
        Ok(())
    }

    pub fn source_baseline(
        &self,
        source: &str,
        revision: &str,
    ) -> StorageResult<Option<SourceBaseline>> {
        self.connection
            .query_row(
                "SELECT entrypoint_digest, first_observed_scan_id, first_observed_at,
                        trusted_digest, trusted_by_receipt_id
                 FROM source_baselines WHERE source = ?1 AND revision = ?2",
                params![source, revision],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?
            .map(
                |(entrypoint_digest, scan_id, first_observed_at, trusted_digest, receipt_id)| {
                    Ok(SourceBaseline {
                        source: source.to_owned(),
                        revision: revision.to_owned(),
                        entrypoint_digest,
                        first_observed_scan_id: ScanId::parse(scan_id).map_err(invalid_id)?,
                        first_observed_at,
                        trusted_digest,
                        trusted_by_receipt_id: receipt_id
                            .map(|id| ReceiptId::parse(id).map_err(invalid_id))
                            .transpose()?,
                    })
                },
            )
            .transpose()
    }

    pub fn skill_exists(&self, id: &SkillId) -> StorageResult<bool> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM skills WHERE id = ?1",
            [id.as_str()],
            |row| row.get(0),
        )?;
        Ok(count != 0)
    }

    pub fn skill_governance_state(
        &self,
        id: &SkillId,
    ) -> StorageResult<Option<crate::model::GovernanceState>> {
        self.connection
            .query_row(
                "SELECT governance_state FROM skills WHERE id = ?1",
                [id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|value| enum_from_text(&value))
            .transpose()
    }

    pub fn update_skill_governance_state(
        &self,
        id: &SkillId,
        state: crate::model::GovernanceState,
    ) -> StorageResult<()> {
        let changed = self.connection.execute(
            "UPDATE skills SET governance_state = ?1 WHERE id = ?2",
            params![enum_text(&state)?, id.as_str()],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidData(format!(
                "cannot update missing Skill {id}"
            )));
        }
        Ok(())
    }

    pub fn save_placement(&self, placement: &crate::model::PlacementRecord) -> StorageResult<()> {
        self.connection.execute(
            "INSERT INTO placements
                (id, scan_id, skill_id, agent_id, root_id, path, kind, symlink_target,
                 fingerprint, exposed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET scan_id = excluded.scan_id,
                 skill_id = excluded.skill_id, agent_id = excluded.agent_id,
                 root_id = excluded.root_id, path = excluded.path, kind = excluded.kind,
                 symlink_target = excluded.symlink_target,
                 fingerprint = excluded.fingerprint, exposed = excluded.exposed",
            params![
                placement.id.as_str(),
                placement.scan_id.as_str(),
                placement.skill_id.as_str(),
                placement.agent_id.as_ref().map(AgentId::as_str),
                placement.root_id.as_str(),
                placement.path,
                enum_text(&placement.kind)?,
                placement.symlink_target,
                placement.fingerprint,
                placement.exposed,
            ],
        )?;
        Ok(())
    }

    pub fn save_evidence(&self, evidence: &EvidenceRecord) -> StorageResult<()> {
        self.connection.execute(
            "INSERT INTO evidence
                (id, scan_id, kind, quality, subject_type, subject_id, path, digest,
                 details_json, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET details_json = excluded.details_json,
                observed_at = excluded.observed_at",
            params![
                evidence.id.as_str(),
                evidence.scan_id.as_str(),
                enum_text(&evidence.kind)?,
                enum_text(&evidence.quality)?,
                evidence.subject_type,
                evidence.subject_id,
                evidence.path,
                evidence.digest,
                json(&evidence.details)?,
                evidence.observed_at,
            ],
        )?;
        Ok(())
    }

    pub fn save_usage_event(&self, event: &UsageEvent) -> StorageResult<()> {
        let stage = enum_text(&event.stage)?;
        let quality = enum_text(&event.quality)?;
        let event_count = i64::try_from(event.observed_event_count).map_err(|_| {
            StorageError::InvalidData("usage event count exceeds SQLite range".to_owned())
        })?;
        let conflict_clause = if event.occurred_at.is_some() {
            "ON CONFLICT(skill_id, agent_id, stage, quality, source_path_digest,
                         observed_event_count, occurred_at) DO NOTHING"
        } else {
            "ON CONFLICT(skill_id, agent_id, stage, quality, source_path_digest,
                         observed_event_count) WHERE occurred_at IS NULL DO NOTHING"
        };
        self.connection.execute(
            &format!(
                "INSERT INTO usage_events
                (evidence_id, skill_id, agent_id, stage, quality, source_path_digest,
                 observed_event_count, occurred_at, outcome)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 {conflict_clause}"
            ),
            params![
                event.evidence_id.as_str(),
                event.skill_id.as_str(),
                event.agent_id.as_str(),
                stage,
                quality,
                event.source_path_digest,
                event_count,
                event.occurred_at,
                event.outcome,
            ],
        )?;
        Ok(())
    }

    pub fn evidence_belongs_to_scan(
        &self,
        evidence_id: &EvidenceId,
        scan_id: &ScanId,
    ) -> StorageResult<bool> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM evidence WHERE id = ?1 AND scan_id = ?2",
            params![evidence_id.as_str(), scan_id.as_str()],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    }

    pub fn get_evidence(&self, id: &EvidenceId) -> StorageResult<Option<EvidenceRecord>> {
        let stored = self
            .connection
            .query_row(
                "SELECT scan_id, kind, quality, subject_type, subject_id, path, digest,
                        details_json, observed_at
                 FROM evidence WHERE id = ?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(
                |(
                    scan_id,
                    kind,
                    quality,
                    subject_type,
                    subject_id,
                    path,
                    digest,
                    details,
                    observed_at,
                )| {
                    Ok(EvidenceRecord {
                        id: id.clone(),
                        scan_id: ScanId::parse(scan_id).map_err(invalid_id)?,
                        kind: enum_from_text(&kind)?,
                        quality: enum_from_text(&quality)?,
                        subject_type,
                        subject_id,
                        path,
                        digest,
                        details: from_json(&details)?,
                        observed_at,
                    })
                },
            )
            .transpose()
    }

    pub fn finding_evidence(&self, id: &FindingId) -> StorageResult<Vec<EvidenceRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT e.id, e.scan_id, e.kind, e.quality, e.subject_type, e.subject_id,
                    e.path, e.digest, e.details_json, e.observed_at
             FROM finding_evidence fe
             JOIN evidence e ON e.id = fe.evidence_id
             WHERE fe.finding_id = ?1",
        )?;
        let rows = statement.query_map([id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                scan_id,
                kind,
                quality,
                subject_type,
                subject_id,
                path,
                digest,
                details,
                observed_at,
            ) = row?;
            Ok(EvidenceRecord {
                id: EvidenceId::parse(id).map_err(invalid_id)?,
                scan_id: ScanId::parse(scan_id).map_err(invalid_id)?,
                kind: enum_from_text(&kind)?,
                quality: enum_from_text(&quality)?,
                subject_type,
                subject_id,
                path,
                digest,
                details: from_json(&details)?,
                observed_at,
            })
        })
        .collect()
    }

    pub fn save_roster_entry(&self, entry: &RosterEntry) -> StorageResult<()> {
        self.connection.execute(
            "INSERT INTO roster_entries (agent_id, skill_id, state, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id, skill_id) DO UPDATE SET
                 state = excluded.state, updated_at = excluded.updated_at",
            params![
                entry.agent_id.as_str(),
                entry.skill_id.as_str(),
                enum_text(&entry.state)?,
                entry.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn roster_entry(
        &self,
        agent_id: &AgentId,
        skill_id: &SkillId,
    ) -> StorageResult<Option<RosterEntry>> {
        let stored = self
            .connection
            .query_row(
                "SELECT state, updated_at FROM roster_entries
                 WHERE agent_id = ?1 AND skill_id = ?2",
                params![agent_id.as_str(), skill_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        stored
            .map(|(state, updated_at)| {
                Ok(RosterEntry {
                    agent_id: agent_id.clone(),
                    skill_id: skill_id.clone(),
                    state: enum_from_text(&state)?,
                    updated_at,
                })
            })
            .transpose()
    }

    pub fn roster_states_for_skill(
        &self,
        skill_id: &SkillId,
    ) -> StorageResult<Vec<crate::model::RosterState>> {
        let mut statement = self
            .connection
            .prepare("SELECT state FROM roster_entries WHERE skill_id = ?1 ORDER BY agent_id")?;
        let values = statement
            .query_map([skill_id.as_str()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        values
            .into_iter()
            .map(|value| enum_from_text(&value))
            .collect()
    }

    pub fn delete_roster_entry(&self, agent_id: &AgentId, skill_id: &SkillId) -> StorageResult<()> {
        self.connection.execute(
            "DELETE FROM roster_entries WHERE agent_id = ?1 AND skill_id = ?2",
            params![agent_id.as_str(), skill_id.as_str()],
        )?;
        Ok(())
    }

    pub fn index_skill(
        &self,
        skill_id: &SkillId,
        name: &str,
        description: &str,
        triggers: &str,
        body: &str,
    ) -> StorageResult<()> {
        self.connection.execute(
            "DELETE FROM skills_fts WHERE skill_id = ?1",
            [skill_id.as_str()],
        )?;
        self.connection.execute(
            "INSERT INTO skills_fts (skill_id, name, description, triggers, body)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![skill_id.as_str(), name, description, triggers, body],
        )?;
        Ok(())
    }

    /// Returns local Skill IDs selected by SQLite FTS5. User input is reduced
    /// to quoted unicode word terms so FTS operators cannot alter the query.
    pub fn search_skill_ids(&self, task: &str, limit: usize) -> StorageResult<Vec<SkillId>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let terms = task
            .split(|character: char| !character.is_alphanumeric())
            .filter(|term| !term.is_empty())
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let expression = terms.join(" OR ");
        let mut statement = self.connection.prepare(
            "SELECT skill_id FROM skills_fts
             WHERE skills_fts MATCH ?1
             ORDER BY bm25(skills_fts, 0.0, 8.0, 5.0, 6.0, 1.0), skill_id
             LIMIT ?2",
        )?;
        let mut ids = statement
            .query_map(params![expression, limit as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if ids.is_empty() {
            let mut fallback = self.connection.prepare(
                "SELECT skill_id FROM skills_fts
                 WHERE instr(lower(name), lower(?1)) > 0
                    OR instr(lower(description), lower(?1)) > 0
                    OR instr(lower(triggers), lower(?1)) > 0
                    OR instr(lower(body), lower(?1)) > 0
                 ORDER BY skill_id LIMIT ?2",
            )?;
            ids = fallback
                .query_map(params![task.trim(), limit as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
        }
        ids.into_iter()
            .map(|id| SkillId::parse(id).map_err(invalid_id))
            .collect()
    }

    pub fn save_report(
        &self,
        report: &ReportRecord,
        findings: &[FindingRecord],
    ) -> StorageResult<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let inserted = transaction.execute(
            "INSERT INTO reports
                (id, scan_id, created_at, scan_payload_updated_at, summary_json)
             SELECT ?1, ?2, ?3, p.updated_at, ?4
             FROM scan_payloads p WHERE p.scan_id = ?2",
            params![
                report.id.as_str(),
                report.scan_id.as_str(),
                report.created_at,
                json(&report.summary)?,
            ],
        )?;
        if inserted != 1 {
            return Err(StorageError::InvalidData(format!(
                "Report {} has no persisted Snapshot payload",
                report.id
            )));
        }
        for finding in findings {
            save_finding(&transaction, finding)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get_report(&self, id: &ReportId) -> StorageResult<Option<ReportRecord>> {
        let stored = self
            .connection
            .query_row(
                "SELECT scan_id, created_at, summary_json FROM reports WHERE id = ?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(|(scan_id, created_at, summary)| {
                Ok(ReportRecord {
                    id: id.clone(),
                    scan_id: ScanId::parse(scan_id).map_err(invalid_id)?,
                    created_at,
                    summary: from_json(&summary)?,
                })
            })
            .transpose()
    }

    pub fn get_finding(&self, id: &FindingId) -> StorageResult<Option<FindingRecord>> {
        let stored = self
            .connection
            .query_row(
                "SELECT report_id, category, severity, title, summary, details_json
                 FROM findings WHERE id = ?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((report_id, category, severity, title, summary, details)) = stored else {
            return Ok(None);
        };
        let mut statement = self.connection.prepare(
            "SELECT evidence_id FROM finding_evidence WHERE finding_id = ?1 ORDER BY evidence_id",
        )?;
        let evidence_ids = statement
            .query_map([id.as_str()], |row| row.get::<_, String>(0))?
            .map(|result| {
                EvidenceId::parse(result?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(FindingRecord {
            id: id.clone(),
            report_id: ReportId::parse(report_id).map_err(invalid_id)?,
            category: enum_from_text(&category)?,
            severity: enum_from_text(&severity)?,
            title,
            summary,
            details: from_json(&details)?,
            evidence_ids,
        }))
    }

    /// Stores an immutable Plan. Reusing an ID with different content is rejected.
    pub fn save_plan(&self, plan: &PlanRecord) -> StorageResult<()> {
        let encoded = json(plan)?;
        if let Some(existing) = self
            .connection
            .query_row(
                "SELECT immutable_json FROM plans WHERE id = ?1",
                [plan.id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            if existing == encoded {
                return Ok(());
            }
            return Err(StorageError::InvalidData(format!(
                "plan {} already exists with different immutable content",
                plan.id
            )));
        }

        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO plans
                (id, scan_id, report_id, created_at, status, input_json, fingerprint, immutable_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                plan.id.as_str(),
                plan.scan_id.as_str(),
                plan.report_id.as_ref().map(ReportId::as_str),
                plan.created_at,
                enum_text(&plan.status)?,
                json(&plan.input)?,
                plan.fingerprint,
                encoded,
            ],
        )?;
        for operation in &plan.operations {
            transaction.execute(
                "INSERT INTO plan_operations
                    (id, plan_id, position, target_path, expected_fingerprint, action_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    operation.id.as_str(),
                    plan.id.as_str(),
                    operation.position,
                    operation.target_path,
                    operation.expected_fingerprint,
                    json(&operation.action)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get_plan(&self, id: &PlanId) -> StorageResult<Option<PlanRecord>> {
        let stored = self
            .connection
            .query_row(
                "SELECT immutable_json, status FROM plans WHERE id = ?1",
                [id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        stored
            .map(|(encoded, status)| {
                let mut plan: PlanRecord = from_json(&encoded)?;
                plan.status = enum_from_text(&status)?;
                Ok(plan)
            })
            .transpose()
    }

    pub fn ready_plan_with_reuse_identity(
        &self,
        scan_id: &ScanId,
        fingerprint: &str,
        raw: &serde_json::Value,
        reuse_identity: Option<&serde_json::Value>,
    ) -> StorageResult<Option<PlanRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT immutable_json, status FROM plans
             WHERE scan_id = ?1 AND fingerprint = ?2 AND status = 'ready'
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = statement.query_map(params![scan_id.as_str(), fingerprint], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (encoded, status) = row?;
            let mut plan: PlanRecord = from_json(&encoded)?;
            plan.status = enum_from_text(&status)?;
            if plan.input.get("raw") == Some(raw)
                && plan.input.get("reuse_identity") == reuse_identity
            {
                return Ok(Some(plan));
            }
        }
        Ok(None)
    }

    pub fn update_plan_status(&self, id: &PlanId, next: PlanStatus) -> StorageResult<()> {
        let current: Option<String> = self
            .connection
            .query_row(
                "SELECT status FROM plans WHERE id = ?1",
                [id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let current = current
            .ok_or_else(|| StorageError::InvalidData(format!("cannot update missing plan {id}")))?;
        let current: PlanStatus = enum_from_text(&current)?;
        if !valid_plan_transition(&current, &next) {
            return Err(StorageError::InvalidData(format!(
                "invalid plan status transition {current:?} -> {next:?}"
            )));
        }
        self.connection.execute(
            "UPDATE plans SET status = ?1 WHERE id = ?2",
            params![enum_text(&next)?, id.as_str()],
        )?;
        Ok(())
    }

    pub fn save_receipt(&self, receipt: &ReceiptRecord) -> StorageResult<()> {
        let transaction = self.connection.unchecked_transaction()?;
        save_receipt_transaction(&transaction, receipt)?;
        transaction.commit()?;
        Ok(())
    }

    /// Commits the filesystem Receipt projection and terminal Plan state as one
    /// SQLite transaction. The filesystem journal remains the recovery source if
    /// this transaction cannot commit.
    pub fn save_apply_receipt(
        &self,
        plan: &PlanId,
        next: PlanStatus,
        receipt: &ReceiptRecord,
    ) -> StorageResult<()> {
        if receipt.plan_id != *plan
            || !matches!(
                next,
                PlanStatus::Applied | PlanStatus::FailedRolledBack | PlanStatus::RecoveryRequired
            )
        {
            return Err(StorageError::InvalidData(
                "Apply receipt does not match the terminal Plan state".to_owned(),
            ));
        }
        self.save_apply_receipt_with_trusted_sources(plan, next, receipt, &[])
    }

    pub fn save_apply_receipt_with_trusted_sources(
        &self,
        plan: &PlanId,
        next: PlanStatus,
        receipt: &ReceiptRecord,
        trusted_sources: &[TrustedSourceBaseline],
    ) -> StorageResult<()> {
        if receipt.plan_id != *plan
            || !matches!(
                next,
                PlanStatus::Applied | PlanStatus::FailedRolledBack | PlanStatus::RecoveryRequired
            )
        {
            return Err(StorageError::InvalidData(
                "Apply receipt does not match the terminal Plan state".to_owned(),
            ));
        }
        if !trusted_sources.is_empty()
            && (next != PlanStatus::Applied || receipt.status != ReceiptStatus::Applied)
        {
            return Err(StorageError::InvalidData(
                "trusted source baselines require a verified Applied receipt".to_owned(),
            ));
        }
        let transaction = self.connection.unchecked_transaction()?;
        save_receipt_transaction(&transaction, receipt)?;
        if next == PlanStatus::Applied && receipt.status == ReceiptStatus::Applied {
            let changed = transaction.execute(
                "INSERT INTO snapshot_invalidations (snapshot_id, receipt_id, created_at)
                 SELECT scan_id, ?1, ?2 FROM plans WHERE id = ?3",
                params![receipt.id.as_str(), receipt.created_at, plan.as_str()],
            )?;
            if changed != 1 {
                return Err(StorageError::InvalidData(format!(
                    "plan {plan} has no Snapshot to invalidate"
                )));
            }
        }
        for source in trusted_sources {
            let changed = transaction.execute(
                "INSERT INTO source_baselines
                    (source, revision, entrypoint_digest, first_observed_scan_id,
                     first_observed_at, trusted_digest, trusted_by_receipt_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?3, ?6)
                 ON CONFLICT(source, revision) DO UPDATE SET
                    trusted_digest = excluded.trusted_digest,
                    trusted_by_receipt_id = excluded.trusted_by_receipt_id
                 WHERE source_baselines.trusted_digest IS NULL
                    OR source_baselines.trusted_digest = excluded.trusted_digest",
                params![
                    source.source,
                    source.revision,
                    source.digest,
                    source.scan_id.as_str(),
                    source.observed_at,
                    receipt.id.as_str(),
                ],
            )?;
            if changed != 1 {
                return Err(StorageError::InvalidData(format!(
                    "trusted source baseline conflicts for {}@{}",
                    source.source, source.revision
                )));
            }
        }
        let changed = transaction.execute(
            "UPDATE plans SET status = ?1 WHERE id = ?2 AND status = 'applying'",
            params![enum_text(&next)?, plan.as_str()],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidData(format!(
                "plan {plan} is no longer applying"
            )));
        }
        transaction.commit()?;
        Ok(())
    }

    /// Commits an Undo receipt and consumes the original receipt atomically.
    /// A failed commit therefore cannot leave SQLite claiming that the original
    /// receipt is still undoable while also recording a successful reverse.
    pub fn save_undo_receipt(
        &self,
        original: &ReceiptId,
        receipt: &ReceiptRecord,
        invalidates_snapshot_id: Option<&ScanId>,
    ) -> StorageResult<()> {
        if receipt.reverses_receipt_id.as_ref() != Some(original)
            || receipt.status != ReceiptStatus::Undone
        {
            return Err(StorageError::InvalidData(
                "Undo receipt does not match the original Receipt".to_owned(),
            ));
        }
        let transaction = self.connection.unchecked_transaction()?;
        save_receipt_transaction(&transaction, receipt)?;
        if let Some(snapshot_id) = invalidates_snapshot_id {
            transaction.execute(
                "INSERT INTO snapshot_invalidations (snapshot_id, receipt_id, created_at)
                 VALUES (?1, ?2, ?3)",
                params![
                    snapshot_id.as_str(),
                    receipt.id.as_str(),
                    receipt.created_at
                ],
            )?;
        }
        let changed = transaction.execute(
            "UPDATE receipts SET status = 'undone', completed_at = ?1
             WHERE id = ?2 AND status = 'applied'",
            params![chrono::Utc::now().timestamp(), original.as_str()],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidData(format!(
                "receipt {original} is no longer undoable"
            )));
        }
        transaction.commit()?;
        Ok(())
    }

    /// Imports a filesystem recovery Receipt in one transaction. Apply recovery
    /// transitions its Applying Plan; a verified reverse Undo keeps the terminal
    /// Plan unchanged. An already-imported recovery Receipt is accepted so older
    /// split states can converge on retry.
    pub fn save_recovery_receipt(
        &self,
        plan: &PlanId,
        receipt: &ReceiptRecord,
    ) -> StorageResult<RecoveryReceiptSaveOutcome> {
        if receipt.plan_id != *plan || receipt.status != ReceiptStatus::RecoveryRequired {
            return Err(StorageError::InvalidData(
                "Recovery receipt does not match the recovery Plan".to_owned(),
            ));
        }
        let transaction = self.connection.unchecked_transaction()?;
        let existing = transaction
            .query_row(
                "SELECT plan_id, status, reverses_receipt_id FROM receipts WHERE id = ?1",
                [receipt.id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let inserted = match existing {
            Some((existing_plan, existing_status, existing_reverse)) => {
                let expected_reverse = receipt.reverses_receipt_id.as_ref().map(ReceiptId::as_str);
                if existing_plan != plan.as_str()
                    || existing_status != "recovery_required"
                    || existing_reverse.as_deref() != expected_reverse
                {
                    return Err(StorageError::InvalidData(format!(
                        "recovery receipt {} conflicts with retained state",
                        receipt.id
                    )));
                }
                false
            }
            None => {
                save_receipt_transaction(&transaction, receipt)?;
                true
            }
        };
        let is_reverse = receipt.reverses_receipt_id.is_some();
        if let Some(original_id) = &receipt.reverses_receipt_id {
            let original = transaction
                .query_row(
                    "SELECT plan_id, status FROM receipts WHERE id = ?1",
                    [original_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            if original.as_ref().map(|(original_plan, original_status)| {
                (original_plan.as_str(), original_status.as_str())
            }) != Some((plan.as_str(), "applied"))
            {
                return Err(StorageError::InvalidData(format!(
                    "recovery receipt {} does not reverse an applied Receipt from Plan {plan}",
                    receipt.id
                )));
            }
        }
        let plan_transitioned = if is_reverse {
            false
        } else {
            transaction.execute(
                "UPDATE plans SET status = 'recovery_required'
                 WHERE id = ?1 AND status = 'applying'",
                [plan.as_str()],
            )? == 1
        };
        let plan_status = transaction
            .query_row(
                "SELECT status FROM plans WHERE id = ?1",
                [plan.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let valid_plan_status = if is_reverse {
            plan_status.as_deref() == Some("applied")
        } else {
            plan_status.as_deref() == Some("recovery_required")
        };
        if !valid_plan_status {
            return Err(StorageError::InvalidData(format!(
                "plan {plan} is not recoverable from its current state"
            )));
        }
        transaction.commit()?;
        Ok(RecoveryReceiptSaveOutcome {
            inserted,
            plan_transitioned,
        })
    }

    pub fn receipt_exists(&self, id: &ReceiptId) -> StorageResult<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM receipts WHERE id = ?1)",
            [id.as_str()],
            |row| row.get(0),
        )?)
    }

    pub fn get_receipt(&self, id: &ReceiptId) -> StorageResult<Option<ReceiptRecord>> {
        let stored = self
            .connection
            .query_row(
                "SELECT plan_id, reverses_receipt_id, created_at, completed_at, status,
                        verification_json FROM receipts WHERE id = ?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((plan_id, reverses, created_at, completed_at, status, verification)) = stored
        else {
            return Ok(None);
        };
        let mut statement = self.connection.prepare(
            "SELECT operation_id, position, status, before_state_json, after_state_json, error
             FROM receipt_operations WHERE receipt_id = ?1 ORDER BY position",
        )?;
        let rows = statement.query_map([id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut operation_results = Vec::new();
        for row in rows {
            let (operation_id, position, result_status, before, after, error) = row?;
            operation_results.push(crate::model::OperationResult {
                operation_id: crate::model::OperationId::parse(operation_id).map_err(invalid_id)?,
                position,
                status: result_status,
                before_state: from_json(&before)?,
                after_state: after.map(|value| from_json(&value)).transpose()?,
                error,
            });
        }
        Ok(Some(ReceiptRecord {
            id: id.clone(),
            plan_id: PlanId::parse(plan_id).map_err(invalid_id)?,
            reverses_receipt_id: reverses
                .map(ReceiptId::parse)
                .transpose()
                .map_err(invalid_id)?,
            created_at,
            completed_at,
            status: enum_from_text(&status)?,
            verification: from_json(&verification)?,
            operation_results,
        }))
    }

    pub fn reverse_receipt_for(&self, id: &ReceiptId) -> StorageResult<Option<ReceiptId>> {
        self.connection
            .query_row(
                "SELECT id FROM receipts
                 WHERE reverses_receipt_id = ?1 AND status = 'undone'
                 ORDER BY completed_at DESC LIMIT 1",
                [id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(ReceiptId::parse)
            .transpose()
            .map_err(invalid_id)
    }

    pub fn recovery_required(&self) -> StorageResult<bool> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM receipts WHERE status = 'recovery_required'",
            [],
            |row| row.get(0),
        )?;
        Ok(count != 0)
    }

    pub fn pending_plans(
        &self,
        latest_scan_id: Option<&ScanId>,
        limit: usize,
    ) -> StorageResult<(usize, Vec<PlanRecord>)> {
        let latest_scan_id = latest_scan_id.map(ScanId::as_str);
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM plans
             WHERE status IN ('applying', 'recovery_required')
                OR (status = 'ready' AND scan_id = ?1)",
            params![latest_scan_id],
            |row| row.get(0),
        )?;
        let mut statement = self.connection.prepare(
            "SELECT immutable_json, status FROM plans
             WHERE status IN ('applying', 'recovery_required')
                OR (status = 'ready' AND scan_id = ?1)
             ORDER BY CASE status
                 WHEN 'recovery_required' THEN 0
                 WHEN 'applying' THEN 1
                 ELSE 2
             END,
             created_at DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![latest_scan_id, limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut plans = Vec::new();
        for row in rows {
            let (encoded, status) = row?;
            let mut plan: PlanRecord = from_json(&encoded)?;
            plan.status = enum_from_text(&status)?;
            plans.push(plan);
        }
        let count = usize::try_from(count)
            .map_err(|_| StorageError::InvalidData("pending Plan count is invalid".into()))?;
        Ok((count, plans))
    }

    pub fn latest_receipt(&self) -> StorageResult<Option<ReceiptRecord>> {
        let id = self
            .connection
            .query_row(
                "SELECT id FROM receipts ORDER BY created_at DESC, id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        id.map(|id| {
            let id = ReceiptId::parse(id).map_err(invalid_id)?;
            self.get_receipt(&id)?.ok_or_else(|| {
                StorageError::InvalidData(format!("latest Receipt {id} disappeared"))
            })
        })
        .transpose()
    }

    pub fn recovery_receipts(&self) -> StorageResult<Vec<ReceiptRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM receipts WHERE status = 'recovery_required'
             ORDER BY created_at ASC, id ASC",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                let id = ReceiptId::parse(id).map_err(invalid_id)?;
                self.get_receipt(&id)?.ok_or_else(|| {
                    StorageError::InvalidData(format!("recovery Receipt {id} disappeared"))
                })
            })
            .collect()
    }

    pub fn lifecycle_counts(&self) -> StorageResult<LifecycleCounts> {
        Ok(LifecycleCounts {
            evidence_rows: table_count(&self.connection, "evidence")?,
            raw_usage_rows: table_count(&self.connection, "usage_events")?,
            monthly_usage_rows: table_count(&self.connection, "usage_monthly")?
                + table_count(&self.connection, "usage_monthly_sources")?,
            oldest_raw_usage_at: self.connection.query_row(
                "SELECT MIN(occurred_at) FROM usage_events",
                [],
                |row| row.get(0),
            )?,
            plans: table_count(&self.connection, "plans")?,
            receipts: table_count(&self.connection, "receipts")?,
            evidence_exclusions: table_count(&self.connection, "evidence_exclusions")?,
            source_root_permissions: table_count(&self.connection, "source_root_permissions")?,
        })
    }

    pub fn evidence_exclusions(&self) -> StorageResult<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT agent_kind FROM evidence_exclusions ORDER BY agent_kind")?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn set_evidence_exclusion(&self, agent_kind: &str, excluded: bool) -> StorageResult<()> {
        if excluded {
            self.connection.execute(
                "INSERT OR IGNORE INTO evidence_exclusions (agent_kind, created_at)
                 VALUES (?1, CAST(strftime('%s', 'now') AS INTEGER))",
                [agent_kind],
            )?;
        } else {
            self.connection.execute(
                "DELETE FROM evidence_exclusions WHERE agent_kind = ?1",
                [agent_kind],
            )?;
        }
        Ok(())
    }

    pub fn export_lifecycle(&self) -> StorageResult<serde_json::Value> {
        let evidence = query_json_rows(
            &self.connection,
            "SELECT json_object(
                'id', id, 'scan_id', scan_id, 'kind', kind, 'quality', quality,
                'subject_type', subject_type, 'subject_id', subject_id, 'path', path,
                'digest', digest, 'details', json(details_json), 'observed_at', observed_at)
             FROM evidence ORDER BY observed_at, id",
        )?;
        let usage = query_json_rows(
            &self.connection,
            "SELECT json_object(
                'evidence_id', evidence_id, 'skill_id', skill_id, 'agent_id', agent_id,
                'stage', stage, 'quality', quality, 'source_path_digest', source_path_digest,
                'observed_event_count', observed_event_count,
                'occurred_at', occurred_at, 'outcome', outcome)
             FROM usage_events ORDER BY occurred_at, evidence_id",
        )?;
        let monthly_sources = query_json_rows(
            &self.connection,
            "SELECT json_object(
                'month_start', month_start, 'skill_id', skill_id, 'agent_id', agent_id,
                'stage', stage, 'quality', quality,
                'source_path_digest', source_path_digest,
                'max_observed_event_count', max_observed_event_count,
                'first_seen_at', first_seen_at, 'last_seen_at', last_seen_at)
             FROM usage_monthly_sources
             ORDER BY month_start, skill_id, agent_id, stage, quality, source_path_digest",
        )?;
        let monthly_legacy = query_json_rows(
            &self.connection,
            "SELECT json_object(
                'month_start', month_start, 'skill_id', skill_id, 'agent_id', agent_id,
                'stage', stage, 'quality', quality, 'event_count', event_count,
                'first_seen_at', first_seen_at, 'last_seen_at', last_seen_at,
                'derivation', derivation)
             FROM usage_monthly ORDER BY month_start, skill_id, agent_id, stage, quality",
        )?;
        Ok(serde_json::json!({
            "evidence": evidence,
            "usage_events": usage,
            "usage_monthly_sources": monthly_sources,
            "usage_monthly_legacy": monthly_legacy,
        }))
    }

    pub fn purge_usage_before(&self, cutoff: i64) -> StorageResult<PurgeCounts> {
        let transaction = self.connection.unchecked_transaction()?;
        let aggregated_raw_usage_rows: u64 = transaction.query_row(
            "SELECT COUNT(*)
             FROM usage_events u JOIN evidence e ON e.id = u.evidence_id
             WHERE COALESCE(u.occurred_at, e.observed_at) < ?1",
            [cutoff],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO usage_monthly_sources
                (month_start, skill_id, agent_id, stage, quality, source_path_digest,
                 max_observed_event_count, first_seen_at, last_seen_at)
             SELECT CAST(strftime('%s', strftime('%Y-%m-01', u.retention_at, 'unixepoch')) AS INTEGER),
                    u.skill_id, u.agent_id, u.stage, u.quality, u.source_path_digest,
                    MAX(u.observed_event_count), MIN(u.retention_at), MAX(u.retention_at)
             FROM (
                SELECT u.*,
                       COALESCE(u.occurred_at, e.observed_at) AS retention_at
                FROM usage_events u JOIN evidence e ON e.id = u.evidence_id
             ) u
             WHERE u.retention_at < ?1
             GROUP BY strftime('%Y-%m', u.retention_at, 'unixepoch'),
                      u.skill_id, u.agent_id, u.stage, u.quality, u.source_path_digest
             ON CONFLICT(month_start, skill_id, agent_id, stage, quality, source_path_digest)
             DO UPDATE SET
                max_observed_event_count = MAX(
                    usage_monthly_sources.max_observed_event_count,
                    excluded.max_observed_event_count),
                first_seen_at = MIN(usage_monthly_sources.first_seen_at, excluded.first_seen_at),
                last_seen_at = MAX(usage_monthly_sources.last_seen_at, excluded.last_seen_at)",
            [cutoff],
        )?;
        transaction.execute_batch(
            "CREATE TEMP TABLE purge_usage_evidence (
                evidence_id TEXT PRIMARY KEY
             ) WITHOUT ROWID;",
        )?;
        transaction.execute(
            "INSERT INTO purge_usage_evidence (evidence_id)
             SELECT u.evidence_id
             FROM usage_events u JOIN evidence e ON e.id = u.evidence_id
             WHERE COALESCE(u.occurred_at, e.observed_at) < ?1
             UNION
             SELECT id FROM evidence
             WHERE kind = 'usage'
               AND COALESCE(json_extract(details_json, '$.last_seen_unix'), observed_at) < ?1",
            [cutoff],
        )?;
        transaction.execute(
            "DELETE FROM finding_evidence
             WHERE evidence_id IN (SELECT evidence_id FROM purge_usage_evidence)",
            [],
        )?;
        let deleted_raw_usage_rows = transaction.execute(
            "DELETE FROM usage_events
                 WHERE evidence_id IN (SELECT evidence_id FROM purge_usage_evidence)",
            [],
        )? as u64;
        let deleted_evidence_rows = transaction.execute(
            "DELETE FROM evidence
             WHERE id IN (SELECT evidence_id FROM purge_usage_evidence)",
            [],
        )? as u64;
        transaction.execute_batch("DROP TABLE purge_usage_evidence;")?;

        let deleted_payload_usage_summaries: u64 = transaction.query_row(
            "SELECT COALESCE(SUM((
                SELECT COUNT(*) FROM json_each(p.payload_json, '$.usage') AS usage
                WHERE COALESCE(
                    json_extract(usage.value, '$.last_seen_unix'), p.updated_at) < ?1
             )), 0) FROM scan_payloads p",
            [cutoff],
            |row| row.get(0),
        )?;
        transaction.execute(
            "UPDATE scan_payloads
             SET payload_json = json_set(
                    payload_json,
                    '$.usage',
                    COALESCE((
                        SELECT json_group_array(json(usage.value))
                        FROM json_each(scan_payloads.payload_json, '$.usage') AS usage
                        WHERE COALESCE(
                            json_extract(usage.value, '$.last_seen_unix'),
                            scan_payloads.updated_at) >= ?1
                    ), json('[]'))
                 ),
                 updated_at = MAX(
                    scan_payloads.updated_at + 1,
                    CAST(strftime('%s', 'now') AS INTEGER)
                 )
             WHERE EXISTS (
                SELECT 1 FROM json_each(scan_payloads.payload_json, '$.usage') AS usage
                WHERE COALESCE(
                    json_extract(usage.value, '$.last_seen_unix'),
                    scan_payloads.updated_at) < ?1
             )",
            [cutoff],
        )?;
        transaction.commit()?;
        Ok(PurgeCounts {
            aggregated_raw_usage_rows,
            deleted_raw_usage_rows,
            deleted_evidence_rows,
            deleted_payload_usage_summaries,
        })
    }

    pub fn purge_plans_and_receipts(&self) -> StorageResult<PlanReceiptPurgeCounts> {
        if self.recovery_required()? {
            return Err(StorageError::InvalidData(
                "recovery is required before Plans and Receipts can be purged".into(),
            ));
        }
        let transaction = self.connection.unchecked_transaction()?;
        let plans = table_count(&transaction, "plans")?;
        let receipts = table_count(&transaction, "receipts")?;
        transaction.execute(
            "UPDATE source_baselines
             SET trusted_by_receipt_id = NULL
             WHERE trusted_by_receipt_id IS NOT NULL",
            [],
        )?;
        transaction.execute("DELETE FROM snapshot_invalidations", [])?;
        transaction.execute("DELETE FROM receipt_operations", [])?;
        transaction.execute("DELETE FROM receipts", [])?;
        transaction.execute("DELETE FROM plan_operations", [])?;
        transaction.execute("DELETE FROM plans", [])?;
        transaction.commit()?;
        Ok(PlanReceiptPurgeCounts { plans, receipts })
    }
}

fn migration_v1(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "CREATE TABLE scans (
            id TEXT PRIMARY KEY,
            started_at INTEGER NOT NULL,
            completed_at INTEGER,
            status TEXT NOT NULL,
            coverage_notes_json TEXT NOT NULL
        );
        CREATE TABLE agents (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL
        );
        CREATE TABLE roots (
            id TEXT PRIMARY KEY,
            scan_id TEXT NOT NULL REFERENCES scans(id),
            agent_id TEXT REFERENCES agents(id),
            path TEXT NOT NULL,
            status TEXT NOT NULL,
            detail TEXT,
            UNIQUE(scan_id, path)
        );
        CREATE TABLE skills (
            id TEXT PRIMARY KEY,
            identity_key TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            description TEXT,
            declared_source TEXT,
            declared_revision TEXT,
            content_digest TEXT NOT NULL,
            digest_version INTEGER NOT NULL,
            governance_state TEXT NOT NULL,
            canonical_path TEXT
        );
        CREATE TABLE placements (
            id TEXT PRIMARY KEY,
            scan_id TEXT NOT NULL REFERENCES scans(id),
            skill_id TEXT NOT NULL REFERENCES skills(id),
            agent_id TEXT NOT NULL REFERENCES agents(id),
            root_id TEXT NOT NULL REFERENCES roots(id),
            path TEXT NOT NULL,
            kind TEXT NOT NULL,
            symlink_target TEXT,
            fingerprint TEXT NOT NULL,
            exposed INTEGER NOT NULL,
            UNIQUE(scan_id, agent_id, path)
        );
        CREATE TABLE evidence (
            id TEXT PRIMARY KEY,
            scan_id TEXT NOT NULL REFERENCES scans(id),
            kind TEXT NOT NULL,
            quality TEXT NOT NULL,
            subject_type TEXT NOT NULL,
            subject_id TEXT NOT NULL,
            path TEXT,
            digest TEXT,
            details_json TEXT NOT NULL,
            observed_at INTEGER NOT NULL
        );
        CREATE INDEX evidence_subject ON evidence(subject_type, subject_id);
        CREATE TABLE usage_events (
            evidence_id TEXT PRIMARY KEY REFERENCES evidence(id),
            skill_id TEXT NOT NULL REFERENCES skills(id),
            agent_id TEXT NOT NULL REFERENCES agents(id),
            stage TEXT NOT NULL,
            quality TEXT NOT NULL,
            occurred_at INTEGER NOT NULL,
            outcome TEXT
        );
        CREATE INDEX usage_events_skill_time ON usage_events(skill_id, occurred_at);
        CREATE TABLE roster_entries (
            agent_id TEXT NOT NULL REFERENCES agents(id),
            skill_id TEXT NOT NULL REFERENCES skills(id),
            state TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(agent_id, skill_id)
        );
        CREATE TABLE reports (
            id TEXT PRIMARY KEY,
            scan_id TEXT NOT NULL REFERENCES scans(id),
            created_at INTEGER NOT NULL,
            summary_json TEXT NOT NULL
        );
        CREATE TABLE findings (
            id TEXT PRIMARY KEY,
            report_id TEXT NOT NULL REFERENCES reports(id),
            category TEXT NOT NULL,
            severity TEXT NOT NULL,
            title TEXT NOT NULL,
            summary TEXT NOT NULL,
            details_json TEXT NOT NULL
        );
        CREATE TABLE finding_evidence (
            finding_id TEXT NOT NULL REFERENCES findings(id),
            evidence_id TEXT NOT NULL REFERENCES evidence(id),
            PRIMARY KEY(finding_id, evidence_id)
        );
        CREATE TABLE plans (
            id TEXT PRIMARY KEY,
            scan_id TEXT NOT NULL REFERENCES scans(id),
            report_id TEXT REFERENCES reports(id),
            created_at INTEGER NOT NULL,
            status TEXT NOT NULL,
            input_json TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            immutable_json TEXT NOT NULL
        );
        CREATE TABLE plan_operations (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL REFERENCES plans(id),
            position INTEGER NOT NULL,
            target_path TEXT NOT NULL,
            expected_fingerprint TEXT,
            action_json TEXT NOT NULL,
            UNIQUE(plan_id, position)
        );
        CREATE TABLE receipts (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL REFERENCES plans(id),
            reverses_receipt_id TEXT REFERENCES receipts(id),
            created_at INTEGER NOT NULL,
            completed_at INTEGER,
            status TEXT NOT NULL,
            verification_json TEXT NOT NULL
        );
        CREATE TABLE receipt_operations (
            receipt_id TEXT NOT NULL REFERENCES receipts(id),
            operation_id TEXT NOT NULL REFERENCES plan_operations(id),
            position INTEGER NOT NULL,
            status TEXT NOT NULL,
            before_state_json TEXT NOT NULL,
            after_state_json TEXT,
            error TEXT,
            PRIMARY KEY(receipt_id, operation_id)
        );
        CREATE VIRTUAL TABLE skills_fts USING fts5(
            skill_id UNINDEXED,
            name,
            description,
            triggers,
            body,
            tokenize = 'unicode61'
        );",
    )?;
    Ok(())
}

fn migration_v2(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS scan_payloads (
            scan_id TEXT PRIMARY KEY REFERENCES scans(id) ON DELETE CASCADE,
            payload_json TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn migration_v3(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "ALTER TABLE roots ADD COLUMN kind TEXT NOT NULL DEFAULT 'skills';
         ALTER TABLE roots ADD COLUMN explicit INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE placements RENAME TO placements_v2;
         CREATE TABLE placements (
            id TEXT PRIMARY KEY,
            scan_id TEXT NOT NULL REFERENCES scans(id),
            skill_id TEXT NOT NULL REFERENCES skills(id),
            agent_id TEXT REFERENCES agents(id),
            root_id TEXT NOT NULL REFERENCES roots(id),
            path TEXT NOT NULL,
            kind TEXT NOT NULL,
            symlink_target TEXT,
            fingerprint TEXT NOT NULL,
            exposed INTEGER NOT NULL,
            UNIQUE(scan_id, agent_id, path)
         );
         INSERT INTO placements
            (id, scan_id, skill_id, agent_id, root_id, path, kind, symlink_target,
             fingerprint, exposed)
         SELECT id, scan_id, skill_id, agent_id, root_id, path, kind, symlink_target,
             fingerprint, exposed FROM placements_v2;
         DROP TABLE placements_v2;",
    )?;
    Ok(())
}

fn migration_v4(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_monthly (
            month_start INTEGER NOT NULL,
            skill_id TEXT NOT NULL REFERENCES skills(id),
            agent_id TEXT NOT NULL REFERENCES agents(id),
            stage TEXT NOT NULL,
            quality TEXT NOT NULL,
            event_count INTEGER NOT NULL,
            first_seen_at INTEGER NOT NULL,
            last_seen_at INTEGER NOT NULL,
            PRIMARY KEY(month_start, skill_id, agent_id, stage, quality)
        );",
    )?;
    Ok(())
}

fn migration_v5(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "ALTER TABLE scan_payloads ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
         CREATE TABLE source_baselines (
            source TEXT NOT NULL,
            revision TEXT NOT NULL,
            entrypoint_digest TEXT NOT NULL,
            first_observed_scan_id TEXT NOT NULL REFERENCES scans(id),
            first_observed_at INTEGER NOT NULL,
            PRIMARY KEY(source, revision)
         );",
    )?;
    Ok(())
}

fn migration_v6(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "ALTER TABLE source_baselines ADD COLUMN trusted_digest TEXT;
         ALTER TABLE source_baselines ADD COLUMN trusted_by_receipt_id TEXT REFERENCES receipts(id);",
    )?;
    Ok(())
}

fn migration_v7(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "CREATE TABLE evidence_exclusions (
            agent_kind TEXT PRIMARY KEY,
            created_at INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

fn migration_v8(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "ALTER TABLE placements RENAME TO placements_v8;
         ALTER TABLE roots RENAME TO roots_v8;
         CREATE TABLE roots (
            id TEXT PRIMARY KEY, scan_id TEXT NOT NULL REFERENCES scans(id),
            agent_id TEXT REFERENCES agents(id), path TEXT NOT NULL,
            status TEXT NOT NULL, detail TEXT, kind TEXT NOT NULL DEFAULT 'skills',
            explicit INTEGER NOT NULL DEFAULT 0, UNIQUE(scan_id, agent_id, path));
         INSERT INTO roots SELECT * FROM roots_v8;
         CREATE TABLE placements (
            id TEXT PRIMARY KEY, scan_id TEXT NOT NULL REFERENCES scans(id),
            skill_id TEXT NOT NULL REFERENCES skills(id), agent_id TEXT REFERENCES agents(id),
            root_id TEXT NOT NULL REFERENCES roots(id), path TEXT NOT NULL, kind TEXT NOT NULL,
            symlink_target TEXT, fingerprint TEXT NOT NULL, exposed INTEGER NOT NULL,
            UNIQUE(scan_id, agent_id, path));
         INSERT INTO placements SELECT * FROM placements_v8;
         DROP TABLE placements_v8;
         DROP TABLE roots_v8;",
    )?;
    Ok(())
}

fn migration_v9(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "ALTER TABLE usage_monthly ADD COLUMN derivation TEXT NOT NULL
            DEFAULT 'legacy_scan_aggregate';
         DROP INDEX usage_events_skill_time;
         ALTER TABLE usage_events RENAME TO usage_events_v8;
         CREATE TABLE usage_events (
            evidence_id TEXT PRIMARY KEY REFERENCES evidence(id),
            skill_id TEXT NOT NULL REFERENCES skills(id),
            agent_id TEXT NOT NULL REFERENCES agents(id),
            stage TEXT NOT NULL,
            quality TEXT NOT NULL,
            source_path_digest TEXT NOT NULL,
            observed_event_count INTEGER NOT NULL,
            occurred_at INTEGER,
            outcome TEXT,
            UNIQUE(skill_id, agent_id, stage, quality, source_path_digest,
                   observed_event_count, occurred_at)
         );
         CREATE UNIQUE INDEX usage_events_unknown_identity
            ON usage_events(skill_id, agent_id, stage, quality, source_path_digest,
                            observed_event_count)
            WHERE occurred_at IS NULL;
         INSERT INTO usage_events
            (evidence_id, skill_id, agent_id, stage, quality, source_path_digest,
             observed_event_count, occurred_at, outcome)
         SELECT evidence_id, skill_id, agent_id, stage, quality, source_path_digest,
                observed_event_count, occurred_at, outcome
         FROM (
            SELECT u.evidence_id, u.skill_id, u.agent_id, u.stage, u.quality,
                   COALESCE(json_extract(e.details_json, '$.source_path_digest'), e.digest, '')
                       AS source_path_digest,
                   COALESCE(json_extract(e.details_json, '$.event_count'), 1)
                       AS observed_event_count,
                   u.occurred_at, u.outcome,
                   ROW_NUMBER() OVER (
                       PARTITION BY u.skill_id, u.agent_id, u.stage, u.quality,
                           COALESCE(json_extract(e.details_json, '$.source_path_digest'), e.digest, ''),
                           COALESCE(json_extract(e.details_json, '$.event_count'), 1),
                           u.occurred_at
                       ORDER BY u.evidence_id DESC
                   ) AS rank
            FROM usage_events_v8 u JOIN evidence e ON e.id = u.evidence_id
         ) WHERE rank = 1;
         DROP TABLE usage_events_v8;
         CREATE INDEX usage_events_skill_time ON usage_events(skill_id, occurred_at);
         CREATE TABLE usage_monthly_sources (
            month_start INTEGER NOT NULL,
            skill_id TEXT NOT NULL REFERENCES skills(id),
            agent_id TEXT NOT NULL REFERENCES agents(id),
            stage TEXT NOT NULL,
            quality TEXT NOT NULL,
            source_path_digest TEXT NOT NULL,
            max_observed_event_count INTEGER NOT NULL,
            first_seen_at INTEGER NOT NULL,
            last_seen_at INTEGER NOT NULL,
            PRIMARY KEY(month_start, skill_id, agent_id, stage, quality, source_path_digest)
         );",
    )?;
    Ok(())
}

fn migration_v10(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "CREATE TABLE source_root_permissions (
            id TEXT PRIMARY KEY,
            canonical_path TEXT NOT NULL,
            finding_id TEXT NOT NULL,
            snapshot_id TEXT NOT NULL,
            identity_json TEXT NOT NULL,
            granted_at INTEGER NOT NULL,
            revoked_at INTEGER
         );
         CREATE UNIQUE INDEX one_active_source_root_permission
            ON source_root_permissions(canonical_path) WHERE revoked_at IS NULL;",
    )?;
    Ok(())
}

fn migration_v11(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "CREATE TABLE snapshot_invalidations (
            snapshot_id TEXT NOT NULL REFERENCES scans(id),
            receipt_id TEXT NOT NULL REFERENCES receipts(id),
            created_at INTEGER NOT NULL,
            PRIMARY KEY(snapshot_id, receipt_id)
         );
         INSERT INTO snapshot_invalidations (snapshot_id, receipt_id, created_at)
         SELECT p.scan_id, r.id, r.created_at
         FROM receipts r JOIN plans p ON p.id = r.plan_id
         WHERE r.status = 'applied' AND r.reverses_receipt_id IS NULL;
         INSERT OR IGNORE INTO snapshot_invalidations (snapshot_id, receipt_id, created_at)
         SELECT latest.id, reverse.id, reverse.created_at
         FROM receipts reverse
         JOIN plans p ON p.id = reverse.plan_id
         JOIN scans latest ON latest.id = (
             SELECT s.id FROM scans s
             WHERE s.status = 'completed'
             ORDER BY s.completed_at DESC, s.started_at DESC, s.rowid DESC
             LIMIT 1
         )
         WHERE reverse.status = 'undone'
           AND reverse.reverses_receipt_id IS NOT NULL
           AND latest.id != p.scan_id
           AND reverse.completed_at >= latest.completed_at;",
    )?;
    Ok(())
}

fn migration_v12(transaction: &Transaction<'_>) -> StorageResult<()> {
    transaction.execute_batch(
        "ALTER TABLE reports ADD COLUMN scan_payload_updated_at INTEGER;
         UPDATE reports
         SET scan_payload_updated_at = (
             SELECT p.updated_at FROM scan_payloads p WHERE p.scan_id = reports.scan_id
         )
         WHERE EXISTS (
             SELECT 1 FROM scan_payloads p
             WHERE p.scan_id = reports.scan_id AND reports.created_at >= p.updated_at
         );
         CREATE INDEX reports_scan_payload_version
            ON reports(scan_id, scan_payload_updated_at);",
    )?;
    Ok(())
}

fn table_count(connection: &Connection, table: &str) -> StorageResult<u64> {
    let query = format!("SELECT COUNT(*) FROM {table}");
    Ok(connection.query_row(&query, [], |row| row.get(0))?)
}

fn query_json_rows(connection: &Connection, query: &str) -> StorageResult<Vec<serde_json::Value>> {
    let mut statement = connection.prepare(query)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.map(|row| {
        let encoded = row?;
        Ok(serde_json::from_str(&encoded)?)
    })
    .collect()
}

fn save_finding(transaction: &Transaction<'_>, finding: &FindingRecord) -> StorageResult<()> {
    transaction.execute(
        "INSERT INTO findings
            (id, report_id, category, severity, title, summary, details_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            finding.id.as_str(),
            finding.report_id.as_str(),
            enum_text(&finding.category)?,
            enum_text(&finding.severity)?,
            finding.title,
            finding.summary,
            json(&finding.details)?,
        ],
    )?;
    for evidence_id in &finding.evidence_ids {
        transaction.execute(
            "INSERT INTO finding_evidence (finding_id, evidence_id) VALUES (?1, ?2)",
            params![finding.id.as_str(), evidence_id.as_str()],
        )?;
    }
    Ok(())
}

fn valid_plan_transition(current: &PlanStatus, next: &PlanStatus) -> bool {
    matches!(
        (current, next),
        (PlanStatus::Ready, PlanStatus::Applying)
            | (PlanStatus::Applying, PlanStatus::Ready)
            | (PlanStatus::Applying, PlanStatus::Applied)
            | (PlanStatus::Applying, PlanStatus::FailedRolledBack)
            | (PlanStatus::Applying, PlanStatus::RecoveryRequired)
    )
}

fn save_receipt_transaction(
    transaction: &Transaction<'_>,
    receipt: &ReceiptRecord,
) -> StorageResult<()> {
    transaction.execute(
        "INSERT INTO receipts
            (id, plan_id, reverses_receipt_id, created_at, completed_at, status, verification_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET completed_at = excluded.completed_at,
            status = excluded.status, verification_json = excluded.verification_json",
        params![
            receipt.id.as_str(),
            receipt.plan_id.as_str(),
            receipt.reverses_receipt_id.as_ref().map(ReceiptId::as_str),
            receipt.created_at,
            receipt.completed_at,
            enum_text(&receipt.status)?,
            json(&receipt.verification)?,
        ],
    )?;
    for result in &receipt.operation_results {
        transaction.execute(
            "INSERT INTO receipt_operations
                (receipt_id, operation_id, position, status, before_state_json,
                 after_state_json, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(receipt_id, operation_id) DO UPDATE SET
                status = excluded.status, after_state_json = excluded.after_state_json,
                error = excluded.error",
            params![
                receipt.id.as_str(),
                result.operation_id.as_str(),
                result.position,
                result.status,
                json(&result.before_state)?,
                result.after_state.as_ref().map(json).transpose()?,
                result.error,
            ],
        )?;
    }
    Ok(())
}

fn enum_text<T: Serialize>(value: &T) -> StorageResult<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(value) => Ok(value),
        _ => Err(StorageError::InvalidData(
            "expected enum to serialize as a string".to_owned(),
        )),
    }
}

fn enum_from_text<T: DeserializeOwned>(value: &str) -> StorageResult<T> {
    Ok(serde_json::from_value(serde_json::Value::String(
        value.to_owned(),
    ))?)
}

fn json<T: Serialize>(value: &T) -> StorageResult<String> {
    Ok(serde_json::to_string(value)?)
}

fn from_json<T: DeserializeOwned>(value: &str) -> StorageResult<T> {
    Ok(serde_json::from_str(value)?)
}

fn invalid_id(error: crate::model::InvalidId) -> StorageError {
    StorageError::InvalidData(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AgentKind, EvidenceKind, EvidenceQuality, GovernanceState, OperationAction, OperationId,
        PlacementId, PlacementKind, PlacementRecord, PlanId, PlanOperation, RootId, RootStatus,
        ScanStatus, UsageStage,
    };

    fn scan() -> ScanRun {
        ScanRun {
            id: ScanId::new(),
            started_at: 10,
            completed_at: Some(20),
            status: ScanStatus::Completed,
            coverage_notes: vec!["known roots only".to_owned()],
        }
    }

    #[test]
    fn migration_creates_current_schema() {
        let store = StateStore::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn failed_migration_rolls_back_schema_and_version() {
        fn fail_after_schema_write(transaction: &Transaction<'_>) -> StorageResult<()> {
            transaction.execute_batch("CREATE TABLE partial_migration (id INTEGER)")?;
            Err(StorageError::InvalidData("injected failure".into()))
        }

        let mut store = StateStore {
            connection: Connection::open_in_memory().unwrap(),
        };
        let error = store.run_migration(1, fail_after_schema_write).unwrap_err();

        assert_eq!(error.to_string(), "injected failure");
        assert_eq!(
            store
                .connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        let retained_table: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'partial_migration'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained_table, 0);
    }

    #[test]
    fn migration_registry_must_be_complete_and_strictly_ordered() {
        let incomplete: [(i64, Migration); 1] = [(1, migration_v1)];
        assert_eq!(
            validate_migrations(&incomplete, 2).unwrap_err().to_string(),
            "migration registry has 1 entries for schema 2"
        );

        let skipped: [(i64, Migration); 2] = [(1, migration_v1), (3, migration_v2)];
        assert_eq!(
            validate_migrations(&skipped, 2).unwrap_err().to_string(),
            "migration registry expected target 2, found 3"
        );
    }

    #[test]
    fn fts_match_searches_complete_body_and_treats_operators_as_text() {
        let store = StateStore::open_in_memory().unwrap();
        let body_only = SkillId::parse("skill_body_only").unwrap();
        let other = SkillId::parse("skill_other").unwrap();
        store
            .index_skill(
                &body_only,
                "helper",
                "generic helper",
                "",
                "instructions include phosphorescent telemetry reconciliation",
            )
            .unwrap();
        store
            .index_skill(&other, "other", "unrelated", "", "different body")
            .unwrap();

        assert_eq!(
            store.search_skill_ids("phosphorescent", 5).unwrap(),
            vec![body_only.clone()]
        );
        assert_eq!(
            store
                .search_skill_ids("phosphorescent OR unrelated", 5)
                .unwrap()
                .len(),
            2
        );
        assert!(store.search_skill_ids("\" OR *", 5).unwrap().is_empty());
    }

    #[test]
    fn rebuilding_fts_with_current_empty_semantics_removes_historical_terms() {
        let store = StateStore::open_in_memory().unwrap();
        let skill = SkillId::parse("skill_historical_body").unwrap();
        store
            .index_skill(
                &skill,
                "helper",
                "historical generic metadata",
                "",
                "instructions include phosphorescent telemetry reconciliation",
            )
            .unwrap();
        assert_eq!(
            store.search_skill_ids("phosphorescent", 5).unwrap(),
            vec![skill.clone()]
        );

        store.index_skill(&skill, "helper", "", "", "").unwrap();

        assert!(
            store
                .search_skill_ids("phosphorescent", 5)
                .unwrap()
                .is_empty()
        );
        let current: (String, String) = store
            .connection
            .query_row(
                "SELECT description, body FROM skills_fts WHERE skill_id = ?1",
                [skill.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(current, (String::new(), String::new()));
    }

    #[test]
    fn search_falls_back_to_unicode_phrase_matching() {
        let store = StateStore::open_in_memory().unwrap();
        let skill = SkillId::parse("skill_database").unwrap();
        store
            .index_skill(
                &skill,
                "dms-mysql",
                "MySQL 数据库管理和表结构查询",
                "",
                "执行查询",
            )
            .unwrap();

        assert_eq!(
            store.search_skill_ids("数据库管理", 5).unwrap(),
            vec![skill]
        );
    }

    #[test]
    fn scan_snapshot_savepoint_rolls_back_partial_graph_writes() {
        let store = StateStore::open_in_memory().unwrap();
        store.begin_scan_snapshot().unwrap();
        store
            .save_scan(&ScanRun {
                id: ScanId::parse("scan_partial").unwrap(),
                started_at: 1,
                completed_at: Some(2),
                status: ScanStatus::Completed,
                coverage_notes: vec![],
            })
            .unwrap();
        store.rollback_scan_snapshot().unwrap();
        let count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM scans WHERE id = 'scan_partial'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn migration_upgrades_prior_states_sequentially() {
        for starting_version in [1_i64, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] {
            let mut connection = Connection::open_in_memory().unwrap();
            let transaction = connection.transaction().unwrap();
            migration_v1(&transaction).unwrap();
            if starting_version >= 2 {
                migration_v2(&transaction).unwrap();
            }
            if starting_version >= 3 {
                migration_v3(&transaction).unwrap();
            }
            if starting_version >= 4 {
                migration_v4(&transaction).unwrap();
            }
            if starting_version >= 5 {
                migration_v5(&transaction).unwrap();
            }
            if starting_version >= 6 {
                migration_v6(&transaction).unwrap();
            }
            if starting_version >= 7 {
                migration_v7(&transaction).unwrap();
            }
            if starting_version >= 8 {
                migration_v8(&transaction).unwrap();
            }
            if starting_version >= 9 {
                migration_v9(&transaction).unwrap();
            }
            if starting_version >= 10 {
                migration_v10(&transaction).unwrap();
            }
            if starting_version >= 11 {
                migration_v11(&transaction).unwrap();
            }
            transaction
                .pragma_update(None, "user_version", starting_version)
                .unwrap();
            transaction.commit().unwrap();

            let store = StateStore::initialize(connection).unwrap();
            assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
            assert_eq!(store.lifecycle_counts().unwrap().monthly_usage_rows, 0);
        }
    }

    #[test]
    fn migration_requirement_detects_missing_old_and_current_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("skillroster.db");
        assert!(StateStore::requires_migration(&path).unwrap());

        let mut connection = Connection::open(&path).unwrap();
        let transaction = connection.transaction().unwrap();
        for migration in [
            migration_v1 as Migration,
            migration_v2,
            migration_v3,
            migration_v4,
            migration_v5,
            migration_v6,
            migration_v7,
            migration_v8,
            migration_v9,
            migration_v10,
        ] {
            migration(&transaction).unwrap();
        }
        transaction.pragma_update(None, "user_version", 10).unwrap();
        transaction.commit().unwrap();
        drop(connection);
        assert!(StateStore::requires_migration(&path).unwrap());

        drop(StateStore::open(&path).unwrap());
        assert!(!StateStore::requires_migration(&path).unwrap());
    }

    #[test]
    fn snapshot_invalidation_migration_backfills_applied_and_later_undo_receipts() {
        let mut connection = Connection::open_in_memory().unwrap();
        let transaction = connection.transaction().unwrap();
        for (_, migration) in &MIGRATIONS[..10] {
            migration(&transaction).unwrap();
        }
        transaction
            .execute_batch(
                "INSERT INTO scans VALUES
                    ('scan_a', 1, 10, 'completed', '[]'),
                    ('scan_b', 11, 20, 'completed', '[]');
                 INSERT INTO plans VALUES
                    ('plan_a', 'scan_a', NULL, 2, 'applied', '{}', 'a', '{}'),
                    ('plan_b', 'scan_b', NULL, 12, 'applied', '{}', 'b', '{}');
                 INSERT INTO receipts VALUES
                    ('receipt_original', 'plan_a', NULL, 3, 4, 'undone', '{}'),
                    ('receipt_reverse', 'plan_a', 'receipt_original', 21, 22, 'undone', '{}'),
                    ('receipt_applied', 'plan_b', NULL, 13, 14, 'applied', '{}');",
            )
            .unwrap();

        migration_v11(&transaction).unwrap();

        let rows = transaction
            .prepare(
                "SELECT snapshot_id, receipt_id FROM snapshot_invalidations
                 ORDER BY receipt_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                ("scan_b".to_owned(), "receipt_applied".to_owned()),
                ("scan_b".to_owned(), "receipt_reverse".to_owned()),
            ]
        );
    }

    #[test]
    fn snapshot_invalidation_migration_fails_closed_for_ambiguous_same_second_order() {
        let mut connection = Connection::open_in_memory().unwrap();
        let transaction = connection.transaction().unwrap();
        for (_, migration) in &MIGRATIONS[..10] {
            migration(&transaction).unwrap();
        }
        transaction
            .execute_batch(
                "INSERT INTO scans VALUES
                    ('scan_before', 1, 10, 'completed', '[]'),
                    ('scan_after', 20, 20, 'completed', '[]');
                 INSERT INTO plans VALUES
                    ('plan_a', 'scan_before', NULL, 2, 'applied', '{}', 'a', '{}');
                 INSERT INTO receipts VALUES
                    ('receipt_original', 'plan_a', NULL, 3, 4, 'undone', '{}'),
                    ('receipt_reverse', 'plan_a', 'receipt_original', 19, 20, 'undone', '{}');",
            )
            .unwrap();

        migration_v11(&transaction).unwrap();

        let reverse_invalidations: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM snapshot_invalidations
                 WHERE receipt_id = 'receipt_reverse'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reverse_invalidations, 1);
    }

    #[test]
    fn report_payload_version_migration_backfills_only_previously_fresh_reports() {
        let mut connection = Connection::open_in_memory().unwrap();
        let transaction = connection.transaction().unwrap();
        for (_, migration) in &MIGRATIONS[..11] {
            migration(&transaction).unwrap();
        }
        transaction
            .execute_batch(
                "INSERT INTO scans VALUES ('scan_a', 1, 2, 'completed', '[]');
                 INSERT INTO scan_payloads VALUES ('scan_a', '{}', 100);
                 INSERT INTO reports VALUES
                    ('report_stale', 'scan_a', 99, '{}'),
                    ('report_fresh', 'scan_a', 100, '{}');",
            )
            .unwrap();

        migration_v12(&transaction).unwrap();

        let bound = transaction
            .prepare("SELECT id FROM reports WHERE scan_payload_updated_at IS NOT NULL ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(bound, vec!["report_fresh"]);
        let indexed: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name = 'reports_scan_payload_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexed, 1);
    }

    #[test]
    fn migration_preserves_distinct_v8_observations_without_combining_legacy_totals() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        let transaction = connection.transaction().unwrap();
        migration_v1(&transaction).unwrap();
        migration_v2(&transaction).unwrap();
        migration_v3(&transaction).unwrap();
        migration_v4(&transaction).unwrap();
        migration_v5(&transaction).unwrap();
        migration_v6(&transaction).unwrap();
        migration_v7(&transaction).unwrap();
        migration_v8(&transaction).unwrap();
        transaction
            .execute(
                "INSERT INTO scans VALUES ('scan_v8', 1, 2, 'completed', '[]')",
                [],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO agents VALUES ('agent_v8', 'codex', 'Codex')",
                [],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO skills
                    (id, identity_key, name, content_digest, digest_version, governance_state)
                 VALUES ('skill_v8', 'content:v8', 'v8', 'sha256:v8', 1, 'observed')",
                [],
            )
            .unwrap();
        for (evidence_id, event_count, observed_at) in
            [("evidence_v8_a", 1_i64, 10_i64), ("evidence_v8_b", 2, 20)]
        {
            transaction
                .execute(
                    "INSERT INTO evidence
                        (id, scan_id, kind, quality, subject_type, subject_id, digest,
                         details_json, observed_at)
                     VALUES (?1, 'scan_v8', 'usage', 'observed', 'skill', 'skill_v8',
                             'sha256:source', ?2, ?3)",
                    params![
                        evidence_id,
                        serde_json::json!({
                            "source_path_digest": "sha256:source",
                            "event_count": event_count,
                            "first_seen_unix": 10,
                            "last_seen_unix": observed_at,
                        })
                        .to_string(),
                        observed_at,
                    ],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO usage_events
                        (evidence_id, skill_id, agent_id, stage, quality, occurred_at)
                     VALUES (?1, 'skill_v8', 'agent_v8', 'loaded', 'observed', ?2)",
                    params![evidence_id, observed_at],
                )
                .unwrap();
        }
        transaction
            .execute(
                "INSERT INTO usage_monthly
                    (month_start, skill_id, agent_id, stage, quality, event_count,
                     first_seen_at, last_seen_at)
                 VALUES (1000, 'skill_v8', 'agent_v8', 'loaded', 'observed', 7, 1, 2)",
                [],
            )
            .unwrap();
        transaction
            .pragma_update(None, "user_version", 8_i64)
            .unwrap();
        transaction.commit().unwrap();

        let store = StateStore::initialize(connection).unwrap();
        let export = store.export_lifecycle().unwrap();
        assert_eq!(export["usage_events"].as_array().unwrap().len(), 2);
        assert_eq!(export["usage_events"][0]["observed_event_count"], 1);
        assert_eq!(export["usage_events"][1]["observed_event_count"], 2);
        assert_eq!(
            export["usage_monthly_legacy"][0]["derivation"],
            "legacy_scan_aggregate"
        );
        assert_eq!(
            store
                .purge_usage_before(100)
                .unwrap()
                .aggregated_raw_usage_rows,
            2
        );
        let retained = store.export_lifecycle().unwrap();
        assert_eq!(
            retained["usage_monthly_sources"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            retained["usage_monthly_sources"][0]["max_observed_event_count"],
            2
        );
        assert_eq!(retained["usage_monthly_legacy"][0]["event_count"], 7);
    }

    #[test]
    fn logical_skill_identity_keeps_its_first_id() {
        let store = StateStore::open_in_memory().unwrap();
        let mut skill = SkillRecord {
            id: SkillId::new(),
            identity_key: "source@example#v1".to_owned(),
            name: "Example".to_owned(),
            description: None,
            declared_source: Some("source@example".to_owned()),
            declared_revision: Some("v1".to_owned()),
            content_digest: "sha256:one".to_owned(),
            digest_version: 1,
            governance_state: GovernanceState::Observed,
            canonical_path: None,
        };
        let stable = store.save_skill(&skill).unwrap();
        store
            .update_skill_governance_state(&stable, GovernanceState::Managed)
            .unwrap();
        skill.id = SkillId::new();
        skill.content_digest = "sha256:two".to_owned();
        assert_eq!(store.save_skill(&skill).unwrap(), stable);
        assert_eq!(
            store.skill_governance_state(&stable).unwrap(),
            Some(GovernanceState::Managed)
        );

        let conflicting = SkillRecord {
            id: stable,
            identity_key: "content:different-strong-identity".to_owned(),
            ..skill
        };
        assert!(store.save_skill(&conflicting).is_err());
    }

    #[test]
    fn source_baseline_is_first_observed_and_immutable() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let initial = SourceBaseline {
            source: "github:example/source-skill".to_owned(),
            revision: "v1".to_owned(),
            entrypoint_digest: "a".repeat(64),
            first_observed_scan_id: scan.id.clone(),
            first_observed_at: 20,
            trusted_digest: None,
            trusted_by_receipt_id: None,
        };
        store.record_source_baseline(&initial).unwrap();
        store
            .record_source_baseline(&SourceBaseline {
                entrypoint_digest: "b".repeat(64),
                first_observed_at: 30,
                ..initial.clone()
            })
            .unwrap();

        assert_eq!(
            store
                .source_baseline(&initial.source, &initial.revision)
                .unwrap(),
            Some(initial)
        );
    }

    #[test]
    fn plan_is_immutable_and_status_transition_is_checked() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let operation = PlanOperation {
            id: OperationId::new(),
            position: 0,
            target_path: "/tmp/example".to_owned(),
            expected_fingerprint: None,
            action: OperationAction::CreateDirectory,
        };
        let mut plan = PlanRecord {
            id: PlanId::new(),
            scan_id: scan.id,
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![operation],
        };
        store.save_plan(&plan).unwrap();
        plan.fingerprint = "changed".to_owned();
        assert!(store.save_plan(&plan).is_err());
        assert!(
            store
                .update_plan_status(&plan.id, PlanStatus::Applied)
                .is_err()
        );
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        assert_eq!(
            store.get_plan(&plan.id).unwrap().unwrap().status,
            PlanStatus::Applying
        );
        store
            .update_plan_status(&plan.id, PlanStatus::Ready)
            .unwrap();
        assert_eq!(
            store.get_plan(&plan.id).unwrap().unwrap().status,
            PlanStatus::Ready
        );
    }

    #[test]
    fn pending_plans_are_current_bounded_and_lifecycle_prioritized() {
        let store = StateStore::open_in_memory().unwrap();
        let old_scan = scan();
        let mut current_scan = scan();
        current_scan.started_at = 30;
        current_scan.completed_at = Some(40);
        store.save_scan(&old_scan).unwrap();
        store.save_scan(&current_scan).unwrap();
        let save_plan = |scan_id: ScanId, status: PlanStatus, created_at: i64| {
            let plan = PlanRecord {
                id: PlanId::new(),
                scan_id,
                report_id: None,
                created_at,
                status,
                input: serde_json::json!({}),
                fingerprint: format!("plan-{created_at}"),
                operations: Vec::new(),
            };
            store.save_plan(&plan).unwrap();
            plan.id
        };
        let stale_ready = save_plan(old_scan.id.clone(), PlanStatus::Ready, 1);
        let applying = save_plan(old_scan.id.clone(), PlanStatus::Applying, 2);
        let recovery = save_plan(old_scan.id, PlanStatus::RecoveryRequired, 3);
        for created_at in 10..35 {
            save_plan(current_scan.id.clone(), PlanStatus::Ready, created_at);
        }

        let (count, plans) = store.pending_plans(Some(&current_scan.id), 20).unwrap();

        assert_eq!(count, 27);
        assert_eq!(plans.len(), 20);
        assert_eq!(plans[0].id, recovery);
        assert_eq!(plans[0].status, PlanStatus::RecoveryRequired);
        assert_eq!(plans[1].id, applying);
        assert_eq!(plans[1].status, PlanStatus::Applying);
        assert!(plans.iter().all(|plan| plan.id != stale_ready));
    }

    #[test]
    fn undo_receipt_and_original_status_commit_atomically() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let plan = PlanRecord {
            id: PlanId::new(),
            scan_id: scan.id,
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();
        let original = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: None,
            created_at: 31,
            completed_at: Some(32),
            status: ReceiptStatus::Applied,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        store
            .save_apply_receipt(&plan.id, PlanStatus::Applied, &original)
            .unwrap();
        let reverse = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id,
            reverses_receipt_id: Some(original.id.clone()),
            created_at: 33,
            completed_at: Some(34),
            status: ReceiptStatus::Undone,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };
        store
            .save_undo_receipt(&original.id, &reverse, None)
            .unwrap();
        assert_eq!(
            store.get_receipt(&original.id).unwrap().unwrap().status,
            ReceiptStatus::Undone
        );
        assert_eq!(
            store.reverse_receipt_for(&original.id).unwrap(),
            Some(reverse.id.clone())
        );

        let duplicate = ReceiptRecord {
            id: ReceiptId::new(),
            ..reverse
        };
        assert!(
            store
                .save_undo_receipt(&original.id, &duplicate, None)
                .is_err()
        );
        assert!(!store.receipt_exists(&duplicate.id).unwrap());
    }

    #[test]
    fn applied_and_undo_receipts_invalidate_only_the_snapshot_they_changed() {
        let store = StateStore::open_in_memory().unwrap();
        let original_scan = scan();
        store.save_scan(&original_scan).unwrap();
        let plan = PlanRecord {
            id: PlanId::new(),
            scan_id: original_scan.id.clone(),
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();
        let original = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: None,
            created_at: 31,
            completed_at: Some(32),
            status: ReceiptStatus::Applied,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };

        assert!(
            store
                .snapshot_invalidating_receipt(&original_scan.id)
                .unwrap()
                .is_none()
        );
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        store
            .save_apply_receipt(&plan.id, PlanStatus::Applied, &original)
            .unwrap();
        let applied_invalidation = store
            .snapshot_invalidating_receipt(&original_scan.id)
            .unwrap()
            .unwrap();
        assert_eq!(applied_invalidation.receipt_id, original.id.clone());
        assert_eq!(
            applied_invalidation.kind,
            SnapshotInvalidationKind::AppliedReceipt
        );
        let newer_scan = scan();
        store.save_scan(&newer_scan).unwrap();
        assert!(
            store
                .snapshot_invalidating_receipt(&newer_scan.id)
                .unwrap()
                .is_none()
        );

        let reverse = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id,
            reverses_receipt_id: Some(original.id.clone()),
            created_at: 33,
            completed_at: Some(34),
            status: ReceiptStatus::Undone,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };
        store
            .save_undo_receipt(&original.id, &reverse, Some(&newer_scan.id))
            .unwrap();
        assert!(
            store
                .snapshot_invalidating_receipt(&original_scan.id)
                .unwrap()
                .is_none()
        );
        let undo_invalidation = store
            .snapshot_invalidating_receipt(&newer_scan.id)
            .unwrap()
            .unwrap();
        assert_eq!(undo_invalidation.receipt_id, reverse.id);
        assert_eq!(
            undo_invalidation.kind,
            SnapshotInvalidationKind::UndoReceipt
        );
        let post_undo_scan = scan();
        store.save_scan(&post_undo_scan).unwrap();
        assert!(
            store
                .snapshot_invalidating_receipt(&post_undo_scan.id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn failed_apply_finalization_keeps_sqlite_atomic_for_journal_recovery() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let plan = PlanRecord {
            id: PlanId::new(),
            scan_id: scan.id,
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_receipt_insert BEFORE INSERT ON receipts
                 BEGIN SELECT RAISE(FAIL, 'injected receipt failure'); END;",
            )
            .unwrap();
        let receipt = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: None,
            created_at: 31,
            completed_at: Some(32),
            status: ReceiptStatus::Applied,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };
        assert!(
            store
                .save_apply_receipt(&plan.id, PlanStatus::Applied, &receipt)
                .is_err()
        );
        assert!(!store.receipt_exists(&receipt.id).unwrap());
        assert_eq!(
            store.get_plan(&plan.id).unwrap().unwrap().status,
            PlanStatus::Applying
        );
    }

    #[test]
    fn recovery_receipt_atomically_converges_new_and_legacy_split_state() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let plan = PlanRecord {
            id: PlanId::new(),
            scan_id: scan.id,
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        let receipt = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: None,
            created_at: 31,
            completed_at: Some(32),
            status: ReceiptStatus::RecoveryRequired,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };

        store.save_receipt(&receipt).unwrap();
        assert_eq!(
            store.save_recovery_receipt(&plan.id, &receipt).unwrap(),
            RecoveryReceiptSaveOutcome {
                inserted: false,
                plan_transitioned: true,
            }
        );
        assert_eq!(
            store.get_plan(&plan.id).unwrap().unwrap().status,
            PlanStatus::RecoveryRequired
        );
    }

    #[test]
    fn failed_recovery_finalization_retains_neither_receipt_nor_plan_transition() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let plan = PlanRecord {
            id: PlanId::new(),
            scan_id: scan.id,
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_recovery_transition BEFORE UPDATE OF status ON plans
                 WHEN NEW.status = 'recovery_required'
                 BEGIN SELECT RAISE(FAIL, 'injected recovery failure'); END;",
            )
            .unwrap();
        let receipt = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: None,
            created_at: 31,
            completed_at: Some(32),
            status: ReceiptStatus::RecoveryRequired,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };

        assert!(store.save_recovery_receipt(&plan.id, &receipt).is_err());
        assert!(!store.receipt_exists(&receipt.id).unwrap());
        assert_eq!(
            store.get_plan(&plan.id).unwrap().unwrap().status,
            PlanStatus::Applying
        );
    }

    #[test]
    fn reverse_recovery_receipt_preserves_applied_plan_and_is_idempotent() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let plan = PlanRecord {
            id: PlanId::new(),
            scan_id: scan.id,
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        let original = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: None,
            created_at: 31,
            completed_at: Some(32),
            status: ReceiptStatus::Applied,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };
        store
            .save_apply_receipt(&plan.id, PlanStatus::Applied, &original)
            .unwrap();
        let recovery = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: Some(original.id.clone()),
            created_at: 33,
            completed_at: Some(34),
            status: ReceiptStatus::RecoveryRequired,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };

        assert_eq!(
            store.save_recovery_receipt(&plan.id, &recovery).unwrap(),
            RecoveryReceiptSaveOutcome {
                inserted: true,
                plan_transitioned: false,
            }
        );
        assert_eq!(
            store.get_plan(&plan.id).unwrap().unwrap().status,
            PlanStatus::Applied
        );
        assert_eq!(
            store.save_recovery_receipt(&plan.id, &recovery).unwrap(),
            RecoveryReceiptSaveOutcome {
                inserted: false,
                plan_transitioned: false,
            }
        );
    }

    #[test]
    fn reverse_recovery_receipt_rejects_a_rolled_back_plan() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let plan = PlanRecord {
            id: PlanId::new(),
            scan_id: scan.id,
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        store
            .update_plan_status(&plan.id, PlanStatus::FailedRolledBack)
            .unwrap();
        let original = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: None,
            created_at: 31,
            completed_at: Some(32),
            status: ReceiptStatus::Applied,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };
        store.save_receipt(&original).unwrap();
        let recovery = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: Some(original.id),
            created_at: 33,
            completed_at: Some(34),
            status: ReceiptStatus::RecoveryRequired,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };

        assert!(store.save_recovery_receipt(&plan.id, &recovery).is_err());
        assert!(!store.receipt_exists(&recovery.id).unwrap());
        assert_eq!(
            store.get_plan(&plan.id).unwrap().unwrap().status,
            PlanStatus::FailedRolledBack
        );
    }

    #[test]
    fn recovery_receipt_rejects_a_changed_reverse_identity() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let plan = PlanRecord {
            id: PlanId::new(),
            scan_id: scan.id,
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![],
        };
        store.save_plan(&plan).unwrap();
        store
            .update_plan_status(&plan.id, PlanStatus::Applying)
            .unwrap();
        let original = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: None,
            created_at: 31,
            completed_at: Some(32),
            status: ReceiptStatus::Applied,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };
        store.save_receipt(&original).unwrap();
        let retained = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id.clone(),
            reverses_receipt_id: None,
            created_at: 33,
            completed_at: Some(34),
            status: ReceiptStatus::RecoveryRequired,
            verification: serde_json::json!({}),
            operation_results: vec![],
        };
        store.save_receipt(&retained).unwrap();
        let conflicting = ReceiptRecord {
            reverses_receipt_id: Some(original.id),
            ..retained
        };

        assert!(store.save_recovery_receipt(&plan.id, &conflicting).is_err());
        assert_eq!(
            store
                .get_receipt(&conflicting.id)
                .unwrap()
                .unwrap()
                .reverses_receipt_id,
            None
        );
        assert_eq!(
            store.get_plan(&plan.id).unwrap().unwrap().status,
            PlanStatus::Applying
        );
    }

    #[test]
    fn receipt_operation_result_round_trips_action_target_and_fingerprints() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let operation = PlanOperation {
            id: OperationId::new(),
            position: 0,
            target_path: "/approved/skill".to_owned(),
            expected_fingerprint: Some("missing".to_owned()),
            action: OperationAction::CreateDirectory,
        };
        let plan = PlanRecord {
            id: PlanId::new(),
            scan_id: scan.id,
            report_id: None,
            created_at: 30,
            status: PlanStatus::Ready,
            input: serde_json::json!({}),
            fingerprint: "plan-sha256".to_owned(),
            operations: vec![operation.clone()],
        };
        store.save_plan(&plan).unwrap();
        let receipt = ReceiptRecord {
            id: ReceiptId::new(),
            plan_id: plan.id,
            reverses_receipt_id: None,
            created_at: 31,
            completed_at: Some(32),
            status: ReceiptStatus::Applied,
            verification: serde_json::json!({}),
            operation_results: vec![crate::model::OperationResult {
                operation_id: operation.id,
                position: 0,
                status: "applied".to_owned(),
                before_state: serde_json::json!({
                    "action": "create_directory",
                    "target": "/approved/skill",
                    "fingerprint": "missing"
                }),
                after_state: Some(serde_json::json!({
                    "target": "/approved/skill",
                    "fingerprint": "directory:sha256:abc"
                })),
                error: None,
            }],
        };
        store.save_receipt(&receipt).unwrap();
        let stored = store.get_receipt(&receipt.id).unwrap().unwrap();
        assert_eq!(stored.operation_results.len(), 1);
        assert_eq!(
            stored.operation_results[0].before_state["action"],
            "create_directory"
        );
        assert_eq!(
            stored.operation_results[0].after_state.as_ref().unwrap()["fingerprint"],
            "directory:sha256:abc"
        );
    }

    #[test]
    fn scan_graph_round_trips_latest_scan() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        let agent = AgentRecord {
            id: AgentId::new(),
            kind: AgentKind::Codex,
            display_name: "Codex".to_owned(),
        };
        store.save_agent(&agent).unwrap();
        let root = RootRecord {
            id: RootId::new(),
            scan_id: scan.id.clone(),
            agent_id: Some(agent.id.clone()),
            path: "/tmp/skills".to_owned(),
            kind: "skills".to_owned(),
            status: RootStatus::Included,
            explicit: false,
            detail: None,
        };
        store.save_root(&root).unwrap();
        let skill = SkillRecord {
            id: SkillId::new(),
            identity_key: "sha256:abc".to_owned(),
            name: "example".to_owned(),
            description: None,
            declared_source: None,
            declared_revision: None,
            content_digest: "sha256:abc".to_owned(),
            digest_version: 1,
            governance_state: GovernanceState::Observed,
            canonical_path: None,
        };
        store.save_skill(&skill).unwrap();
        store
            .save_placement(&PlacementRecord {
                id: PlacementId::new(),
                scan_id: scan.id.clone(),
                skill_id: skill.id,
                agent_id: Some(agent.id),
                root_id: root.id,
                path: "/tmp/skills/example".to_owned(),
                kind: PlacementKind::Directory,
                symlink_target: None,
                fingerprint: "fingerprint".to_owned(),
                exposed: true,
            })
            .unwrap();
        assert_eq!(store.latest_completed_scan().unwrap().unwrap().id, scan.id);
    }

    #[test]
    fn stable_placement_moves_to_the_latest_scan_graph() {
        let store = StateStore::open_in_memory().unwrap();
        let agent = AgentRecord {
            id: AgentId::new(),
            kind: AgentKind::Codex,
            display_name: "Codex".to_owned(),
        };
        store.save_agent(&agent).unwrap();
        let placement_id = PlacementId::new();

        let first_scan = scan();
        store.save_scan(&first_scan).unwrap();
        let first_root = RootRecord {
            id: RootId::new(),
            scan_id: first_scan.id.clone(),
            agent_id: Some(agent.id.clone()),
            path: "/tmp/skills-a".to_owned(),
            kind: "skills".to_owned(),
            status: RootStatus::Included,
            explicit: false,
            detail: None,
        };
        store.save_root(&first_root).unwrap();
        let first_skill_record = SkillRecord {
            id: SkillId::new(),
            identity_key: "sha256:first".to_owned(),
            name: "example".to_owned(),
            description: None,
            declared_source: None,
            declared_revision: None,
            content_digest: "sha256:first".to_owned(),
            digest_version: 1,
            governance_state: GovernanceState::Observed,
            canonical_path: None,
        };
        let first_skill = store.save_skill(&first_skill_record).unwrap();
        let first_placement = PlacementRecord {
            id: placement_id.clone(),
            scan_id: first_scan.id,
            skill_id: first_skill,
            agent_id: Some(agent.id.clone()),
            root_id: first_root.id.clone(),
            path: "/tmp/skills-a/example".to_owned(),
            kind: PlacementKind::Directory,
            symlink_target: None,
            fingerprint: "first".to_owned(),
            exposed: false,
        };
        store.save_placement(&first_placement).unwrap();

        let second_scan = scan();
        store.save_scan(&second_scan).unwrap();
        let second_root = RootRecord {
            id: RootId::new(),
            scan_id: second_scan.id.clone(),
            path: "/tmp/skills-b".to_owned(),
            explicit: true,
            ..first_root
        };
        store.save_root(&second_root).unwrap();
        let second_skill = store
            .save_skill(&SkillRecord {
                id: SkillId::new(),
                identity_key: "sha256:second".to_owned(),
                content_digest: "sha256:second".to_owned(),
                ..first_skill_record
            })
            .unwrap();
        store
            .save_placement(&PlacementRecord {
                scan_id: second_scan.id.clone(),
                skill_id: second_skill.clone(),
                root_id: second_root.id.clone(),
                path: "/tmp/skills-b/example".to_owned(),
                kind: PlacementKind::Symlink,
                symlink_target: Some("/tmp/source/example".to_owned()),
                fingerprint: "second".to_owned(),
                exposed: true,
                ..first_placement
            })
            .unwrap();

        let stored = store
            .connection
            .query_row(
                "SELECT scan_id, skill_id, agent_id, root_id, path, kind,
                        symlink_target, fingerprint, exposed
                 FROM placements WHERE id = ?1",
                [placement_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, bool>(8)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(stored.0, second_scan.id.as_str());
        assert_eq!(stored.1, second_skill.as_str());
        assert_eq!(stored.2.as_deref(), Some(agent.id.as_str()));
        assert_eq!(stored.3, second_root.id.as_str());
        assert_eq!(stored.4, "/tmp/skills-b/example");
        assert_eq!(stored.5, "symlink");
        assert_eq!(stored.6.as_deref(), Some("/tmp/source/example"));
        assert_eq!(stored.7, "second");
        assert!(stored.8);
    }

    #[test]
    fn latest_scan_uses_insertion_order_when_timestamps_tie() {
        let store = StateStore::open_in_memory().unwrap();
        let first = ScanRun {
            id: ScanId::parse("scan_ffffffffffffffffffffffffffffffff").unwrap(),
            started_at: 10,
            completed_at: Some(20),
            status: ScanStatus::Completed,
            coverage_notes: vec![],
        };
        let second = ScanRun {
            id: ScanId::parse("scan_00000000000000000000000000000000").unwrap(),
            ..first.clone()
        };
        store.save_scan(&first).unwrap();
        store
            .save_scan_payload(&first.id, &serde_json::json!({"order": 1}))
            .unwrap();
        store.save_scan(&second).unwrap();
        store
            .save_scan_payload(&second.id, &serde_json::json!({"order": 2}))
            .unwrap();

        assert_eq!(
            store.latest_completed_scan().unwrap().unwrap().id,
            second.id
        );
        let (latest_id, payload): (ScanId, serde_json::Value) =
            store.latest_scan_payload().unwrap().unwrap();
        assert_eq!(latest_id, second.id);
        assert_eq!(payload["order"], 2);
    }

    #[test]
    fn latest_report_uses_insertion_order_when_timestamps_tie() {
        let store = StateStore::open_in_memory().unwrap();
        let created_at = chrono::Utc::now().timestamp();
        let first_scan = ScanRun {
            id: ScanId::parse("scan_ffffffffffffffffffffffffffffffff").unwrap(),
            started_at: created_at,
            completed_at: Some(created_at),
            status: ScanStatus::Completed,
            coverage_notes: vec![],
        };
        let second_scan = ScanRun {
            id: ScanId::parse("scan_00000000000000000000000000000000").unwrap(),
            ..first_scan.clone()
        };
        store.save_scan(&first_scan).unwrap();
        store
            .save_scan_payload(&first_scan.id, &serde_json::json!({"order": 1}))
            .unwrap();
        store
            .save_report(
                &ReportRecord {
                    id: ReportId::parse("report_ffffffffffffffffffffffffffffffff").unwrap(),
                    scan_id: first_scan.id,
                    created_at,
                    summary: serde_json::json!({"order": 1}),
                },
                &[],
            )
            .unwrap();
        store.save_scan(&second_scan).unwrap();
        store
            .save_scan_payload(&second_scan.id, &serde_json::json!({"order": 2}))
            .unwrap();
        let second_report = ReportRecord {
            id: ReportId::parse("report_00000000000000000000000000000000").unwrap(),
            scan_id: second_scan.id,
            created_at,
            summary: serde_json::json!({"order": 2}),
        };
        store.save_report(&second_report, &[]).unwrap();
        store
            .connection
            .execute(
                "UPDATE scan_payloads SET updated_at = ?1",
                rusqlite::params![created_at],
            )
            .unwrap();

        let latest = store.latest_report().unwrap().unwrap();
        assert_eq!(latest.id, second_report.id);
        assert_eq!(latest.summary["order"], 2);
    }

    #[test]
    fn report_freshness_binds_the_exact_payload_version_without_timestamp_ordering() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        store
            .save_scan_payload(&scan.id, &serde_json::json!({"version": 1}))
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE scan_payloads SET updated_at = 10000000000 WHERE scan_id = ?1",
                [scan.id.as_str()],
            )
            .unwrap();
        store
            .save_scan_payload(&scan.id, &serde_json::json!({"version": 1}))
            .unwrap();
        let first = ReportRecord {
            id: ReportId::parse("report_00000000000000000000000000000001").unwrap(),
            scan_id: scan.id.clone(),
            created_at: 100,
            summary: serde_json::json!({"version": 1}),
        };
        store.save_report(&first, &[]).unwrap();
        assert!(store.report_exists_for_scan(&scan.id).unwrap());

        let first_version: i64 = store
            .connection
            .query_row(
                "SELECT updated_at FROM scan_payloads WHERE scan_id = ?1",
                [scan.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        store
            .save_scan_payload(&scan.id, &serde_json::json!({"version": 2}))
            .unwrap();
        let second_version: i64 = store
            .connection
            .query_row(
                "SELECT updated_at FROM scan_payloads WHERE scan_id = ?1",
                [scan.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(second_version, first_version + 1);
        assert!(!store.report_exists_for_scan(&scan.id).unwrap());
        assert!(store.latest_report().unwrap().is_none());

        let second = ReportRecord {
            id: ReportId::parse("report_00000000000000000000000000000002").unwrap(),
            created_at: 100,
            summary: serde_json::json!({"version": 2}),
            ..first
        };
        store.save_report(&second, &[]).unwrap();
        assert!(store.report_exists_for_scan(&scan.id).unwrap());
        assert_eq!(store.latest_report().unwrap().unwrap().id, second.id);
    }

    #[test]
    fn report_freshness_query_uses_the_payload_version_index() {
        let store = StateStore::open_in_memory().unwrap();
        let details = store
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {REPORT_EXISTS_FOR_SCAN_SQL}"))
            .unwrap()
            .query_map(["scan_any"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("reports_scan_payload_version")),
            "query plan did not use the Report freshness index: {details:?}"
        );
    }

    #[test]
    fn purge_aggregates_old_usage_and_preserves_lifecycle_history() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        store
            .save_scan_payload(
                &scan.id,
                &serde_json::json!({
                    "usage": [{
                        "agent": "codex",
                        "skill_id": "skill_fixture",
                        "stage": "loaded",
                        "quality": "observed",
                        "event_count": 3,
                        "first_seen_unix": 10,
                        "last_seen_unix": 10,
                        "source_path_digest": "private"
                    }],
                    "coverage": [{"agent": "codex", "denominator_reliable": false}]
                }),
            )
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE scan_payloads SET updated_at = 10000000000 WHERE scan_id = ?1",
                [scan.id.as_str()],
            )
            .unwrap();
        let agent = AgentRecord {
            id: AgentId::new(),
            kind: AgentKind::Codex,
            display_name: "Codex".to_owned(),
        };
        let agent_id = store.save_agent(&agent).unwrap();
        let skill = SkillRecord {
            id: SkillId::new(),
            identity_key: "content:retention-test".to_owned(),
            name: "retention-test".to_owned(),
            description: None,
            declared_source: None,
            declared_revision: None,
            content_digest: "sha256:retention-test".to_owned(),
            digest_version: 1,
            governance_state: GovernanceState::Observed,
            canonical_path: None,
        };
        let skill_id = store.save_skill(&skill).unwrap();
        let evidence = EvidenceRecord {
            id: EvidenceId::new(),
            scan_id: scan.id.clone(),
            kind: EvidenceKind::Usage,
            quality: EvidenceQuality::Observed,
            subject_type: "skill".to_owned(),
            subject_id: skill_id.to_string(),
            path: None,
            digest: None,
            details: serde_json::json!({"event_count": 3}),
            observed_at: 10,
        };
        store.save_evidence(&evidence).unwrap();
        store
            .save_usage_event(&UsageEvent {
                evidence_id: evidence.id,
                skill_id: skill_id.clone(),
                agent_id: agent_id.clone(),
                stage: UsageStage::Loaded,
                quality: EvidenceQuality::Observed,
                source_path_digest: "sha256:retention-source".to_owned(),
                observed_event_count: 3,
                occurred_at: Some(10),
                outcome: None,
            })
            .unwrap();

        let result = store.purge_usage_before(100).unwrap();
        assert_eq!(result.aggregated_raw_usage_rows, 1);
        assert_eq!(result.deleted_raw_usage_rows, 1);
        assert_eq!(result.deleted_evidence_rows, 1);
        assert_eq!(result.deleted_payload_usage_summaries, 1);
        let payload_version: i64 = store
            .connection
            .query_row(
                "SELECT updated_at FROM scan_payloads WHERE scan_id = ?1",
                [scan.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(payload_version, 10000000001);
        let counts = store.lifecycle_counts().unwrap();
        assert_eq!(counts.raw_usage_rows, 0);
        assert_eq!(counts.monthly_usage_rows, 1);
        let export = store.export_lifecycle().unwrap();
        assert_eq!(
            export["usage_monthly_sources"][0]["max_observed_event_count"],
            3
        );

        let repeated_evidence = EvidenceRecord {
            id: EvidenceId::new(),
            scan_id: scan.id.clone(),
            kind: EvidenceKind::Usage,
            quality: EvidenceQuality::Observed,
            subject_type: "skill".to_owned(),
            subject_id: skill_id.to_string(),
            path: None,
            digest: None,
            details: serde_json::json!({"event_count": 3}),
            observed_at: 20,
        };
        store.save_evidence(&repeated_evidence).unwrap();
        store
            .save_usage_event(&UsageEvent {
                evidence_id: repeated_evidence.id,
                skill_id,
                agent_id,
                stage: UsageStage::Loaded,
                quality: EvidenceQuality::Observed,
                source_path_digest: "sha256:retention-source".to_owned(),
                observed_event_count: 3,
                occurred_at: Some(20),
                outcome: None,
            })
            .unwrap();
        store.purge_usage_before(100).unwrap();
        let repeated_export = store.export_lifecycle().unwrap();
        assert_eq!(
            repeated_export["usage_monthly_sources"][0]["max_observed_event_count"],
            3
        );

        let (_, payload): (ScanId, serde_json::Value) =
            store.latest_scan_payload().unwrap().unwrap();
        assert_eq!(payload["usage"], serde_json::json!([]));
        assert_eq!(payload["coverage"][0]["denominator_reliable"], false);
    }

    #[test]
    fn unknown_event_time_uses_observation_time_only_for_retention() {
        let store = StateStore::open_in_memory().unwrap();
        let scan = scan();
        store.save_scan(&scan).unwrap();
        store
            .save_scan_payload(
                &scan.id,
                &serde_json::json!({
                    "usage": [{
                        "agent": "codex",
                        "skill_id": "skill_fixture",
                        "stage": "loaded",
                        "quality": "observed",
                        "event_count": 1,
                        "first_seen_unix": null,
                        "last_seen_unix": null,
                        "source_path_digest": "private"
                    }]
                }),
            )
            .unwrap();
        store
            .connection
            .execute("UPDATE scan_payloads SET updated_at = 200", [])
            .unwrap();
        let agent_id = store
            .save_agent(&AgentRecord {
                id: AgentId::new(),
                kind: AgentKind::Codex,
                display_name: "Codex".to_owned(),
            })
            .unwrap();
        let skill_id = store
            .save_skill(&SkillRecord {
                id: SkillId::new(),
                identity_key: "content:unknown-time".to_owned(),
                name: "unknown-time".to_owned(),
                description: None,
                declared_source: None,
                declared_revision: None,
                content_digest: "sha256:unknown-time".to_owned(),
                digest_version: 1,
                governance_state: GovernanceState::Observed,
                canonical_path: None,
            })
            .unwrap();
        let evidence = EvidenceRecord {
            id: EvidenceId::new(),
            scan_id: scan.id.clone(),
            kind: EvidenceKind::Usage,
            quality: EvidenceQuality::Observed,
            subject_type: "skill".to_owned(),
            subject_id: skill_id.to_string(),
            path: None,
            digest: None,
            details: serde_json::json!({
                "event_count": 1,
                "first_seen_unix": null,
                "last_seen_unix": null
            }),
            observed_at: 200,
        };
        store.save_evidence(&evidence).unwrap();
        store
            .save_usage_event(&UsageEvent {
                evidence_id: evidence.id.clone(),
                skill_id: skill_id.clone(),
                agent_id: agent_id.clone(),
                stage: UsageStage::Loaded,
                quality: EvidenceQuality::Observed,
                source_path_digest: "sha256:unknown-time-source".to_owned(),
                observed_event_count: 1,
                occurred_at: None,
                outcome: None,
            })
            .unwrap();
        assert!(
            store
                .save_usage_event(&UsageEvent {
                    evidence_id: evidence.id,
                    skill_id: skill_id.clone(),
                    agent_id: agent_id.clone(),
                    stage: UsageStage::Loaded,
                    quality: EvidenceQuality::Observed,
                    source_path_digest: "sha256:different-identity".to_owned(),
                    observed_event_count: 2,
                    occurred_at: None,
                    outcome: None,
                })
                .is_err(),
            "a primary-key collision with a different observation identity must fail closed"
        );

        assert_eq!(store.lifecycle_counts().unwrap().oldest_raw_usage_at, None);
        assert_eq!(
            store.export_lifecycle().unwrap()["usage_events"][0]["occurred_at"],
            serde_json::Value::Null
        );
        assert_eq!(
            store
                .purge_usage_before(100)
                .unwrap()
                .deleted_raw_usage_rows,
            0
        );
        assert_eq!(store.lifecycle_counts().unwrap().raw_usage_rows, 1);

        let epoch_evidence = EvidenceRecord {
            id: EvidenceId::new(),
            scan_id: scan.id,
            kind: EvidenceKind::Usage,
            quality: EvidenceQuality::Observed,
            subject_type: "skill".to_owned(),
            subject_id: skill_id.to_string(),
            path: None,
            digest: None,
            details: serde_json::json!({"event_count": 1, "last_seen_unix": 0}),
            observed_at: 200,
        };
        store.save_evidence(&epoch_evidence).unwrap();
        store
            .save_usage_event(&UsageEvent {
                evidence_id: epoch_evidence.id,
                skill_id,
                agent_id,
                stage: UsageStage::Loaded,
                quality: EvidenceQuality::Observed,
                source_path_digest: "sha256:epoch-time-source".to_owned(),
                observed_event_count: 1,
                occurred_at: Some(0),
                outcome: None,
            })
            .unwrap();
        assert_eq!(
            store.lifecycle_counts().unwrap().oldest_raw_usage_at,
            Some(0)
        );
        assert!(
            store.export_lifecycle().unwrap()["usage_events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["occurred_at"] == 0)
        );
        let epoch_purge = store.purge_usage_before(100).unwrap();
        assert_eq!(epoch_purge.deleted_raw_usage_rows, 1);
        assert_eq!(store.lifecycle_counts().unwrap().raw_usage_rows, 1);

        let purged = store.purge_usage_before(300).unwrap();
        assert_eq!(purged.aggregated_raw_usage_rows, 1);
        assert_eq!(purged.deleted_raw_usage_rows, 1);
        assert_eq!(purged.deleted_evidence_rows, 1);
        let export = store.export_lifecycle().unwrap();
        let unknown_month = export["usage_monthly_sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["source_path_digest"] == "sha256:unknown-time-source")
            .unwrap();
        assert_eq!(unknown_month["first_seen_at"], 200);
        assert_eq!(unknown_month["last_seen_at"], 200);
        let (_, payload): (ScanId, serde_json::Value) =
            store.latest_scan_payload().unwrap().unwrap();
        assert_eq!(payload["usage"], serde_json::json!([]));
    }
}
