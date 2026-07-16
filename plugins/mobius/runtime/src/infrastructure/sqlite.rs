//! The single project-local SQLite store.
//!
//! Trail bytes and projection bytes are opaque here. The application service owns their strict
//! codecs and executes the fixed admission order through one explicit [`WriteTransaction`].

use std::collections::BTreeSet;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use uuid::Uuid;

use crate::application::admission::{AdmissionError, AdmittedProjectRoot};
use crate::domain::{HeadBinding, ObjectiveId, ProjectId};

const SCHEMA_VERSION: i64 = 1;
const SCHEMA_FINGERPRINT: &str = "mobius.sqlite.v1";
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);
const WAL_NEGOTIATION_RETRY_DELAY: Duration = Duration::from_millis(5);
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";
const MOBIUS_SCHEMA_META_COLUMNS: [&str; 8] = [
    "singleton",
    "schema_version",
    "schema_fingerprint",
    "project_id",
    "canonical_root_digest",
    "project_seq",
    "bootstrap_request_id",
    "bootstrap_payload_hash",
];

const SCHEMA_SQL: &str = r#"
CREATE TABLE schema_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    schema_fingerprint TEXT NOT NULL,
    project_id TEXT NOT NULL UNIQUE,
    canonical_root_digest TEXT NOT NULL,
    project_seq INTEGER NOT NULL CHECK (project_seq >= 0),
    bootstrap_request_id TEXT NOT NULL,
    bootstrap_payload_hash TEXT NOT NULL
) STRICT;

CREATE TABLE objective_streams (
    objective_id TEXT PRIMARY KEY,
    objective_seq INTEGER NOT NULL CHECK (objective_seq >= 0),
    created_project_seq INTEGER NOT NULL CHECK (created_project_seq > 0),
    last_project_seq INTEGER NOT NULL CHECK (last_project_seq >= 0)
) STRICT;

CREATE TABLE trail_events (
    project_seq INTEGER PRIMARY KEY CHECK (project_seq > 0),
    objective_id TEXT NOT NULL,
    objective_seq INTEGER NOT NULL CHECK (objective_seq > 0),
    request_id TEXT NOT NULL UNIQUE,
    request_payload_hash TEXT NOT NULL,
    event_schema TEXT NOT NULL,
    event_bytes BLOB NOT NULL,
    received_at TEXT NOT NULL,
    UNIQUE (objective_id, objective_seq),
    FOREIGN KEY (objective_id) REFERENCES objective_streams(objective_id)
) STRICT;

CREATE TABLE objective_projection (
    objective_id TEXT PRIMARY KEY,
    project_seq INTEGER NOT NULL CHECK (project_seq > 0),
    objective_seq INTEGER NOT NULL CHECK (objective_seq > 0),
    is_active INTEGER NOT NULL CHECK (is_active IN (0, 1)),
    projection_schema TEXT NOT NULL,
    projection_bytes BLOB NOT NULL,
    FOREIGN KEY (objective_id) REFERENCES objective_streams(objective_id),
    FOREIGN KEY (project_seq) REFERENCES trail_events(project_seq)
) STRICT;

CREATE TABLE object_projection (
    objective_id TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    accepted_project_seq INTEGER NOT NULL CHECK (accepted_project_seq > 0),
    projection_schema TEXT NOT NULL,
    projection_bytes BLOB NOT NULL,
    PRIMARY KEY (objective_id, object_kind, object_id),
    FOREIGN KEY (objective_id) REFERENCES objective_streams(objective_id),
    FOREIGN KEY (accepted_project_seq) REFERENCES trail_events(project_seq)
) STRICT;

CREATE UNIQUE INDEX one_active_objective
ON objective_projection(is_active)
WHERE is_active = 1;
"#;

const EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 6] = [
    ("index", "one_active_objective"),
    ("table", "object_projection"),
    ("table", "objective_projection"),
    ("table", "objective_streams"),
    ("table", "schema_meta"),
    ("table", "trail_events"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BootstrapRequest<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) payload_hash: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectBinding {
    pub(crate) project_id: ProjectId,
    pub(crate) canonical_root_digest: String,
    pub(crate) project_seq: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HeadSnapshot {
    pub(crate) project_seq: u64,
    pub(crate) objective_seq: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectiveStreamHead {
    pub(crate) objective_seq: u64,
    pub(crate) created_project_seq: u64,
    pub(crate) last_project_seq: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EventMetadata {
    pub(crate) project_seq: u64,
    pub(crate) objective_id: ObjectiveId,
    pub(crate) objective_seq: u64,
    pub(crate) request_id: String,
    pub(crate) request_payload_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EventRow {
    pub(crate) metadata: EventMetadata,
    pub(crate) event_schema: String,
    pub(crate) event_bytes: Vec<u8>,
    pub(crate) received_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectiveProjectionRow {
    pub(crate) objective_id: ObjectiveId,
    pub(crate) project_seq: u64,
    pub(crate) objective_seq: u64,
    pub(crate) is_active: bool,
    pub(crate) projection_schema: String,
    pub(crate) projection_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectProjectionRow {
    pub(crate) objective_id: ObjectiveId,
    pub(crate) object_kind: String,
    pub(crate) object_id: String,
    pub(crate) accepted_project_seq: u64,
    pub(crate) projection_schema: String,
    pub(crate) projection_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum IntegrityIssue {
    IntegrityCheck(String),
    ForeignKey {
        table: String,
        row_id: Option<i64>,
        parent: String,
        foreign_key: i64,
    },
}

impl IntegrityIssue {
    pub(crate) fn is_projection_foreign_key_violation(&self) -> bool {
        matches!(
            self,
            Self::ForeignKey { table, .. }
                if table == "object_projection" || table == "objective_projection"
        )
    }
}

impl Display for IntegrityIssue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntegrityCheck(message) => write!(formatter, "integrity_check: {message}"),
            Self::ForeignKey {
                table,
                row_id,
                parent,
                foreign_key,
            } => write!(
                formatter,
                "foreign_key_check: table={table} row_id={row_id:?} parent={parent} fk={foreign_key}"
            ),
        }
    }
}

pub(crate) struct AppendEvent<'a> {
    pub(crate) objective_id: &'a ObjectiveId,
    pub(crate) expected_heads: &'a HeadBinding,
    pub(crate) request_id: &'a str,
    pub(crate) request_payload_hash: &'a str,
    pub(crate) event_schema: &'a str,
    pub(crate) event_bytes: &'a [u8],
    pub(crate) received_at: &'a str,
}

#[derive(Debug)]
pub(crate) enum StoreError {
    Admission(AdmissionError),
    Sqlite {
        operation: &'static str,
        source: rusqlite::Error,
    },
    SchemaMismatch(String),
    BindingMissing,
    BindingMismatch,
    InvalidValue(&'static str),
    RequestConflict {
        request_id: String,
    },
    StaleHeads {
        expected: HeadBinding,
        actual: HeadSnapshot,
    },
    HeadOverflow,
    SingleActiveObjective {
        active_objective: ObjectiveId,
    },
    ProjectionHeadMismatch,
    Filesystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    AlternateMobiusDatabase(PathBuf),
}

impl StoreError {
    fn sqlite(operation: &'static str, source: rusqlite::Error) -> Self {
        Self::Sqlite { operation, source }
    }
}

impl Display for StoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(error) => write!(formatter, "project admission failed: {error}"),
            Self::Sqlite { operation, source } => write!(formatter, "{operation}: {source}"),
            Self::SchemaMismatch(reason) => write!(formatter, "Mobius schema mismatch: {reason}"),
            Self::BindingMissing => formatter.write_str("Mobius project binding is missing"),
            Self::BindingMismatch => {
                formatter.write_str("Mobius project binding does not match this project root")
            }
            Self::InvalidValue(field) => write!(formatter, "{field} must not be empty"),
            Self::RequestConflict { request_id } => write!(
                formatter,
                "request id {request_id:?} was already committed with a different payload"
            ),
            Self::StaleHeads { expected, actual } => write!(
                formatter,
                "stale heads: expected project/objective {}/{}, found {}/{}",
                expected.expected_project_seq,
                expected.expected_objective_seq,
                actual.project_seq,
                actual.objective_seq
            ),
            Self::HeadOverflow => formatter.write_str("Trail head overflow"),
            Self::SingleActiveObjective { active_objective } => write!(
                formatter,
                "objective {:?} is already active in this project",
                active_objective.as_str()
            ),
            Self::ProjectionHeadMismatch => {
                formatter.write_str("projection heads do not match the current Trail heads")
            }
            Self::Filesystem {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} for {}: {source}", path.display()),
            Self::AlternateMobiusDatabase(path) => write!(
                formatter,
                "a second Mobius database candidate exists at {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Admission(error) => Some(error),
            Self::Sqlite { source, .. } => Some(source),
            Self::Filesystem { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<AdmissionError> for StoreError {
    fn from(error: AdmissionError) -> Self {
        Self::Admission(error)
    }
}

pub(crate) struct SqliteStore {
    connection: Connection,
    admitted: AdmittedProjectRoot,
    project_id: ProjectId,
}

impl SqliteStore {
    /// Read-only doctor entrypoint. It never creates the database, schema, binding, or managed
    /// directories.
    pub(crate) fn inspect_binding(
        admitted: &AdmittedProjectRoot,
    ) -> Result<ProjectBinding, StoreError> {
        admitted.revalidate()?;
        reject_alternate_mobius_databases(admitted)?;
        let connection =
            Connection::open_with_flags(admitted.database_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|error| {
                    StoreError::sqlite("open Mobius database for inspection", error)
                })?;
        admitted.revalidate()?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| StoreError::sqlite("configure SQLite busy timeout", error))?;
        validate_journal_mode(&connection)?;
        validate_schema_objects(&schema_objects(&connection)?)?;
        let binding = read_binding(&connection)?;
        validate_schema_binding(&binding, admitted, None)?;
        Ok(binding)
    }

    pub(crate) fn bootstrap(
        admitted: &AdmittedProjectRoot,
        request: BootstrapRequest<'_>,
    ) -> Result<ProjectBinding, StoreError> {
        let binding = Self::bootstrap_binding(admitted, request)?;
        // Business identity is durable before these derived/empty directories are completed.
        admitted.ensure_post_commit_layout()?;
        Ok(binding)
    }

    fn bootstrap_binding(
        admitted: &AdmittedProjectRoot,
        request: BootstrapRequest<'_>,
    ) -> Result<ProjectBinding, StoreError> {
        require_nonempty(request.request_id, "bootstrap request id")?;
        require_nonempty(request.payload_hash, "bootstrap payload hash")?;
        admitted.ensure_bootstrap_directory()?;
        reject_alternate_mobius_databases(admitted)?;

        let mut connection = Connection::open_with_flags(
            admitted.database_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|error| StoreError::sqlite("open Mobius database", error))?;
        admitted.revalidate()?;
        configure_bootstrap_connection(&connection)?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Exclusive)
            .map_err(|error| StoreError::sqlite("begin exclusive bootstrap", error))?;
        reject_alternate_mobius_databases(admitted)?;
        let schema_objects = schema_objects(&transaction)?;
        if schema_objects.is_empty() {
            transaction
                .execute_batch(SCHEMA_SQL)
                .map_err(|error| StoreError::sqlite("create Mobius schema", error))?;
            let project_id = ProjectId::new(Uuid::new_v4().to_string());
            transaction
                .execute(
                    "INSERT INTO schema_meta (
                        singleton, schema_version, schema_fingerprint, project_id,
                        canonical_root_digest, project_seq, bootstrap_request_id,
                        bootstrap_payload_hash
                     ) VALUES (1, ?1, ?2, ?3, ?4, 0, ?5, ?6)",
                    params![
                        SCHEMA_VERSION,
                        SCHEMA_FINGERPRINT,
                        project_id.as_str(),
                        admitted.canonical_root_digest(),
                        request.request_id,
                        request.payload_hash,
                    ],
                )
                .map_err(|error| StoreError::sqlite("write project binding", error))?;
        } else {
            validate_schema_objects(&schema_objects)?;
        }

        let binding = read_binding(&transaction)?;
        validate_schema_binding(&binding, admitted, None)?;
        reject_alternate_mobius_databases(admitted)?;
        transaction
            .commit()
            .map_err(|error| StoreError::sqlite("commit project bootstrap", error))?;
        Ok(binding)
    }

    pub(crate) fn open_bound(
        admitted: AdmittedProjectRoot,
        project_id: &ProjectId,
    ) -> Result<Self, StoreError> {
        admitted.revalidate()?;
        reject_alternate_mobius_databases(&admitted)?;
        let connection = Connection::open_with_flags(
            admitted.database_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(|error| StoreError::sqlite("open bound Mobius database", error))?;
        admitted.revalidate()?;
        configure_bound_connection(&connection)?;
        validate_schema_objects(&schema_objects(&connection)?)?;
        let binding = read_binding(&connection)?;
        validate_schema_binding(&binding, &admitted, Some(project_id))?;
        Ok(Self {
            connection,
            admitted,
            project_id: project_id.clone(),
        })
    }

    pub(crate) fn begin_read(&mut self) -> Result<ReadTransaction<'_>, StoreError> {
        self.admitted.revalidate()?;
        reject_alternate_mobius_databases(&self.admitted)?;
        let admitted = self.admitted.clone();
        let project_id = self.project_id.clone();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| StoreError::sqlite("begin read transaction", error))?;
        let binding = read_binding(&transaction)?;
        validate_schema_binding(&binding, &admitted, Some(&project_id))?;
        Ok(ReadTransaction { transaction })
    }

    pub(crate) fn begin_write(&mut self) -> Result<WriteTransaction<'_>, StoreError> {
        self.admitted.revalidate()?;
        reject_alternate_mobius_databases(&self.admitted)?;
        let admitted = self.admitted.clone();
        let project_id = self.project_id.clone();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| StoreError::sqlite("begin write transaction", error))?;
        let binding = read_binding(&transaction)?;
        validate_schema_binding(&binding, &admitted, Some(&project_id))?;
        Ok(WriteTransaction { transaction })
    }
}

fn reject_alternate_mobius_databases(admitted: &AdmittedProjectRoot) -> Result<(), StoreError> {
    let Some(mobius_directory) = admitted.database_path().parent() else {
        return Err(StoreError::SchemaMismatch(
            "Mobius database has no managed parent directory".to_owned(),
        ));
    };
    let payload_roots = [
        mobius_directory.join("artifacts"),
        mobius_directory.join("views"),
    ];
    let mut directories = vec![mobius_directory.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory).map_err(|source| StoreError::Filesystem {
            operation: "inspect managed directory",
            path: directory.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| StoreError::Filesystem {
                operation: "read managed directory entry",
                path: directory.clone(),
                source,
            })?;
            let path = entry.path();
            if path == admitted.database_path() {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(StoreError::Filesystem {
                        operation: "inspect managed directory entry",
                        path: path.clone(),
                        source,
                    });
                }
            };
            if metadata.file_type().is_symlink() {
                // Only regular files can be alternate in-project database candidates. Unknown
                // links are neither followed nor treated as Mobius-owned state; the canonical
                // database and every managed root are validated separately by admission.
                continue;
            }
            if metadata.is_dir() {
                // Artifact blobs may intentionally freeze arbitrary SQLite bytes, and views are
                // deletable presentation payloads. Neither is a Mobius database candidate.
                if directory == mobius_directory && payload_roots.contains(&path) {
                    continue;
                }
                directories.push(path);
                continue;
            }
            if metadata.is_file() && has_mobius_database_fingerprint(&path)? {
                return Err(StoreError::AlternateMobiusDatabase(path));
            }
        }
    }
    Ok(())
}

