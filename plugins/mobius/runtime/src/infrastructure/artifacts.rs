//! Project-owned, content-addressed artifact storage.
//!
//! The store deliberately knows nothing about Trail or Evidence admission.  The
//! application service supplies the Trail-derived reachable set to [`ArtifactStore::gc`]
//! and verifies snapshots again while holding its admission transaction.

use std::collections::BTreeSet;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::domain::{ContentDigest, CoreSnapshot};

const MOBIUS_DIRECTORY: &str = ".mobius";
const ARTIFACTS_DIRECTORY: &str = "artifacts";
const BLOBS_DIRECTORY: &str = "blobs";
const STAGING_DIRECTORY: &str = "staging";
const CAPTURE_PREFIX: &str = "capture-";
const CAPTURE_SUFFIX: &str = ".tmp";

#[derive(Clone, Debug)]
pub(crate) struct ArtifactStore {
    artifacts: PathBuf,
    blobs: PathBuf,
    staging: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ArtifactGcReport {
    pub(crate) removed_blobs: u64,
    pub(crate) removed_staging_files: u64,
    pub(crate) reclaimed_bytes: u64,
}

#[derive(Debug)]
pub(crate) enum ArtifactError {
    InvalidProjectRoot(String),
    MissingManagedDirectory(PathBuf),
    ManagedPathIsSymlink(PathBuf),
    ManagedPathIsNotDirectory(PathBuf),
    InvalidDigest(String),
    InvalidManagedEntry(PathBuf),
    MissingBlob(ContentDigest),
    IntegrityMismatch {
        digest: ContentDigest,
        expected_size: u64,
        actual_size: u64,
        actual_digest: String,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl Display for ArtifactError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProjectRoot(message) => {
                write!(formatter, "invalid project root: {message}")
            }
            Self::MissingManagedDirectory(path) => {
                write!(
                    formatter,
                    "managed artifact directory is missing: {}",
                    path.display()
                )
            }
            Self::ManagedPathIsSymlink(path) => {
                write!(
                    formatter,
                    "managed artifact path is a symlink: {}",
                    path.display()
                )
            }
            Self::ManagedPathIsNotDirectory(path) => write!(
                formatter,
                "managed artifact path is not a directory: {}",
                path.display()
            ),
            Self::InvalidDigest(digest) => write!(formatter, "invalid artifact digest: {digest}"),
            Self::InvalidManagedEntry(path) => write!(
                formatter,
                "unexpected or unsafe artifact-store entry: {}",
                path.display()
            ),
            Self::MissingBlob(digest) => {
                write!(formatter, "artifact blob is missing: {}", digest.0)
            }
            Self::IntegrityMismatch {
                digest,
                expected_size,
                actual_size,
                actual_digest,
            } => write!(
                formatter,
                "artifact integrity mismatch for {}: expected size {expected_size}, got {actual_size}; actual digest {actual_digest}",
                digest.0
            ),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "artifact {operation} failed for {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ArtifactError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl ArtifactStore {
    /// Creates the three managed directories below an already-created `.mobius` directory.
    /// Existing directories are accepted only when they are real directories, never symlinks.
    pub(crate) fn initialize(project_root: &Path) -> Result<Self, ArtifactError> {
        let project_root = canonical_project_root(project_root)?;
        let mobius = project_root.join(MOBIUS_DIRECTORY);
        require_real_directory(&mobius)?;

        let artifacts = mobius.join(ARTIFACTS_DIRECTORY);
        create_or_verify_directory(&artifacts)?;
        let blobs = artifacts.join(BLOBS_DIRECTORY);
        create_or_verify_directory(&blobs)?;
        let staging = artifacts.join(STAGING_DIRECTORY);
        create_or_verify_directory(&staging)?;

        let store = Self {
            artifacts,
            blobs,
            staging,
        };
        store.validate_layout()?;
        Ok(store)
    }

    /// Opens an existing store without silently reconstructing a missing layout.
    pub(crate) fn open(project_root: &Path) -> Result<Self, ArtifactError> {
        let project_root = canonical_project_root(project_root)?;
        let artifacts = project_root
            .join(MOBIUS_DIRECTORY)
            .join(ARTIFACTS_DIRECTORY);
        let store = Self {
            blobs: artifacts.join(BLOBS_DIRECTORY),
            staging: artifacts.join(STAGING_DIRECTORY),
            artifacts,
        };
        store.validate_layout()?;
        Ok(store)
    }

    pub(crate) fn capture(&self, bytes: &[u8]) -> Result<CoreSnapshot, ArtifactError> {
        self.capture_reader(bytes)
    }

    /// Durably captures all bytes before returning a reference that may enter Trail.
    pub(crate) fn capture_reader<R: Read>(
        &self,
        mut reader: R,
    ) -> Result<CoreSnapshot, ArtifactError> {
        self.validate_layout()?;
        let staging_path = self.staging.join(format!(
            "{CAPTURE_PREFIX}{}{CAPTURE_SUFFIX}",
            Uuid::new_v4()
        ));
        let mut staging_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging_path)
            .map_err(|source| io_error("create staging file", &staging_path, source))?;

        let mut hasher = Sha256::new();
        let mut size_bytes = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|source| io_error("read capture input", &staging_path, source))?;
            if read == 0 {
                break;
            }
            staging_file
                .write_all(&buffer[..read])
                .map_err(|source| io_error("write staging file", &staging_path, source))?;
            hasher.update(&buffer[..read]);
            size_bytes = size_bytes.checked_add(read as u64).ok_or_else(|| {
                ArtifactError::InvalidProjectRoot("artifact size exceeds u64".to_owned())
            })?;
        }
        #[cfg(test)]
        artifact_crash_checkpoint("after_staging_write");
        staging_file
            .flush()
            .map_err(|source| io_error("flush staging file", &staging_path, source))?;
        staging_file
            .sync_all()
            .map_err(|source| io_error("sync staging file", &staging_path, source))?;
        #[cfg(test)]
        artifact_crash_checkpoint("after_staging_sync");
        drop(staging_file);