fn has_mobius_database_fingerprint(path: &Path) -> Result<bool, StoreError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(StoreError::Filesystem {
                operation: "open possible database candidate",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut header = [0_u8; SQLITE_HEADER.len()];
    let read = file
        .read(&mut header)
        .map_err(|source| StoreError::Filesystem {
            operation: "read possible database candidate",
            path: path.to_path_buf(),
            source,
        })?;
    if read != SQLITE_HEADER.len() || &header != SQLITE_HEADER {
        return Ok(false);
    }

    // `immutable=1` is deliberate for the main-file pass: it prevents a nominally read-only
    // check from creating `<candidate>-shm` or `<candidate>-wal` beside a rejected file.
    let mut uri = url::Url::from_file_path(path).map_err(|()| {
        StoreError::SchemaMismatch(format!(
            "possible Mobius database path cannot be represented as a file URI: {}",
            path.display()
        ))
    })?;
    uri.query_pairs_mut().append_pair("immutable", "1");
    let connection = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| StoreError::sqlite("open possible Mobius database candidate", error))?;
    if connection_has_mobius_fingerprint(&connection)? {
        return Ok(true);
    }

    // An immutable connection intentionally ignores WAL. If a live candidate has committed its
    // schema only there, use SQLite's read-only WAL view on the original candidate. This keeps
    // every database byte under the project-local `.mobius/` boundary.
    let wal_path = candidate_sidecar_path(path, "-wal");
    let wal_metadata = match fs::symlink_metadata(&wal_path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(StoreError::Filesystem {
                operation: "inspect possible database WAL",
                path: wal_path,
                source,
            });
        }
    };
    let Some(wal_metadata) = wal_metadata else {
        return Ok(false);
    };
    if wal_metadata.file_type().is_symlink() || !wal_metadata.is_file() {
        return Err(StoreError::SchemaMismatch(format!(
            "possible Mobius database WAL is not a regular file: {}",
            wal_path.display()
        )));
    }
    let shm_path = candidate_sidecar_path(path, "-shm");
    match fs::symlink_metadata(&shm_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(StoreError::SchemaMismatch(format!(
                "possible Mobius database shared memory is not a regular file: {}",
                shm_path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(StoreError::SchemaMismatch(format!(
                "possible Mobius database WAL cannot be inspected without existing shared memory: {}",
                shm_path.display()
            )));
        }
        Err(source) => {
            return Err(StoreError::Filesystem {
                operation: "inspect possible database shared memory",
                path: shm_path,
                source,
            });
        }
    }
    drop(connection);
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| StoreError::sqlite("open possible Mobius WAL candidate", error))?;
    connection_has_mobius_fingerprint(&connection)
}

fn connection_has_mobius_fingerprint(connection: &Connection) -> Result<bool, StoreError> {
    let mut statement = connection
        .prepare("PRAGMA table_info(schema_meta)")
        .map_err(|error| StoreError::sqlite("inspect possible Mobius schema metadata", error))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| StoreError::sqlite("query possible Mobius schema metadata", error))?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| StoreError::sqlite("decode possible Mobius schema metadata", error))?;
    let expected_columns = MOBIUS_SCHEMA_META_COLUMNS
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if columns != expected_columns {
        return Ok(false);
    }

    let binding_rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_meta", [], |row| row.get(0))
        .map_err(|error| StoreError::sqlite("count possible Mobius schema bindings", error))?;
    if binding_rows != 1 {
        return Ok(false);
    }
    let signature = connection
        .query_row(
            "SELECT schema_version, schema_fingerprint FROM schema_meta WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| StoreError::sqlite("read possible Mobius schema fingerprint", error))?;
    Ok(signature.as_ref().is_some_and(|(version, fingerprint)| {
        *version == SCHEMA_VERSION && fingerprint == SCHEMA_FINGERPRINT
    }))
}

fn candidate_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

pub(crate) struct ReadTransaction<'connection> {
    transaction: Transaction<'connection>,
}

impl ReadTransaction<'_> {
    pub(crate) fn project_head(&self) -> Result<u64, StoreError> {
        read_project_head(&self.transaction)
    }

    pub(crate) fn objective_ids(&self) -> Result<Vec<ObjectiveId>, StoreError> {
        read_objective_ids(&self.transaction)
    }

    pub(crate) fn active_objective(&self) -> Result<Option<ObjectiveId>, StoreError> {
        read_active_objective(&self.transaction)
    }

    pub(crate) fn stream_head(
        &self,
        objective_id: &ObjectiveId,
    ) -> Result<Option<ObjectiveStreamHead>, StoreError> {
        read_stream_head(&self.transaction, objective_id)
    }

    /// Explicit audit primitive. Normal opens stay cheap; audit can request SQLite's full
    /// structural and referential checks inside its pinned read transaction.
    pub(crate) fn integrity_issues(&self) -> Result<Vec<IntegrityIssue>, StoreError> {
        read_integrity_issues(&self.transaction)
    }

    pub(crate) fn heads(&self, objective_id: &ObjectiveId) -> Result<HeadSnapshot, StoreError> {
        read_heads(&self.transaction, objective_id)
    }

    pub(crate) fn trail_events(
        &self,
        objective_id: Option<&ObjectiveId>,
    ) -> Result<Vec<EventRow>, StoreError> {
        read_trail_events(&self.transaction, objective_id)
    }

    pub(crate) fn objective_projection(
        &self,
        objective_id: &ObjectiveId,
    ) -> Result<Option<ObjectiveProjectionRow>, StoreError> {
        read_objective_projection(&self.transaction, objective_id)
    }

    pub(crate) fn object_projections(
        &self,
        objective_id: &ObjectiveId,
    ) -> Result<Vec<ObjectProjectionRow>, StoreError> {
        read_object_projections(&self.transaction, objective_id)
    }

    pub(crate) fn commit(self) -> Result<(), StoreError> {
        self.transaction
            .commit()
            .map_err(|error| StoreError::sqlite("finish read transaction", error))
    }
}

pub(crate) struct WriteTransaction<'connection> {
    transaction: Transaction<'connection>,
}

impl WriteTransaction<'_> {
    /// First mutation step after binding validation. A same-payload retry returns the committed
    /// event; a reused request id with different bytes fails before any head check.
    pub(crate) fn lookup_request(
        &self,
        request_id: &str,
        request_payload_hash: &str,
    ) -> Result<Option<EventMetadata>, StoreError> {
        require_nonempty(request_id, "request id")?;
        require_nonempty(request_payload_hash, "request payload hash")?;
        let Some(row) = read_event_by_request(&self.transaction, request_id)? else {
            return Ok(None);
        };
        if row.metadata.request_payload_hash != request_payload_hash {
            return Err(StoreError::RequestConflict {
                request_id: request_id.to_owned(),
            });
        }
        Ok(Some(row.metadata))
    }

    pub(crate) fn heads(&self, objective_id: &ObjectiveId) -> Result<HeadSnapshot, StoreError> {
        read_heads(&self.transaction, objective_id)
    }

    pub(crate) fn project_head(&self) -> Result<u64, StoreError> {
        read_project_head(&self.transaction)
    }

    pub(crate) fn objective_ids(&self) -> Result<Vec<ObjectiveId>, StoreError> {
        read_objective_ids(&self.transaction)
    }

    pub(crate) fn active_objective(&self) -> Result<Option<ObjectiveId>, StoreError> {
        read_active_objective(&self.transaction)
    }

    pub(crate) fn stream_head(
        &self,
        objective_id: &ObjectiveId,
    ) -> Result<Option<ObjectiveStreamHead>, StoreError> {
        read_stream_head(&self.transaction, objective_id)
    }

    pub(crate) fn integrity_issues(&self) -> Result<Vec<IntegrityIssue>, StoreError> {
        read_integrity_issues(&self.transaction)
    }

    pub(crate) fn check_heads(
        &self,
        objective_id: &ObjectiveId,
        expected: &HeadBinding,
    ) -> Result<HeadSnapshot, StoreError> {
        let actual = self.heads(objective_id)?;
        if actual.project_seq != expected.expected_project_seq
            || actual.objective_seq != expected.expected_objective_seq
        {
            return Err(StoreError::StaleHeads {
                expected: expected.clone(),
                actual,
            });
        }
        Ok(actual)
    }

    pub(crate) fn trail_events(
        &self,
        objective_id: Option<&ObjectiveId>,
    ) -> Result<Vec<EventRow>, StoreError> {
        read_trail_events(&self.transaction, objective_id)
    }

    pub(crate) fn objective_projection(
        &self,
        objective_id: &ObjectiveId,
    ) -> Result<Option<ObjectiveProjectionRow>, StoreError> {
        read_objective_projection(&self.transaction, objective_id)
    }

    pub(crate) fn object_projections(
        &self,
        objective_id: &ObjectiveId,
    ) -> Result<Vec<ObjectProjectionRow>, StoreError> {
        read_object_projections(&self.transaction, objective_id)
    }

    pub(crate) fn append_event(&self, event: AppendEvent<'_>) -> Result<EventMetadata, StoreError> {
        require_nonempty(event.objective_id.as_str(), "objective id")?;
        require_nonempty(event.request_id, "request id")?;
        require_nonempty(event.request_payload_hash, "request payload hash")?;
        require_nonempty(event.event_schema, "event schema")?;
        if event.event_bytes.is_empty() {
            return Err(StoreError::InvalidValue("event bytes"));
        }
        require_nonempty(event.received_at, "received time")?;

        if let Some(existing) = self.lookup_request(event.request_id, event.request_payload_hash)? {
            return Ok(existing);
        }
        let current = self.check_heads(event.objective_id, event.expected_heads)?;
        let project_seq = current
            .project_seq
            .checked_add(1)
            .ok_or(StoreError::HeadOverflow)?;
        let objective_seq = current
            .objective_seq
            .checked_add(1)
            .ok_or(StoreError::HeadOverflow)?;

        if current.objective_seq == 0 {
            self.transaction
                .execute(
                    "INSERT INTO objective_streams (
                        objective_id, objective_seq, created_project_seq, last_project_seq
                     ) VALUES (?1, 0, ?2, 0)",
                    params![event.objective_id.as_str(), u64_to_i64(project_seq)?],
                )
                .map_err(|error| StoreError::sqlite("create Objective stream", error))?;
        }

        self.transaction
            .execute(
                "INSERT INTO trail_events (
                    project_seq, objective_id, objective_seq, request_id,
                    request_payload_hash, event_schema, event_bytes, received_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    u64_to_i64(project_seq)?,
                    event.objective_id.as_str(),
                    u64_to_i64(objective_seq)?,
                    event.request_id,
                    event.request_payload_hash,
                    event.event_schema,
                    event.event_bytes,
                    event.received_at,
                ],
            )
            .map_err(|error| StoreError::sqlite("append Trail event", error))?;
        self.transaction
            .execute(
                "UPDATE objective_streams
                 SET objective_seq = ?2, last_project_seq = ?3
                 WHERE objective_id = ?1",
                params![
                    event.objective_id.as_str(),
                    u64_to_i64(objective_seq)?,
                    u64_to_i64(project_seq)?,
                ],
            )
            .map_err(|error| StoreError::sqlite("advance Objective head", error))?;
        self.transaction
            .execute(
                "UPDATE schema_meta SET project_seq = ?1 WHERE singleton = 1",
                [u64_to_i64(project_seq)?],
            )
            .map_err(|error| StoreError::sqlite("advance project head", error))?;

        Ok(EventMetadata {
            project_seq,
            objective_id: event.objective_id.clone(),
            objective_seq,
            request_id: event.request_id.to_owned(),
            request_payload_hash: event.request_payload_hash.to_owned(),
        })
    }

    pub(crate) fn replace_objective_projection(
        &self,
        row: &ObjectiveProjectionRow,
    ) -> Result<(), StoreError> {
        require_nonempty(row.projection_schema.as_str(), "projection schema")?;
        if row.projection_bytes.is_empty() {
            return Err(StoreError::InvalidValue("projection bytes"));
        }
        let Some(stream_head) = self.stream_head(&row.objective_id)? else {
            return Err(StoreError::ProjectionHeadMismatch);
        };
        if stream_head.last_project_seq != row.project_seq
            || stream_head.objective_seq != row.objective_seq
        {
            return Err(StoreError::ProjectionHeadMismatch);
        }
        if row.is_active {
            let active = self
                .transaction
                .query_row(
                    "SELECT objective_id FROM objective_projection
                     WHERE is_active = 1 AND objective_id <> ?1",
                    [row.objective_id.as_str()],
                    |sqlite_row| sqlite_row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| StoreError::sqlite("check active Objective", error))?;
            if let Some(active) = active {
                return Err(StoreError::SingleActiveObjective {
                    active_objective: ObjectiveId::new(active),
                });
            }
        }
        self.transaction
            .execute(
                "INSERT INTO objective_projection (
                    objective_id, project_seq, objective_seq, is_active,
                    projection_schema, projection_bytes
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(objective_id) DO UPDATE SET
                    project_seq = excluded.project_seq,
                    objective_seq = excluded.objective_seq,
                    is_active = excluded.is_active,
                    projection_schema = excluded.projection_schema,
                    projection_bytes = excluded.projection_bytes",
                params![
                    row.objective_id.as_str(),
                    u64_to_i64(row.project_seq)?,
                    u64_to_i64(row.objective_seq)?,
                    i64::from(row.is_active),
                    row.projection_schema,
                    row.projection_bytes,
                ],
            )
            .map_err(|error| StoreError::sqlite("replace Objective projection", error))?;
        Ok(())
    }

    pub(crate) fn replace_object_projections(
        &self,
        objective_id: &ObjectiveId,
        rows: &[ObjectProjectionRow],
    ) -> Result<(), StoreError> {
        for row in rows {
            if &row.objective_id != objective_id {
                return Err(StoreError::InvalidValue(
                    "object projection Objective identity",
                ));
            }
            require_nonempty(&row.object_kind, "object kind")?;
            require_nonempty(&row.object_id, "object id")?;
            require_nonempty(&row.projection_schema, "object projection schema")?;
            if row.projection_bytes.is_empty() {
                return Err(StoreError::InvalidValue("object projection bytes"));
            }
            let accepted = self
                .transaction
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM trail_events
                        WHERE project_seq = ?1 AND objective_id = ?2
                     )",
                    params![u64_to_i64(row.accepted_project_seq)?, objective_id.as_str()],
                    |sqlite_row| sqlite_row.get::<_, bool>(0),
                )
                .map_err(|error| StoreError::sqlite("verify object acceptance event", error))?;
            if !accepted {
                return Err(StoreError::ProjectionHeadMismatch);
            }
        }
        self.transaction
            .execute(
                "DELETE FROM object_projection WHERE objective_id = ?1",
                [objective_id.as_str()],
            )
            .map_err(|error| StoreError::sqlite("clear object projection", error))?;
        for row in rows {
            self.transaction
                .execute(
                    "INSERT INTO object_projection (
                        objective_id, object_kind, object_id, accepted_project_seq,
                        projection_schema, projection_bytes
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        row.objective_id.as_str(),
                        row.object_kind,
                        row.object_id,
                        u64_to_i64(row.accepted_project_seq)?,
                        row.projection_schema,
                        row.projection_bytes,
                    ],
                )
                .map_err(|error| StoreError::sqlite("write object projection", error))?;
        }
        Ok(())
    }

    /// Rebuild primitive. Trail and stream heads remain untouched.
    pub(crate) fn clear_projections(&self) -> Result<(), StoreError> {
        self.transaction
            .execute("DELETE FROM object_projection", [])
            .map_err(|error| StoreError::sqlite("clear all object projections", error))?;
        self.transaction
            .execute("DELETE FROM objective_projection", [])
            .map_err(|error| StoreError::sqlite("clear all Objective projections", error))?;
        Ok(())
    }

    pub(crate) fn commit(self) -> Result<(), StoreError> {
        self.transaction
            .commit()
            .map_err(|error| StoreError::sqlite("commit write transaction", error))
    }
}