        let digest = ContentDigest(format!(
            "{}{}",
            ContentDigest::SHA256_PREFIX,
            lower_hex(&hasher.finalize())
        ));
        let snapshot = CoreSnapshot {
            digest: digest.clone(),
            size_bytes,
        };
        let target = self.blob_path(&digest)?;

        match fs::symlink_metadata(&target) {
            Ok(_) => {
                self.verify(&snapshot)?;
                remove_regular_file(&staging_path, "remove duplicate staging file")?;
                sync_directory(&self.staging)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // `staging` and `blobs` are siblings on the same project-owned filesystem, so
                // rename is the one atomic publication step.  The application service serializes
                // captures with the project-global writer critical section.
                fs::rename(&staging_path, &target)
                    .map_err(|source| io_error("publish blob", &target, source))?;
                #[cfg(test)]
                artifact_crash_checkpoint("after_rename");
                sync_directory(&self.blobs)?;
                sync_directory(&self.staging)?;
                sync_directory(&self.artifacts)?;
                #[cfg(test)]
                artifact_crash_checkpoint("after_directory_sync");
                self.verify(&snapshot)?;
            }
            Err(source) => return Err(io_error("inspect blob", &target, source)),
        }

        Ok(snapshot)
    }

    /// Reads a snapshot only after recomputing both its digest and size.
    #[cfg(test)]
    pub(crate) fn read(&self, snapshot: &CoreSnapshot) -> Result<Vec<u8>, ArtifactError> {
        self.validate_layout()?;
        let path = self.blob_path(&snapshot.digest)?;
        let metadata = safe_regular_file_metadata(&path, &snapshot.digest)?;
        let capacity = usize::try_from(metadata.len()).map_err(|_| {
            ArtifactError::InvalidProjectRoot(
                "artifact is too large to read on this host".to_owned(),
            )
        })?;
        let mut file = File::open(&path).map_err(|source| io_error("open blob", &path, source))?;
        let mut bytes = Vec::with_capacity(capacity);
        file.read_to_end(&mut bytes)
            .map_err(|source| io_error("read blob", &path, source))?;
        verify_bytes(snapshot, &bytes)?;
        Ok(bytes)
    }

    /// Recomputes the stored bytes; filename presence alone is never an integrity result.
    pub(crate) fn verify(&self, snapshot: &CoreSnapshot) -> Result<(), ArtifactError> {
        self.validate_layout()?;
        let path = self.blob_path(&snapshot.digest)?;
        safe_regular_file_metadata(&path, &snapshot.digest)?;
        let mut file = File::open(&path).map_err(|source| io_error("open blob", &path, source))?;
        verify_open_file(snapshot, &mut file, &path)
    }

    /// Deletes only blobs absent from the supplied Trail-derived reachable set, plus known
    /// capture staging files.  All reachable snapshots are verified before any deletion starts.
    pub(crate) fn gc(
        &self,
        reachable: &BTreeSet<CoreSnapshot>,
    ) -> Result<ArtifactGcReport, ArtifactError> {
        self.validate_layout()?;
        for snapshot in reachable {
            self.verify(snapshot)?;
        }

        let reachable_digests = reachable
            .iter()
            .map(|snapshot| snapshot.digest.0.as_str())
            .collect::<BTreeSet<_>>();
        let mut blob_deletions = Vec::new();
        for entry in read_directory_sorted(&self.blobs)? {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|source| io_error("inspect blob entry", &path, source))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ArtifactError::InvalidManagedEntry(path.clone()))?;
            let digest = ContentDigest(format!("{}{name}", ContentDigest::SHA256_PREFIX));
            if !metadata.is_file() || digest.canonical_sha256_hex().is_none() {
                return Err(ArtifactError::InvalidManagedEntry(path));
            }
            if !reachable_digests.contains(digest.0.as_str()) {
                blob_deletions.push((path, metadata.len()));
            }
        }

        let mut staging_deletions = Vec::new();
        for entry in read_directory_sorted(&self.staging)? {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|source| io_error("inspect staging entry", &path, source))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ArtifactError::InvalidManagedEntry(path.clone()))?;
            if !metadata.is_file()
                || !name.starts_with(CAPTURE_PREFIX)
                || !name.ends_with(CAPTURE_SUFFIX)
            {
                return Err(ArtifactError::InvalidManagedEntry(path));
            }
            staging_deletions.push((path, metadata.len()));
        }

        let mut report = ArtifactGcReport::default();
        for (path, size) in blob_deletions {
            remove_regular_file(&path, "remove unreachable blob")?;
            report.removed_blobs += 1;
            report.reclaimed_bytes = report.reclaimed_bytes.saturating_add(size);
        }
        for (path, size) in staging_deletions {
            remove_regular_file(&path, "remove staging file")?;
            report.removed_staging_files += 1;
            report.reclaimed_bytes = report.reclaimed_bytes.saturating_add(size);
        }
        sync_directory(&self.blobs)?;
        sync_directory(&self.staging)?;
        sync_directory(&self.artifacts)?;
        Ok(report)
    }

    fn blob_path(&self, digest: &ContentDigest) -> Result<PathBuf, ArtifactError> {
        let hex = digest
            .canonical_sha256_hex()
            .ok_or_else(|| ArtifactError::InvalidDigest(digest.0.clone()))?;
        Ok(self.blobs.join(hex))
    }

    fn validate_layout(&self) -> Result<(), ArtifactError> {
        let mobius = self.artifacts.parent().ok_or_else(|| {
            ArtifactError::InvalidProjectRoot("artifact root has no .mobius parent".to_owned())
        })?;
        require_real_directory(mobius)?;
        require_real_directory(&self.artifacts)?;
        require_real_directory(&self.blobs)?;
        require_real_directory(&self.staging)?;
        if self.blobs.parent() != Some(self.artifacts.as_path())
            || self.staging.parent() != Some(self.artifacts.as_path())
        {
            return Err(ArtifactError::InvalidProjectRoot(
                "artifact directories escaped their managed root".to_owned(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
fn artifact_crash_checkpoint(checkpoint: &str) {
    if std::env::var("MOBIUS_ARTIFACT_CRASH_TEST_MODE").as_deref() == Ok(checkpoint) {
        let exit_code = match checkpoint {
            "after_staging_write" => 61,
            "after_staging_sync" => 62,
            "after_rename" => 63,
            "after_directory_sync" => 64,
            _ => 60,
        };
        std::process::exit(exit_code);
    }
}

fn canonical_project_root(path: &Path) -> Result<PathBuf, ArtifactError> {
    let canonical = fs::canonicalize(path).map_err(|source| {
        ArtifactError::InvalidProjectRoot(format!("{}: {source}", path.display()))
    })?;
    let metadata = fs::metadata(&canonical).map_err(|source| {
        ArtifactError::InvalidProjectRoot(format!("{}: {source}", canonical.display()))
    })?;
    if !metadata.is_dir() {
        return Err(ArtifactError::InvalidProjectRoot(format!(
            "{} is not a directory",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn create_or_verify_directory(path: &Path) -> Result<(), ArtifactError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ArtifactError::ManagedPathIsSymlink(path.to_path_buf()))
        }
        Ok(metadata) if !metadata.is_dir() => {
            Err(ArtifactError::ManagedPathIsNotDirectory(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                require_real_directory(path)
            }
            Err(source) => Err(io_error("create managed directory", path, source)),
        },
        Err(source) => Err(io_error("inspect managed directory", path, source)),
    }
}

fn require_real_directory(path: &Path) -> Result<(), ArtifactError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ArtifactError::MissingManagedDirectory(path.to_path_buf()));
        }
        Err(source) => return Err(io_error("inspect managed directory", path, source)),
    };
    if metadata.file_type().is_symlink() {
        return Err(ArtifactError::ManagedPathIsSymlink(path.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(ArtifactError::ManagedPathIsNotDirectory(path.to_path_buf()));
    }
    Ok(())
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

#[cfg(test)]
fn verify_bytes(snapshot: &CoreSnapshot, bytes: &[u8]) -> Result<(), ArtifactError> {
    if snapshot.digest.canonical_sha256_hex().is_none() {
        return Err(ArtifactError::InvalidDigest(snapshot.digest.0.clone()));
    }
    let actual_digest = format!(
        "{}{}",
        ContentDigest::SHA256_PREFIX,
        lower_hex(&Sha256::digest(bytes))
    );
    let actual_size = bytes.len() as u64;
    if actual_size != snapshot.size_bytes || actual_digest != snapshot.digest.0 {
        return Err(ArtifactError::IntegrityMismatch {
            digest: snapshot.digest.clone(),
            expected_size: snapshot.size_bytes,
            actual_size,
            actual_digest,
        });
    }
    Ok(())
}

fn verify_open_file(
    snapshot: &CoreSnapshot,
    file: &mut File,
    path: &Path,
) -> Result<(), ArtifactError> {
    if snapshot.digest.canonical_sha256_hex().is_none() {
        return Err(ArtifactError::InvalidDigest(snapshot.digest.0.clone()));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|source| io_error("seek blob", path, source))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut actual_size = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| io_error("verify blob", path, source))?;
        if read == 0 {
            break;
        }
        actual_size = actual_size
            .checked_add(read as u64)
            .ok_or_else(|| ArtifactError::InvalidProjectRoot("artifact size overflow".into()))?;
        hasher.update(&buffer[..read]);
    }
    let actual_digest = format!(
        "{}{}",
        ContentDigest::SHA256_PREFIX,
        lower_hex(&hasher.finalize())
    );
    if actual_size != snapshot.size_bytes || actual_digest != snapshot.digest.0 {
        return Err(ArtifactError::IntegrityMismatch {
            digest: snapshot.digest.clone(),
            expected_size: snapshot.size_bytes,
            actual_size,
            actual_digest,
        });
    }
    Ok(())
}

fn safe_regular_file_metadata(
    path: &Path,
    digest: &ContentDigest,
) -> Result<fs::Metadata, ArtifactError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ArtifactError::MissingBlob(digest.clone()));
        }
        Err(source) => return Err(io_error("inspect blob", path, source)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ArtifactError::InvalidManagedEntry(path.to_path_buf()));
    }
    Ok(metadata)
}

fn remove_regular_file(path: &Path, operation: &'static str) -> Result<(), ArtifactError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect file before removal", path, source))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ArtifactError::InvalidManagedEntry(path.to_path_buf()));
    }
    fs::remove_file(path).map_err(|source| io_error(operation, path, source))
}

fn read_directory_sorted(path: &Path) -> Result<Vec<fs::DirEntry>, ArtifactError> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| io_error("read managed directory", path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error("read managed directory entry", path, source))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn sync_directory(path: &Path) -> Result<(), ArtifactError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("sync directory", path, source))
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> ArtifactError {
    ArtifactError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    #[cfg(target_os = "linux")]
    use std::process::Command;

    use super::*;

    struct TestProject {
        root: PathBuf,
    }

    impl TestProject {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("mobius-artifact-test-{}", Uuid::new_v4()));
            fs::create_dir(&root).expect("create test project");
            fs::create_dir(root.join(MOBIUS_DIRECTORY)).expect("create .mobius");
            Self { root }
        }
    }

    impl Drop for TestProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn blob_path(project: &TestProject, snapshot: &CoreSnapshot) -> PathBuf {
        project
            .root
            .join(MOBIUS_DIRECTORY)
            .join(ARTIFACTS_DIRECTORY)
            .join(BLOBS_DIRECTORY)
            .join(snapshot.digest.canonical_sha256_hex().unwrap())
    }

    #[test]
    fn capture_publishes_expected_digest_and_reads_only_verified_bytes() {
        let project = TestProject::new();
        let store = ArtifactStore::initialize(&project.root).expect("initialize store");
        let snapshot = store
            .capture_reader(Cursor::new(b"durable evidence"))
            .expect("capture bytes");

        assert_eq!(snapshot.size_bytes, 16);
        assert_eq!(
            snapshot.digest.0,
            "sha256:03f04bd9bde76fc0a793a58f9d09cad7a471b735a6d0f099ed312051df73cef3"
        );
        assert_eq!(store.read(&snapshot).unwrap(), b"durable evidence");
        assert!(
            fs::read_dir(&store.staging)
                .expect("read staging")
                .next()
                .is_none(),
            "successful capture must not leave staging bytes"
        );
    }

    #[test]
    fn duplicate_capture_fully_verifies_existing_blob() {
        let project = TestProject::new();
        let store = ArtifactStore::initialize(&project.root).unwrap();
        let snapshot = store.capture(b"same bytes").unwrap();
        assert_eq!(store.capture(b"same bytes").unwrap(), snapshot);

        fs::write(blob_path(&project, &snapshot), b"corruption").unwrap();
        let error = store.capture(b"same bytes").unwrap_err();
        assert!(matches!(error, ArtifactError::IntegrityMismatch { .. }));
    }

    #[test]
    fn missing_and_corrupt_snapshots_fail_closed() {
        let project = TestProject::new();
        let store = ArtifactStore::initialize(&project.root).unwrap();
        let snapshot = store.capture(b"verified").unwrap();
        let path = blob_path(&project, &snapshot);

        fs::write(&path, b"tampered").unwrap();
        assert!(matches!(
            store.read(&snapshot),
            Err(ArtifactError::IntegrityMismatch { .. })
        ));
        fs::remove_file(&path).unwrap();
        assert!(matches!(
            store.verify(&snapshot),
            Err(ArtifactError::MissingBlob(_))
        ));
    }

    #[test]
    fn gc_retains_reachable_and_removes_only_orphans_and_staging() {
        let project = TestProject::new();
        let store = ArtifactStore::initialize(&project.root).unwrap();
        let reachable = store.capture(b"reachable").unwrap();
        let orphan = store.capture(b"crash orphan").unwrap();
        let staging = store.staging.join(format!(
            "{CAPTURE_PREFIX}{}{CAPTURE_SUFFIX}",
            Uuid::new_v4()
        ));
        fs::write(&staging, b"interrupted capture").unwrap();

        let report = store
            .gc(&BTreeSet::from([reachable.clone()]))
            .expect("collect unreachable bytes");
        assert_eq!(report.removed_blobs, 1);
        assert_eq!(report.removed_staging_files, 1);
        assert_eq!(store.read(&reachable).unwrap(), b"reachable");
        assert!(matches!(
            store.read(&orphan),
            Err(ArtifactError::MissingBlob(_))
        ));
    }

    #[test]
    fn gc_checks_all_reachable_snapshots_before_deleting_anything() {
        let project = TestProject::new();
        let store = ArtifactStore::initialize(&project.root).unwrap();
        let reachable = store.capture(b"reachable").unwrap();
        let orphan = store.capture(b"must remain on failed gc").unwrap();
        fs::remove_file(blob_path(&project, &reachable)).unwrap();

        assert!(matches!(
            store.gc(&BTreeSet::from([reachable])),
            Err(ArtifactError::MissingBlob(_))
        ));
        assert_eq!(store.read(&orphan).unwrap(), b"must remain on failed gc");
    }

    #[cfg(unix)]
    #[test]
    fn managed_symlinks_and_blob_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        let project = TestProject::new();
        let outside = project.root.join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(
            &outside,
            project
                .root
                .join(MOBIUS_DIRECTORY)
                .join(ARTIFACTS_DIRECTORY),
        )
        .unwrap();
        assert!(matches!(
            ArtifactStore::initialize(&project.root),
            Err(ArtifactError::ManagedPathIsSymlink(_))
        ));
    }

    #[test]
    fn digest_parser_rejects_noncanonical_and_path_shaped_values() {
        for invalid in [
            "",
            "sha256:ABCDEF",
            "sha256:../../outside",
            "sha1:0000000000000000000000000000000000000000",
            "sha256:000000000000000000000000000000000000000000000000000000000000000g",
        ] {
            assert!(
                ContentDigest(invalid.to_owned())
                    .canonical_sha256_hex()
                    .is_none(),
                "accepted {invalid}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    const CRASH_ROOT_ENV: &str = "MOBIUS_ARTIFACT_CRASH_TEST_ROOT";
    #[cfg(target_os = "linux")]
    const CRASH_MODE_ENV: &str = "MOBIUS_ARTIFACT_CRASH_TEST_MODE";

    #[cfg(target_os = "linux")]
    #[test]
    fn artifact_process_crash_writer() {
        let Some(root) = std::env::var_os(CRASH_ROOT_ENV) else {
            return;
        };
        let mode = std::env::var(CRASH_MODE_ENV).unwrap();
        let store = ArtifactStore::open(&PathBuf::from(root)).unwrap();
        let snapshot = store.capture(b"durable orphan before Trail").unwrap();
        assert_eq!(mode, "after_capture_before_trail");
        assert_eq!(
            store.read(&snapshot).unwrap(),
            b"durable orphan before Trail"
        );
        std::process::exit(65);
    }

    #[cfg(target_os = "linux")]
    fn run_crash_child(project: &TestProject, mode: &str) -> i32 {
        Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("infrastructure::artifacts::tests::artifact_process_crash_writer")
            .arg("--nocapture")
            .env(CRASH_ROOT_ENV, &project.root)
            .env(CRASH_MODE_ENV, mode)
            .status()
            .expect("artifact crash child must run")
            .code()
            .expect("artifact crash child must exit explicitly")
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_loss_at_capture_boundaries_recovers_to_staging_or_durable_orphan() {
        for (mode, exit_code, published) in [
            ("after_staging_write", 61, false),
            ("after_staging_sync", 62, false),
            ("after_rename", 63, true),
            ("after_directory_sync", 64, true),
            ("after_capture_before_trail", 65, true),
        ] {
            let project = TestProject::new();
            ArtifactStore::initialize(&project.root).unwrap();
            assert_eq!(run_crash_child(&project, mode), exit_code, "{mode}");
            let store = ArtifactStore::open(&project.root).unwrap();
            let blob_count = fs::read_dir(&store.blobs).unwrap().count();
            let staging_count = fs::read_dir(&store.staging).unwrap().count();
            assert_eq!(blob_count, usize::from(published), "{mode}");
            assert_eq!(staging_count, usize::from(!published), "{mode}");

            if published {
                let snapshot = store.capture(b"durable orphan before Trail").unwrap();
                assert_eq!(
                    store.read(&snapshot).unwrap(),
                    b"durable orphan before Trail"
                );
            }
            let report = store.gc(&BTreeSet::new()).unwrap();
            assert_eq!(
                report.removed_blobs + report.removed_staging_files,
                1,
                "{mode}"
            );
            assert_eq!(fs::read_dir(&store.blobs).unwrap().count(), 0, "{mode}");
            assert_eq!(fs::read_dir(&store.staging).unwrap().count(), 0, "{mode}");
        }
    }
}