fn configure_bootstrap_connection(connection: &Connection) -> Result<(), StoreError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| StoreError::sqlite("configure SQLite busy timeout", error))?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| StoreError::sqlite("enable SQLite foreign keys", error))?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| StoreError::sqlite("configure SQLite durability", error))?;
    let mode = enable_wal_for_bootstrap(connection)?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::SchemaMismatch(format!(
            "journal mode is {mode:?}, expected WAL"
        )));
    }
    Ok(())
}

/// `PRAGMA journal_mode` may return BUSY without invoking SQLite's busy handler. Retrying only
/// that negotiation keeps BEGIN EXCLUSIVE as the sole bootstrap serialization mechanism.
fn enable_wal_for_bootstrap(connection: &Connection) -> Result<String, StoreError> {
    let started = Instant::now();
    loop {
        match connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0)) {
            Ok(mode) => return Ok(mode),
            Err(error)
                if matches!(
                    error.sqlite_error_code(),
                    Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
                ) && started.elapsed() < BUSY_TIMEOUT =>
            {
                thread::sleep(WAL_NEGOTIATION_RETRY_DELAY);
            }
            Err(error) => return Err(StoreError::sqlite("enable SQLite WAL", error)),
        }
    }
}

fn configure_bound_connection(connection: &Connection) -> Result<(), StoreError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| StoreError::sqlite("configure SQLite busy timeout", error))?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| StoreError::sqlite("enable SQLite foreign keys", error))?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| StoreError::sqlite("configure SQLite durability", error))?;
    validate_journal_mode(connection)
}

fn validate_journal_mode(connection: &Connection) -> Result<(), StoreError> {
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|error| StoreError::sqlite("read SQLite journal mode", error))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::SchemaMismatch(format!(
            "journal mode is {mode:?}, expected WAL"
        )));
    }
    Ok(())
}

fn schema_objects(connection: &Connection) -> Result<Vec<(String, String)>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT type, name FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%'
             ORDER BY type, name",
        )
        .map_err(|error| StoreError::sqlite("inspect Mobius schema", error))?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| StoreError::sqlite("inspect Mobius schema", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| StoreError::sqlite("inspect Mobius schema", error))
}

fn validate_schema_objects(objects: &[(String, String)]) -> Result<(), StoreError> {
    let actual = objects
        .iter()
        .map(|(kind, name)| (kind.as_str(), name.as_str()))
        .collect::<Vec<_>>();
    if actual != EXPECTED_SCHEMA_OBJECTS {
        return Err(StoreError::SchemaMismatch(format!(
            "expected only the five Core tables and active-objective index, found {actual:?}"
        )));
    }
    Ok(())
}

fn read_binding(connection: &Connection) -> Result<ProjectBinding, StoreError> {
    let binding_rows = connection
        .query_row("SELECT COUNT(*) FROM schema_meta", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| StoreError::sqlite("count project bindings", error))?;
    if binding_rows == 0 {
        return Err(StoreError::BindingMissing);
    }
    if binding_rows != 1 {
        return Err(StoreError::SchemaMismatch(format!(
            "schema_meta contains {binding_rows} binding rows, expected one"
        )));
    }
    let rows = connection
        .query_row(
            "SELECT schema_version, schema_fingerprint, project_id,
                    canonical_root_digest, project_seq
             FROM schema_meta WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| StoreError::sqlite("read project binding", error))?;
    let Some((version, fingerprint, project_id, digest, project_seq)) = rows else {
        return Err(StoreError::BindingMissing);
    };
    if version != SCHEMA_VERSION || fingerprint != SCHEMA_FINGERPRINT {
        return Err(StoreError::SchemaMismatch(format!(
            "schema identity {version}/{fingerprint:?} is unsupported"
        )));
    }
    if project_id.is_empty() || digest.is_empty() || project_seq < 0 {
        return Err(StoreError::SchemaMismatch(
            "binding contains an invalid value".into(),
        ));
    }
    Ok(ProjectBinding {
        project_id: ProjectId::new(project_id),
        canonical_root_digest: digest,
        project_seq: i64_to_u64(project_seq)?,
    })
}

fn validate_schema_binding(
    binding: &ProjectBinding,
    admitted: &AdmittedProjectRoot,
    project_id: Option<&ProjectId>,
) -> Result<(), StoreError> {
    if binding.canonical_root_digest != admitted.canonical_root_digest()
        || project_id.is_some_and(|expected| expected != &binding.project_id)
    {
        return Err(StoreError::BindingMismatch);
    }
    Ok(())
}

fn read_heads(
    connection: &Connection,
    objective_id: &ObjectiveId,
) -> Result<HeadSnapshot, StoreError> {
    let project_seq = read_project_head(connection)?;
    let objective_seq = read_stream_head(connection, objective_id)?
        .map(|head| head.objective_seq)
        .unwrap_or(0);
    Ok(HeadSnapshot {
        project_seq,
        objective_seq,
    })
}

fn read_stream_head(
    connection: &Connection,
    objective_id: &ObjectiveId,
) -> Result<Option<ObjectiveStreamHead>, StoreError> {
    let row = connection
        .query_row(
            "SELECT objective_seq, created_project_seq, last_project_seq FROM objective_streams
             WHERE objective_id = ?1",
            [objective_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| StoreError::sqlite("read Objective stream head", error))?;
    row.map(|(objective_seq, created_project_seq, last_project_seq)| {
        Ok(ObjectiveStreamHead {
            objective_seq: i64_to_u64(objective_seq)?,
            created_project_seq: i64_to_u64(created_project_seq)?,
            last_project_seq: i64_to_u64(last_project_seq)?,
        })
    })
    .transpose()
}

fn read_project_head(connection: &Connection) -> Result<u64, StoreError> {
    let project_seq = connection
        .query_row(
            "SELECT project_seq FROM schema_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| StoreError::sqlite("read project head", error))?;
    i64_to_u64(project_seq)
}

fn read_objective_ids(connection: &Connection) -> Result<Vec<ObjectiveId>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT objective_id FROM objective_streams
             ORDER BY created_project_seq, objective_id",
        )
        .map_err(|error| StoreError::sqlite("read Objective identities", error))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0).map(ObjectiveId::new))
        .map_err(|error| StoreError::sqlite("read Objective identities", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| StoreError::sqlite("read Objective identities", error))
}

fn read_active_objective(connection: &Connection) -> Result<Option<ObjectiveId>, StoreError> {
    connection
        .query_row(
            "SELECT objective_id FROM objective_projection WHERE is_active = 1",
            [],
            |row| row.get::<_, String>(0).map(ObjectiveId::new),
        )
        .optional()
        .map_err(|error| StoreError::sqlite("read active Objective", error))
}

fn read_integrity_issues(connection: &Connection) -> Result<Vec<IntegrityIssue>, StoreError> {
    let mut issues = Vec::new();
    let mut statement = connection
        .prepare("PRAGMA integrity_check")
        .map_err(|error| StoreError::sqlite("prepare SQLite integrity check", error))?;
    let messages = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| StoreError::sqlite("run SQLite integrity check", error))?;
    for message in messages {
        let message =
            message.map_err(|error| StoreError::sqlite("read SQLite integrity result", error))?;
        if !message.eq_ignore_ascii_case("ok") {
            issues.push(IntegrityIssue::IntegrityCheck(message));
        }
    }
    drop(statement);

    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|error| StoreError::sqlite("prepare SQLite foreign-key check", error))?;
    let violations = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| StoreError::sqlite("run SQLite foreign-key check", error))?;
    for violation in violations {
        let (table, row_id, parent, foreign_key) = violation
            .map_err(|error| StoreError::sqlite("read SQLite foreign-key result", error))?;
        issues.push(IntegrityIssue::ForeignKey {
            table,
            row_id,
            parent,
            foreign_key,
        });
    }
    Ok(issues)
}

fn read_event_by_request(
    connection: &Connection,
    request_id: &str,
) -> Result<Option<EventRow>, StoreError> {
    connection
        .query_row(
            "SELECT project_seq, objective_id, objective_seq, request_id,
                    request_payload_hash, event_schema, event_bytes, received_at
             FROM trail_events WHERE request_id = ?1",
            [request_id],
            decode_event_row,
        )
        .optional()
        .map_err(|error| StoreError::sqlite("read idempotent request", error))
}

fn read_trail_events(
    connection: &Connection,
    objective_id: Option<&ObjectiveId>,
) -> Result<Vec<EventRow>, StoreError> {
    let (sql, binding) = if let Some(objective_id) = objective_id {
        (
            "SELECT project_seq, objective_id, objective_seq, request_id,
                    request_payload_hash, event_schema, event_bytes, received_at
             FROM trail_events WHERE objective_id = ?1 ORDER BY project_seq",
            Some(objective_id.as_str()),
        )
    } else {
        (
            "SELECT project_seq, objective_id, objective_seq, request_id,
                    request_payload_hash, event_schema, event_bytes, received_at
             FROM trail_events ORDER BY project_seq",
            None,
        )
    };
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| StoreError::sqlite("read Trail", error))?;
    let rows = if let Some(binding) = binding {
        statement.query_map([binding], decode_event_row)
    } else {
        statement.query_map([], decode_event_row)
    }
    .map_err(|error| StoreError::sqlite("read Trail", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| StoreError::sqlite("read Trail", error))
}

fn decode_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    let project_seq = row.get::<_, i64>(0)?;
    let objective_seq = row.get::<_, i64>(2)?;
    if project_seq < 0 || objective_seq < 0 {
        return Err(rusqlite::Error::IntegralValueOutOfRange(0, project_seq));
    }
    Ok(EventRow {
        metadata: EventMetadata {
            project_seq: project_seq as u64,
            objective_id: ObjectiveId::new(row.get::<_, String>(1)?),
            objective_seq: objective_seq as u64,
            request_id: row.get(3)?,
            request_payload_hash: row.get(4)?,
        },
        event_schema: row.get(5)?,
        event_bytes: row.get(6)?,
        received_at: row.get(7)?,
    })
}

fn read_objective_projection(
    connection: &Connection,
    objective_id: &ObjectiveId,
) -> Result<Option<ObjectiveProjectionRow>, StoreError> {
    connection
        .query_row(
            "SELECT objective_id, project_seq, objective_seq, is_active,
                    projection_schema, projection_bytes
             FROM objective_projection WHERE objective_id = ?1",
            [objective_id.as_str()],
            |row| {
                let project_seq = row.get::<_, i64>(1)?;
                let objective_seq = row.get::<_, i64>(2)?;
                if project_seq < 0 || objective_seq < 0 {
                    return Err(rusqlite::Error::IntegralValueOutOfRange(1, project_seq));
                }
                Ok(ObjectiveProjectionRow {
                    objective_id: ObjectiveId::new(row.get::<_, String>(0)?),
                    project_seq: project_seq as u64,
                    objective_seq: objective_seq as u64,
                    is_active: row.get::<_, i64>(3)? != 0,
                    projection_schema: row.get(4)?,
                    projection_bytes: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|error| StoreError::sqlite("read Objective projection", error))
}

fn read_object_projections(
    connection: &Connection,
    objective_id: &ObjectiveId,
) -> Result<Vec<ObjectProjectionRow>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT objective_id, object_kind, object_id, accepted_project_seq,
                    projection_schema, projection_bytes
             FROM object_projection WHERE objective_id = ?1
             ORDER BY object_kind, object_id",
        )
        .map_err(|error| StoreError::sqlite("read object projection", error))?;
    let rows = statement
        .query_map([objective_id.as_str()], |row| {
            let accepted_project_seq = row.get::<_, i64>(3)?;
            if accepted_project_seq < 0 {
                return Err(rusqlite::Error::IntegralValueOutOfRange(
                    3,
                    accepted_project_seq,
                ));
            }
            Ok(ObjectProjectionRow {
                objective_id: ObjectiveId::new(row.get::<_, String>(0)?),
                object_kind: row.get(1)?,
                object_id: row.get(2)?,
                accepted_project_seq: accepted_project_seq as u64,
                projection_schema: row.get(4)?,
                projection_bytes: row.get(5)?,
            })
        })
        .map_err(|error| StoreError::sqlite("read object projection", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| StoreError::sqlite("read object projection", error))
}

fn require_nonempty(value: &str, field: &'static str) -> Result<(), StoreError> {
    if value.is_empty() {
        Err(StoreError::InvalidValue(field))
    } else {
        Ok(())
    }
}

fn i64_to_u64(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::SchemaMismatch("negative Trail head".into()))
}

fn u64_to_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::HeadOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::admission::{ManagedPath, admit_project_root};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Arc, Barrier};
    use std::thread;

    struct TestProject(PathBuf);

    impl TestProject {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("mobius-sqlite-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn admitted(&self) -> AdmittedProjectRoot {
            admit_project_root(self.path(), std::slice::from_ref(&self.0)).unwrap()
        }

        fn bootstrap(&self) -> ProjectBinding {
            SqliteStore::bootstrap(
                &self.admitted(),
                BootstrapRequest {
                    request_id: "bootstrap-1",
                    payload_hash: "root-payload",
                },
            )
            .unwrap()
        }

        fn store(&self, binding: &ProjectBinding) -> SqliteStore {
            SqliteStore::open_bound(self.admitted(), &binding.project_id).unwrap()
        }
    }

    impl Drop for TestProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn append(
        transaction: &WriteTransaction<'_>,
        objective_id: &ObjectiveId,
        heads: &HeadBinding,
        request_id: &str,
        payload_hash: &str,
    ) -> Result<EventMetadata, StoreError> {
        transaction.append_event(AppendEvent {
            objective_id,
            expected_heads: heads,
            request_id,
            request_payload_hash: payload_hash,
            event_schema: "mobius.event.v1",
            event_bytes: br#"{"event":"opaque"}"#,
            received_at: "2026-07-15T00:00:00Z",
        })
    }

    #[test]
    fn bootstrap_is_response_loss_idempotent_and_uses_uuid_v4() {
        let project = TestProject::new();
        let first = project.bootstrap();
        let second = SqliteStore::bootstrap(
            &project.admitted(),
            BootstrapRequest {
                request_id: "bootstrap-retry",
                payload_hash: "root-payload",
            },
        )
        .unwrap();

        assert_eq!(first, second);
        assert_eq!(
            Uuid::parse_str(first.project_id.as_str())
                .unwrap()
                .get_version_num(),
            4
        );
        assert!(project.path().join(".mobius/artifacts/blobs").is_dir());
        assert!(project.path().join(".mobius/views").is_dir());
        let store = project.store(&first);
        let journal_mode: String = store
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        let synchronous: i64 = store
            .connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "wal");
        assert_eq!(synchronous, 2, "SQLite synchronous must be FULL");
    }

    #[test]
    fn bootstrap_rejects_a_second_fingerprinted_mobius_database_without_deleting_it() {
        let source = TestProject::new();
        source.bootstrap();

        let target = TestProject::new();
        let admitted = target.admitted();
        admitted.ensure_bootstrap_directory().unwrap();
        let alternate = target.path().join(".mobius/legacy.sqlite3");
        fs::copy(source.path().join(".mobius/mobius.sqlite3"), &alternate).unwrap();

        let result = SqliteStore::bootstrap(
            &admitted,
            BootstrapRequest {
                request_id: "second-database",
                payload_hash: "root-payload",
            },
        );
        assert!(matches!(
            result,
            Err(StoreError::AlternateMobiusDatabase(path)) if path == alternate
        ));
        assert!(alternate.is_file());
        assert!(!target.path().join(".mobius/legacy.sqlite3-shm").exists());
        assert!(!target.path().join(".mobius/legacy.sqlite3-wal").exists());
        assert!(!admitted.database_path().exists());
    }

    #[test]
    fn nested_unknown_subtree_rejects_a_mobius_database_but_payload_roots_do_not() {
        let source = TestProject::new();
        source.bootstrap();

        let target = TestProject::new();
        let admitted = target.admitted();
        admitted.ensure_bootstrap_directory().unwrap();
        let backup = target.path().join(".mobius/backup");
        fs::create_dir(&backup).unwrap();
        let nested = backup.join("legacy.sqlite3");
        fs::copy(source.path().join(".mobius/mobius.sqlite3"), &nested).unwrap();
        assert!(matches!(
            SqliteStore::bootstrap(
                &admitted,
                BootstrapRequest {
                    request_id: "nested-second-database",
                    payload_hash: "root-payload",
                },
            ),
            Err(StoreError::AlternateMobiusDatabase(path)) if path == nested
        ));
        assert!(nested.is_file());
        assert!(!admitted.database_path().exists());

        fs::remove_dir_all(&backup).unwrap();
        let binding = target.bootstrap();
        let frozen_database = target.path().join(".mobius/artifacts/blobs/sqlite-payload");
        fs::copy(
            source.path().join(".mobius/mobius.sqlite3"),
            &frozen_database,
        )
        .unwrap();
        assert_eq!(SqliteStore::inspect_binding(&admitted).unwrap(), binding);
        assert!(frozen_database.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn unknown_symlink_is_ignored_without_following_or_deleting_it() {
        use std::os::unix::fs::symlink;

        let source = TestProject::new();
        source.bootstrap();
        let external = source.path().join(".mobius/mobius.sqlite3");

        let target = TestProject::new();
        let admitted = target.admitted();
        admitted.ensure_bootstrap_directory().unwrap();
        let candidate = target.path().join(".mobius/legacy.sqlite3");
        symlink(&external, &candidate).unwrap();

        let binding = SqliteStore::bootstrap(
            &admitted,
            BootstrapRequest {
                request_id: "symlinked-second-database",
                payload_hash: "root-payload",
            },
        )
        .expect("unknown non-owned symlinks must not disable Mobius");
        assert_eq!(SqliteStore::inspect_binding(&admitted).unwrap(), binding);
        assert!(
            fs::symlink_metadata(&candidate)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(external.is_file());
        assert!(admitted.database_path().is_file());
    }

    #[cfg(unix)]
    #[test]
    fn bootstrap_rejects_database_family_symlinks_without_touching_external_targets() {
        use std::os::unix::fs::symlink;

        for (suffix, expected_path) in [
            ("-wal", ManagedPath::DatabaseWal),
            ("-shm", ManagedPath::DatabaseShm),
        ] {
            let project = TestProject::new();
            let admitted = project.admitted();
            admitted.ensure_bootstrap_directory().unwrap();

            let external = TestProject::new();
            let external_target = external.path().join("external-sidecar");
            let expected = format!("external sentinel for {suffix}").into_bytes();
            fs::write(&external_target, &expected).unwrap();

            let sidecar = candidate_sidecar_path(admitted.database_path(), suffix);
            symlink(&external_target, &sidecar).unwrap();

            let result = SqliteStore::bootstrap(
                &admitted,
                BootstrapRequest {
                    request_id: "managed-sidecar-symlink",
                    payload_hash: "root-payload",
                },
            );
            assert!(
                matches!(
                    result,
                    Err(StoreError::Admission(AdmissionError::Symlink(actual_path)))
                        if actual_path == expected_path
                ),
                "bootstrap must reject the canonical {suffix} symlink before SQLite opens"
            );
            assert_eq!(fs::read(&external_target).unwrap(), expected);
            assert!(external_target.is_file());
            assert!(
                fs::symlink_metadata(&sidecar)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert!(!admitted.database_path().exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn bound_reopen_rejects_database_family_symlinks_without_touching_external_targets() {
        use std::os::unix::fs::symlink;

        for (suffix, expected_path) in [
            ("-wal", ManagedPath::DatabaseWal),
            ("-shm", ManagedPath::DatabaseShm),
        ] {
            let project = TestProject::new();
            let binding = project.bootstrap();
            let admitted = project.admitted();
            let database = admitted.database_path().to_owned();
            let sidecar = candidate_sidecar_path(&database, suffix);
            match fs::remove_file(&sidecar) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => panic!("remove existing {suffix} sidecar: {error}"),
            }

            let external = TestProject::new();
            let external_target = external.path().join("external-sidecar");
            let expected = format!("external sentinel for {suffix}").into_bytes();
            fs::write(&external_target, &expected).unwrap();
            symlink(&external_target, &sidecar).unwrap();

            let result = SqliteStore::open_bound(admitted, &binding.project_id);
            assert!(
                matches!(
                    result,
                    Err(StoreError::Admission(AdmissionError::Symlink(actual_path)))
                        if actual_path == expected_path
                ),
                "bound reopen must reject the canonical {suffix} symlink before SQLite opens"
            );
            assert_eq!(fs::read(&external_target).unwrap(), expected);
            assert!(external_target.is_file());
            assert!(
                fs::symlink_metadata(&sidecar)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert!(database.is_file());
        }
    }

    #[test]
    fn unrelated_sqlite_files_in_the_private_root_are_preserved_but_not_candidates() {
        let project = TestProject::new();
        let admitted = project.admitted();
        admitted.ensure_bootstrap_directory().unwrap();
        let unrelated = project.path().join(".mobius/unrelated.sqlite3");
        let connection = Connection::open(&unrelated).unwrap();
        connection
            .execute("CREATE TABLE notes (body TEXT NOT NULL)", [])
            .unwrap();
        drop(connection);

        let binding = project.bootstrap();
        assert!(unrelated.is_file());
        assert!(admitted.database_path().is_file());
        assert_eq!(project.store(&binding).project_id, binding.project_id);
    }

    #[test]
    fn matching_schema_meta_columns_with_a_foreign_fingerprint_are_not_mobius() {
        let project = TestProject::new();
        let admitted = project.admitted();
        admitted.ensure_bootstrap_directory().unwrap();
        let unrelated = project.path().join(".mobius/unrelated.sqlite3");
        let connection = Connection::open(&unrelated).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_meta (
                    singleton INTEGER PRIMARY KEY,
                    schema_version INTEGER NOT NULL,
                    schema_fingerprint TEXT NOT NULL,
                    project_id TEXT NOT NULL,
                    canonical_root_digest TEXT NOT NULL,
                    project_seq INTEGER NOT NULL,
                    bootstrap_request_id TEXT NOT NULL,
                    bootstrap_payload_hash TEXT NOT NULL
                );
                INSERT INTO schema_meta VALUES (
                    1, 1, 'unrelated.application', 'foreign-project', 'foreign-root', 0,
                    'foreign-request', 'foreign-payload'
                );",
            )
            .unwrap();
        drop(connection);

        let binding = project.bootstrap();
        assert!(unrelated.is_file());
        assert!(admitted.database_path().is_file());
        assert_eq!(project.store(&binding).project_id, binding.project_id);
    }

    #[test]
    fn live_candidate_with_committed_schema_only_in_wal_is_rejected_without_new_sidecars() {
        let project = TestProject::new();
        let admitted = project.admitted();
        admitted.ensure_bootstrap_directory().unwrap();
        let alternate = project.path().join(".mobius/legacy.sqlite3");
        let connection = Connection::open(&alternate).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA journal_mode = WAL", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        connection
            .execute_batch("PRAGMA wal_autocheckpoint = 0;")
            .unwrap();
        connection.execute_batch(SCHEMA_SQL).unwrap();
        connection
            .execute(
                "INSERT INTO schema_meta (
                    singleton, schema_version, schema_fingerprint, project_id,
                    canonical_root_digest, project_seq, bootstrap_request_id,
                    bootstrap_payload_hash
                 ) VALUES (1, ?1, ?2, ?3, ?4, 0, ?5, ?6)",
                params![
                    SCHEMA_VERSION,
                    SCHEMA_FINGERPRINT,
                    Uuid::new_v4().to_string(),
                    "alternate-root",
                    "alternate-request",
                    "alternate-payload",
                ],
            )
            .unwrap();
        let wal = project.path().join(".mobius/legacy.sqlite3-wal");
        let shm = project.path().join(".mobius/legacy.sqlite3-shm");
        assert!(wal.is_file());
        assert!(shm.is_file());

        let result = SqliteStore::bootstrap(
            &admitted,
            BootstrapRequest {
                request_id: "live-wal-candidate",
                payload_hash: "root-payload",
            },
        );
        assert!(matches!(
            result,
            Err(StoreError::AlternateMobiusDatabase(path)) if path == alternate
        ));
        assert!(wal.is_file());
        assert!(shm.is_file());
        assert!(!admitted.database_path().exists());
        drop(connection);
    }

    #[test]
    fn wal_candidate_without_shared_memory_fails_closed_without_touching_the_family() {
        let source = TestProject::new();
        let source_admitted = source.admitted();
        source_admitted.ensure_bootstrap_directory().unwrap();
        let source_database = source.path().join(".mobius/source.sqlite3");
        let connection = Connection::open(&source_database).unwrap();
        connection
            .execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;")
            .unwrap();
        connection.execute_batch(SCHEMA_SQL).unwrap();
        connection
            .execute(
                "INSERT INTO schema_meta (
                    singleton, schema_version, schema_fingerprint, project_id,
                    canonical_root_digest, project_seq, bootstrap_request_id,
                    bootstrap_payload_hash
                 ) VALUES (1, ?1, ?2, ?3, ?4, 0, ?5, ?6)",
                params![
                    SCHEMA_VERSION,
                    SCHEMA_FINGERPRINT,
                    Uuid::new_v4().to_string(),
                    "source-root",
                    "source-request",
                    "source-payload",
                ],
            )
            .unwrap();
        let source_wal = source.path().join(".mobius/source.sqlite3-wal");
        assert!(source_wal.is_file());

        let target = TestProject::new();
        let admitted = target.admitted();
        admitted.ensure_bootstrap_directory().unwrap();
        let candidate = target.path().join(".mobius/copy.sqlite3");
        let candidate_wal = target.path().join(".mobius/copy.sqlite3-wal");
        fs::copy(&source_database, &candidate).unwrap();
        fs::copy(&source_wal, &candidate_wal).unwrap();
        let before_main = fs::read(&candidate).unwrap();
        let before_wal = fs::read(&candidate_wal).unwrap();
        let before_entries = fs::read_dir(target.path().join(".mobius"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>();

        let error = SqliteStore::bootstrap(
            &admitted,
            BootstrapRequest {
                request_id: "wal-without-shm",
                payload_hash: "root-payload",
            },
        )
        .expect_err("an ambiguous WAL family must fail closed");
        assert!(
            matches!(error, StoreError::SchemaMismatch(message) if message.contains("without existing shared memory"))
        );
        assert_eq!(fs::read(&candidate).unwrap(), before_main);
        assert_eq!(fs::read(&candidate_wal).unwrap(), before_wal);
        let after_entries = fs::read_dir(target.path().join(".mobius"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>();
        assert_eq!(after_entries, before_entries);
        assert!(!target.path().join(".mobius/copy.sqlite3-shm").exists());
        assert!(!admitted.database_path().exists());
        drop(connection);
    }

    #[test]
    fn bound_operations_fail_closed_if_a_second_mobius_database_appears() {
        let project = TestProject::new();
        let binding = project.bootstrap();
        let alternate = project.path().join(".mobius/legacy.sqlite3");
        fs::copy(project.path().join(".mobius/mobius.sqlite3"), &alternate).unwrap();

        assert!(matches!(
            SqliteStore::inspect_binding(&project.admitted()),
            Err(StoreError::AlternateMobiusDatabase(path)) if path == alternate
        ));
        assert!(matches!(
            SqliteStore::open_bound(project.admitted(), &binding.project_id),
            Err(StoreError::AlternateMobiusDatabase(path)) if path == alternate
        ));
        assert!(alternate.is_file());
    }

    #[test]
    fn binding_inspection_is_read_only_and_requires_an_existing_database() {
        let project = TestProject::new();
        let admitted = project.admitted();
        assert!(SqliteStore::inspect_binding(&admitted).is_err());
        assert!(!project.path().join(".mobius").exists());

        let binding = project.bootstrap();
        assert_eq!(SqliteStore::inspect_binding(&admitted).unwrap(), binding);
    }

    #[test]
    fn concurrent_bootstrap_returns_one_binding() {
        let missing_candidate =
            std::env::temp_dir().join(format!("mobius-missing-candidate-{}", uuid::Uuid::new_v4()));
        assert!(
            !has_mobius_database_fingerprint(&missing_candidate).unwrap(),
            "opening an already absent candidate must report no fingerprint"
        );
        for round in 0..16 {
            let project = Arc::new(TestProject::new());
            let barrier = Arc::new(Barrier::new(5));
            let mut handles = Vec::new();
            for ordinal in 0..4 {
                let project = Arc::clone(&project);
                let barrier = Arc::clone(&barrier);
                handles.push(thread::spawn(move || {
                    let admitted = project.admitted();
                    barrier.wait();
                    SqliteStore::bootstrap(
                        &admitted,
                        BootstrapRequest {
                            request_id: &format!("bootstrap-{round}-{ordinal}"),
                            payload_hash: "root-payload",
                        },
                    )
                    .unwrap()
                }));
            }
            barrier.wait();
            let bindings = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>();
            assert!(bindings.windows(2).all(|pair| pair[0] == pair[1]));
        }
    }

    #[test]
    fn partial_schema_is_rejected_without_repair() {
        let project = TestProject::new();
        let admitted = project.admitted();
        admitted.ensure_bootstrap_directory().unwrap();
        let connection = Connection::open(admitted.database_path()).unwrap();
        connection
            .execute(
                "CREATE TABLE schema_meta (singleton INTEGER PRIMARY KEY)",
                [],
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            SqliteStore::bootstrap(
                &admitted,
                BootstrapRequest {
                    request_id: "bootstrap-1",
                    payload_hash: "root-payload"
                }
            ),
            Err(StoreError::SchemaMismatch(_))
        ));
    }

    #[test]
    fn copied_database_cannot_rebind_to_another_project() {
        let source = TestProject::new();
        source.bootstrap();
        let target = TestProject::new();
        let target_admitted = target.admitted();
        target_admitted.ensure_bootstrap_directory().unwrap();
        fs::copy(
            source.path().join(".mobius/mobius.sqlite3"),
            target_admitted.database_path(),
        )
        .unwrap();

        assert!(matches!(
            SqliteStore::bootstrap(
                &target_admitted,
                BootstrapRequest {
                    request_id: "bootstrap-2",
                    payload_hash: "root-payload"
                }
            ),
            Err(StoreError::BindingMismatch)
        ));
    }

    #[test]
    fn every_open_rechecks_the_submitted_project_identity() {
        let project = TestProject::new();
        project.bootstrap();
        assert!(matches!(
            SqliteStore::open_bound(project.admitted(), &ProjectId::new("foreign-project")),
            Err(StoreError::BindingMismatch)
        ));
    }

    #[test]
    fn append_is_ordered_idempotent_and_stale_safe() {
        let project = TestProject::new();
        let binding = project.bootstrap();
        let mut store = project.store(&binding);
        let objective = ObjectiveId::new("objective-1");

        let transaction = store.begin_write().unwrap();
        let metadata = append(
            &transaction,
            &objective,
            &HeadBinding {
                expected_project_seq: 0,
                expected_objective_seq: 0,
            },
            "request-1",
            "payload-1",
        )
        .unwrap();
        transaction.commit().unwrap();
        assert_eq!((metadata.project_seq, metadata.objective_seq), (1, 1));

        let transaction = store.begin_write().unwrap();
        assert_eq!(
            transaction
                .lookup_request("request-1", "payload-1")
                .unwrap(),
            Some(metadata.clone())
        );
        assert!(matches!(
            transaction.lookup_request("request-1", "different"),
            Err(StoreError::RequestConflict { .. })
        ));
        assert!(matches!(
            transaction.check_heads(
                &objective,
                &HeadBinding {
                    expected_project_seq: 0,
                    expected_objective_seq: 0
                }
            ),
            Err(StoreError::StaleHeads { .. })
        ));
        assert_eq!(
            append(
                &transaction,
                &objective,
                &HeadBinding {
                    expected_project_seq: 0,
                    expected_objective_seq: 0,
                },
                "request-1",
                "payload-1",
            )
            .unwrap(),
            metadata
        );
        transaction.commit().unwrap();
        let transaction = store.begin_read().unwrap();
        assert_eq!(transaction.project_head().unwrap(), 1);
        assert_eq!(
            transaction.stream_head(&objective).unwrap(),
            Some(ObjectiveStreamHead {
                objective_seq: 1,
                created_project_seq: 1,
                last_project_seq: 1,
            })
        );
        assert_eq!(transaction.trail_events(None).unwrap().len(), 1);
    }

    #[test]
    fn project_head_rejects_a_cross_objective_stale_append() {
        let project = TestProject::new();
        let binding = project.bootstrap();
        let mut store = project.store(&binding);
        let first = ObjectiveId::new("objective-1");
        let second = ObjectiveId::new("objective-2");

        let transaction = store.begin_write().unwrap();
        append(
            &transaction,
            &first,
            &HeadBinding {
                expected_project_seq: 0,
                expected_objective_seq: 0,
            },
            "request-1",
            "payload-1",
        )
        .unwrap();
        transaction.commit().unwrap();

        let transaction = store.begin_write().unwrap();
        assert!(matches!(
            append(
                &transaction,
                &second,
                &HeadBinding {
                    expected_project_seq: 0,
                    expected_objective_seq: 0,
                },
                "request-2",
                "payload-2",
            ),
            Err(StoreError::StaleHeads { .. })
        ));
        assert_eq!(transaction.objective_ids().unwrap(), vec![first]);
    }

    const CRASH_ROOT_ENV: &str = "MOBIUS_SQLITE_CRASH_TEST_ROOT";
    const CRASH_PROJECT_ENV: &str = "MOBIUS_SQLITE_CRASH_TEST_PROJECT";
    const CRASH_MODE_ENV: &str = "MOBIUS_SQLITE_CRASH_TEST_MODE";

    #[test]
    fn bootstrap_process_crash_writer() {
        let Some(root) = std::env::var_os(CRASH_ROOT_ENV) else {
            return;
        };
        let root = PathBuf::from(root);
        let mode = std::env::var(CRASH_MODE_ENV).unwrap();
        let admitted = admit_project_root(&root, std::slice::from_ref(&root)).unwrap();
        if mode == "bootstrap_before_commit" {
            admitted.ensure_bootstrap_directory().unwrap();
            let mut connection = Connection::open(admitted.database_path()).unwrap();
            configure_bootstrap_connection(&connection).unwrap();
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Exclusive)
                .unwrap();
            transaction.execute_batch(SCHEMA_SQL).unwrap();
            transaction
                .execute(
                    "INSERT INTO schema_meta (
                        singleton, schema_version, schema_fingerprint, project_id,
                        canonical_root_digest, project_seq, bootstrap_request_id,
                        bootstrap_payload_hash
                     ) VALUES (1, ?1, ?2, ?3, ?4, 0, ?5, ?6)",
                    params![
                        SCHEMA_VERSION,
                        SCHEMA_FINGERPRINT,
                        Uuid::new_v4().to_string(),
                        admitted.canonical_root_digest(),
                        "crash-bootstrap",
                        "root-payload",
                    ],
                )
                .unwrap();
            // The open EXCLUSIVE transaction is abandoned by the process, not by Drop.
            std::process::exit(81);
        }
        assert_eq!(mode, "bootstrap_after_commit");
        SqliteStore::bootstrap_binding(
            &admitted,
            BootstrapRequest {
                request_id: "crash-bootstrap",
                payload_hash: "root-payload",
            },
        )
        .unwrap();
        // Binding is committed, but post-commit directories and the response are still absent.
        std::process::exit(82);
    }

    #[test]
    fn process_crash_writer() {
        let Some(root) = std::env::var_os(CRASH_ROOT_ENV) else {
            return;
        };
        let root = PathBuf::from(root);
        let project_id = ProjectId::new(std::env::var(CRASH_PROJECT_ENV).unwrap());
        let mode = std::env::var(CRASH_MODE_ENV).unwrap();
        let admitted = admit_project_root(&root, std::slice::from_ref(&root)).unwrap();
        let mut store = SqliteStore::open_bound(admitted, &project_id).unwrap();
        let objective = ObjectiveId::new("objective-crash");
        let transaction = store.begin_write().unwrap();
        let event = append(
            &transaction,
            &objective,
            &HeadBinding {
                expected_project_seq: 0,
                expected_objective_seq: 0,
            },
            "crash-request",
            "crash-payload",
        )
        .unwrap();
        if mode == "after_append" {
            std::process::exit(91);
        }
        transaction
            .replace_objective_projection(&ObjectiveProjectionRow {
                objective_id: objective.clone(),
                project_seq: event.project_seq,
                objective_seq: event.objective_seq,
                is_active: true,
                projection_schema: "mobius.projection.v1".into(),
                projection_bytes: b"crash-projection".to_vec(),
            })
            .unwrap();
        if mode == "after_objective_projection" {
            std::process::exit(92);
        }
        transaction
            .replace_object_projections(
                &objective,
                &[ObjectProjectionRow {
                    objective_id: objective.clone(),
                    object_kind: "stage".into(),
                    object_id: "stage-crash".into(),
                    accepted_project_seq: event.project_seq,
                    projection_schema: "mobius.object-projection.v1".into(),
                    projection_bytes: b"crash-object".to_vec(),
                }],
            )
            .unwrap();

        if mode == "after_object_projection" {
            // process::exit bypasses Rust destructors: SQLite must recover this open transaction.
            std::process::exit(93);
        }
        assert_eq!(mode, "after_commit");
        transaction.commit().unwrap();
        // The commit is durable, but the caller receives no success response.
        std::process::exit(94);
    }

    #[test]
    fn rebuild_process_crash_writer() {
        let Some(root) = std::env::var_os(CRASH_ROOT_ENV) else {
            return;
        };
        let root = PathBuf::from(root);
        let project_id = ProjectId::new(std::env::var(CRASH_PROJECT_ENV).unwrap());
        let mode = std::env::var(CRASH_MODE_ENV).unwrap();
        let admitted = admit_project_root(&root, std::slice::from_ref(&root)).unwrap();
        let mut store = SqliteStore::open_bound(admitted, &project_id).unwrap();
        let transaction = store.begin_write().unwrap();
        transaction.clear_projections().unwrap();
        if mode == "rebuild_after_clear" {
            std::process::exit(71);
        }
        assert_eq!(mode, "rebuild_after_first_objective");
        transaction
            .replace_objective_projection(&ObjectiveProjectionRow {
                objective_id: ObjectiveId::new("objective-1"),
                project_seq: 1,
                objective_seq: 1,
                is_active: false,
                projection_schema: "mobius.projection.v1".into(),
                projection_bytes: b"partial-rebuild".to_vec(),
            })
            .unwrap();
        // The old complete projection set must survive this partial rebuild process loss.
        std::process::exit(72);
    }

    fn run_crash_child(project: &TestProject, binding: &ProjectBinding, mode: &str) -> i32 {
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("infrastructure::sqlite::tests::process_crash_writer")
            .arg("--nocapture")
            .env(CRASH_ROOT_ENV, project.path())
            .env(CRASH_PROJECT_ENV, binding.project_id.as_str())
            .env(CRASH_MODE_ENV, mode)
            .status()
            .unwrap();
        status
            .code()
            .expect("crash test child must exit explicitly")
    }

    fn run_bootstrap_crash_child(project: &TestProject, mode: &str) -> i32 {
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("infrastructure::sqlite::tests::bootstrap_process_crash_writer")
            .arg("--nocapture")
            .env(CRASH_ROOT_ENV, project.path())
            .env(CRASH_MODE_ENV, mode)
            .status()
            .unwrap();
        status
            .code()
            .expect("crash test child must exit explicitly")
    }

    fn run_rebuild_crash_child(project: &TestProject, binding: &ProjectBinding, mode: &str) -> i32 {
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("infrastructure::sqlite::tests::rebuild_process_crash_writer")
            .arg("--nocapture")
            .env(CRASH_ROOT_ENV, project.path())
            .env(CRASH_PROJECT_ENV, binding.project_id.as_str())
            .env(CRASH_MODE_ENV, mode)
            .status()
            .unwrap();
        status
            .code()
            .expect("crash test child must exit explicitly")
    }

    #[test]
    fn process_loss_during_bootstrap_rolls_back_or_reuses_the_binding() {
        let before = TestProject::new();
        assert_eq!(
            run_bootstrap_crash_child(&before, "bootstrap_before_commit"),
            81
        );
        let recovered = before.bootstrap();
        assert_eq!(recovered.project_seq, 0);
        assert!(before.path().join(".mobius/artifacts/blobs").is_dir());

        let after = TestProject::new();
        assert_eq!(
            run_bootstrap_crash_child(&after, "bootstrap_after_commit"),
            82
        );
        assert!(!after.path().join(".mobius/artifacts").exists());
        let admitted = after.admitted();
        let committed = SqliteStore::inspect_binding(&admitted).unwrap();
        let retried = after.bootstrap();
        assert_eq!(retried, committed);
        assert!(after.path().join(".mobius/artifacts/blobs").is_dir());
        assert!(after.path().join(".mobius/views").is_dir());
    }

    #[test]
    fn process_loss_reopens_to_rollback_or_committed_state() {
        let project = TestProject::new();
        let binding = project.bootstrap();
        let objective = ObjectiveId::new("objective-crash");

        for (mode, exit_code) in [
            ("after_append", 91),
            ("after_objective_projection", 92),
            ("after_object_projection", 93),
        ] {
            assert_eq!(run_crash_child(&project, &binding, mode), exit_code);
            let mut store = project.store(&binding);
            let transaction = store.begin_read().unwrap();
            assert_eq!(transaction.project_head().unwrap(), 0);
            assert!(transaction.trail_events(None).unwrap().is_empty());
            assert!(
                transaction
                    .objective_projection(&objective)
                    .unwrap()
                    .is_none()
            );
            assert!(
                transaction
                    .object_projections(&objective)
                    .unwrap()
                    .is_empty()
            );
        }

        assert_eq!(run_crash_child(&project, &binding, "after_commit"), 94);
        let mut store = project.store(&binding);
        let transaction = store.begin_read().unwrap();
        assert_eq!(transaction.project_head().unwrap(), 1);
        assert_eq!(transaction.trail_events(None).unwrap().len(), 1);
        assert!(
            transaction
                .objective_projection(&objective)
                .unwrap()
                .unwrap()
                .is_active
        );
        assert_eq!(transaction.object_projections(&objective).unwrap().len(), 1);
        assert!(transaction.integrity_issues().unwrap().is_empty());
    }

    #[test]
    fn projection_primitives_enforce_single_active_and_rebuild_from_trail() {
        let project = TestProject::new();
        let binding = project.bootstrap();
        let mut store = project.store(&binding);
        let first = ObjectiveId::new("objective-1");
        let second = ObjectiveId::new("objective-2");

        let transaction = store.begin_write().unwrap();
        let first_event = append(
            &transaction,
            &first,
            &HeadBinding {
                expected_project_seq: 0,
                expected_objective_seq: 0,
            },
            "request-1",
            "payload-1",
        )
        .unwrap();
        transaction
            .replace_objective_projection(&ObjectiveProjectionRow {
                objective_id: first.clone(),
                project_seq: first_event.project_seq,
                objective_seq: first_event.objective_seq,
                is_active: true,
                projection_schema: "mobius.projection.v1".into(),
                projection_bytes: b"first".to_vec(),
            })
            .unwrap();
        let second_event = append(
            &transaction,
            &second,
            &HeadBinding {
                expected_project_seq: 1,
                expected_objective_seq: 0,
            },
            "request-2",
            "payload-2",
        )
        .unwrap();
        assert_eq!(
            (second_event.project_seq, second_event.objective_seq),
            (2, 1)
        );
        assert!(matches!(
            transaction.replace_objective_projection(&ObjectiveProjectionRow {
                objective_id: second.clone(),
                project_seq: second_event.project_seq,
                objective_seq: second_event.objective_seq,
                is_active: true,
                projection_schema: "mobius.projection.v1".into(),
                projection_bytes: b"second".to_vec(),
            }),
            Err(StoreError::SingleActiveObjective { .. })
        ));
        transaction.clear_projections().unwrap();
        transaction
            .replace_objective_projection(&ObjectiveProjectionRow {
                objective_id: first.clone(),
                project_seq: 2,
                objective_seq: 1,
                is_active: false,
                projection_schema: "mobius.projection.v1".into(),
                projection_bytes: b"rebuilt".to_vec(),
            })
            .unwrap_err();
        // Rebuild rows remain tied to each stream's own accepted Trail head.
        transaction
            .replace_objective_projection(&ObjectiveProjectionRow {
                objective_id: first.clone(),
                project_seq: first_event.project_seq,
                objective_seq: first_event.objective_seq,
                is_active: false,
                projection_schema: "mobius.projection.v1".into(),
                projection_bytes: b"rebuilt".to_vec(),
            })
            .unwrap();
        transaction
            .replace_object_projections(
                &first,
                &[ObjectProjectionRow {
                    objective_id: first.clone(),
                    object_kind: "stage".into(),
                    object_id: "stage-1".into(),
                    accepted_project_seq: first_event.project_seq,
                    projection_schema: "mobius.object-projection.v1".into(),
                    projection_bytes: b"stage".to_vec(),
                }],
            )
            .unwrap();
        transaction
            .replace_objective_projection(&ObjectiveProjectionRow {
                objective_id: second.clone(),
                project_seq: second_event.project_seq,
                objective_seq: second_event.objective_seq,
                is_active: true,
                projection_schema: "mobius.projection.v1".into(),
                projection_bytes: b"second-rebuilt".to_vec(),
            })
            .unwrap();
        transaction.commit().unwrap();

        let transaction = store.begin_read().unwrap();
        assert_eq!(transaction.project_head().unwrap(), 2);
        assert_eq!(
            transaction.objective_ids().unwrap(),
            vec![first.clone(), second.clone()]
        );
        assert_eq!(
            transaction.active_objective().unwrap(),
            Some(second.clone())
        );
        assert_eq!(
            transaction
                .objective_projection(&first)
                .unwrap()
                .unwrap()
                .projection_bytes,
            b"rebuilt"
        );
        assert!(
            transaction
                .objective_projection(&second)
                .unwrap()
                .unwrap()
                .is_active
        );
        assert_eq!(
            transaction.object_projections(&first).unwrap(),
            vec![ObjectProjectionRow {
                objective_id: first.clone(),
                object_kind: "stage".into(),
                object_id: "stage-1".into(),
                accepted_project_seq: first_event.project_seq,
                projection_schema: "mobius.object-projection.v1".into(),
                projection_bytes: b"stage".to_vec(),
            }]
        );
        assert!(transaction.integrity_issues().unwrap().is_empty());
        drop(transaction);
        drop(store);

        for (mode, exit_code) in [
            ("rebuild_after_clear", 71),
            ("rebuild_after_first_objective", 72),
        ] {
            assert_eq!(run_rebuild_crash_child(&project, &binding, mode), exit_code);
            let mut reopened = project.store(&binding);
            let transaction = reopened.begin_read().unwrap();
            assert_eq!(
                transaction.active_objective().unwrap(),
                Some(second.clone())
            );
            assert_eq!(
                transaction
                    .objective_projection(&first)
                    .unwrap()
                    .unwrap()
                    .projection_bytes,
                b"rebuilt"
            );
            assert!(
                transaction
                    .objective_projection(&second)
                    .unwrap()
                    .unwrap()
                    .is_active
            );
            assert_eq!(transaction.object_projections(&first).unwrap().len(), 1);
        }

        let mut store = project.store(&binding);
        assert!(
            store
                .connection
                .execute(
                    "UPDATE objective_projection SET is_active = 1 WHERE objective_id = ?1",
                    [first.as_str()],
                )
                .is_err(),
            "the partial unique index must remain a mechanical second defense"
        );

        {
            let transaction = store.begin_write().unwrap();
            transaction.clear_projections().unwrap();
            transaction
                .replace_objective_projection(&ObjectiveProjectionRow {
                    objective_id: first.clone(),
                    project_seq: first_event.project_seq,
                    objective_seq: first_event.objective_seq,
                    is_active: true,
                    projection_schema: "mobius.projection.v1".into(),
                    projection_bytes: b"uncommitted-rebuild".to_vec(),
                })
                .unwrap();
            // A failed rebuild drops this transaction and restores both projection tables.
        }
        let transaction = store.begin_read().unwrap();
        assert_eq!(
            transaction.active_objective().unwrap(),
            Some(second.clone())
        );
        assert_eq!(
            transaction
                .objective_projection(&first)
                .unwrap()
                .unwrap()
                .projection_bytes,
            b"rebuilt"
        );
        assert_eq!(transaction.object_projections(&first).unwrap().len(), 1);
    }
}
