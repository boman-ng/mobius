//! Live project-root admission.
//!
//! The host supplies a project root and the workspace roots it is allowed to use. Admission
//! resolves both sides before any managed path is derived. A project root must be exactly one of
//! those canonical workspace roots; a workspace parent does not authorize arbitrary descendants.

use std::fmt::{self, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

const MOBIUS_DIRECTORY: &str = ".mobius";
const DATABASE_FILE: &str = "mobius.sqlite3";
const DATABASE_WAL_FILE: &str = "mobius.sqlite3-wal";
const DATABASE_SHM_FILE: &str = "mobius.sqlite3-shm";
const GITIGNORE_CONTENT: &[u8] = b"*\n";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AdmittedProjectRoot {
    canonical_root: PathBuf,
    mobius_directory: PathBuf,
    database_path: PathBuf,
    canonical_root_digest: String,
}

impl AdmittedProjectRoot {
    pub(crate) fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    pub(crate) fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub(crate) fn canonical_root_digest(&self) -> &str {
        &self.canonical_root_digest
    }

    /// Revalidate the paths that can redirect Core outside the admitted project.
    pub(crate) fn revalidate(&self) -> Result<(), AdmissionError> {
        let current = fs::canonicalize(&self.canonical_root)
            .map_err(|error| AdmissionError::io("canonicalize project root", error))?;
        if current != self.canonical_root {
            return Err(AdmissionError::RootChanged);
        }
        reject_symlink(&self.canonical_root, ManagedPath::ProjectRoot)?;
        validate_existing_managed_paths(self)
    }

    /// Create the private root required to open the project-local database. No other managed
    /// directory is created until the binding transaction has committed.
    pub(crate) fn ensure_bootstrap_directory(&self) -> Result<(), AdmissionError> {
        self.revalidate()?;
        create_managed_directory(&self.mobius_directory, ManagedPath::MobiusDirectory)?;
        sync_directory(&self.canonical_root)?;
        self.revalidate()
    }

    /// Complete the non-business layout after the binding transaction commits.
    ///
    /// Retrying this operation only creates missing directories or the exact ignore file. It never
    /// removes, replaces, or follows an existing entry.
    pub(crate) fn ensure_post_commit_layout(&self) -> Result<(), AdmissionError> {
        self.revalidate()?;
        create_managed_directory(&self.mobius_directory, ManagedPath::MobiusDirectory)?;

        let artifacts = self.mobius_directory.join("artifacts");
        let blobs = artifacts.join("blobs");
        let staging = artifacts.join("staging");
        let views = self.mobius_directory.join("views");
        create_managed_directory(&artifacts, ManagedPath::Artifacts)?;
        create_managed_directory(&blobs, ManagedPath::ArtifactBlobs)?;
        create_managed_directory(&staging, ManagedPath::ArtifactStaging)?;
        create_managed_directory(&views, ManagedPath::Views)?;
        ensure_gitignore(&self.mobius_directory.join(".gitignore"))?;

        sync_directory(&artifacts)?;
        sync_directory(&self.mobius_directory)?;
        sync_directory(&self.canonical_root)?;
        validate_existing_managed_paths(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedPath {
    ProjectRoot,
    MobiusDirectory,
    Database,
    DatabaseWal,
    DatabaseShm,
    Artifacts,
    ArtifactBlobs,
    ArtifactStaging,
    Views,
    GitIgnore,
}

impl Display for ManagedPath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ProjectRoot => "project root",
            Self::MobiusDirectory => ".mobius",
            Self::Database => "Mobius database",
            Self::DatabaseWal => "Mobius database WAL",
            Self::DatabaseShm => "Mobius database shared memory",
            Self::Artifacts => "artifact root",
            Self::ArtifactBlobs => "artifact blob root",
            Self::ArtifactStaging => "artifact staging root",
            Self::Views => "view root",
            Self::GitIgnore => ".mobius/.gitignore",
        })
    }
}

#[derive(Debug)]
pub(crate) enum AdmissionError {
    NoAllowedWorkspaceRoots,
    PathTraversal,
    ProjectRootMissing,
    ProjectRootNotDirectory,
    ProjectRootNotAllowed,
    RootChanged,
    Symlink(ManagedPath),
    WrongKind(ManagedPath),
    InvalidGitIgnore,
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

impl AdmissionError {
    fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }
}

impl Display for AdmissionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAllowedWorkspaceRoots => formatter.write_str("no allowed workspace roots"),
            Self::PathTraversal => formatter.write_str("project root contains path traversal"),
            Self::ProjectRootMissing => formatter.write_str("project root does not exist"),
            Self::ProjectRootNotDirectory => formatter.write_str("project root is not a directory"),
            Self::ProjectRootNotAllowed => formatter.write_str(
                "canonical project root is not exactly one of the allowed workspace roots",
            ),
            Self::RootChanged => {
                formatter.write_str("canonical project root changed after admission")
            }
            Self::Symlink(path) => write!(formatter, "{path} must not be a symlink"),
            Self::WrongKind(path) => write!(formatter, "{path} has an unexpected file type"),
            Self::InvalidGitIgnore => {
                formatter.write_str(".mobius/.gitignore does not contain the Mobius ignore policy")
            }
            Self::Io { operation, source } => write!(formatter, "{operation}: {source}"),
        }
    }
}

impl std::error::Error for AdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) fn admit_project_root(
    requested: &Path,
    allowed_workspace_roots: &[PathBuf],
) -> Result<AdmittedProjectRoot, AdmissionError> {
    if allowed_workspace_roots.is_empty() {
        return Err(AdmissionError::NoAllowedWorkspaceRoots);
    }
    if requested
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(AdmissionError::PathTraversal);
    }

    let requested_metadata = fs::symlink_metadata(requested).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            AdmissionError::ProjectRootMissing
        } else {
            AdmissionError::io("inspect project root", error)
        }
    })?;
    if requested_metadata.file_type().is_symlink() {
        return Err(AdmissionError::Symlink(ManagedPath::ProjectRoot));
    }
    if !requested_metadata.is_dir() {
        return Err(AdmissionError::ProjectRootNotDirectory);
    }

    let canonical_root = fs::canonicalize(requested)
        .map_err(|error| AdmissionError::io("canonicalize project root", error))?;
    let mut allowed = false;
    for candidate in allowed_workspace_roots {
        if candidate
            .components()
            .any(|component| component == Component::ParentDir)
        {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(candidate) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        if fs::canonicalize(candidate).ok().as_ref() == Some(&canonical_root) {
            allowed = true;
            break;
        }
    }
    if !allowed {
        return Err(AdmissionError::ProjectRootNotAllowed);
    }

    let mobius_directory = canonical_root.join(MOBIUS_DIRECTORY);
    let admitted = AdmittedProjectRoot {
        database_path: mobius_directory.join(DATABASE_FILE),
        canonical_root_digest: digest_path(&canonical_root),
        canonical_root,
        mobius_directory,
    };
    validate_existing_managed_paths(&admitted)?;
    Ok(admitted)
}

fn digest_path(path: &Path) -> String {
    let digest = Sha256::digest(path.as_os_str().as_encoded_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn validate_existing_managed_paths(root: &AdmittedProjectRoot) -> Result<(), AdmissionError> {
    validate_if_present(
        &root.mobius_directory,
        ManagedPath::MobiusDirectory,
        ExpectedKind::Directory,
    )?;
    validate_if_present(
        &root.database_path,
        ManagedPath::Database,
        ExpectedKind::File,
    )?;
    validate_if_present(
        &root.mobius_directory.join(DATABASE_WAL_FILE),
        ManagedPath::DatabaseWal,
        ExpectedKind::File,
    )?;
    validate_if_present(
        &root.mobius_directory.join(DATABASE_SHM_FILE),
        ManagedPath::DatabaseShm,
        ExpectedKind::File,
    )?;
    validate_if_present(
        &root.mobius_directory.join("artifacts"),
        ManagedPath::Artifacts,
        ExpectedKind::Directory,
    )?;
    validate_if_present(
        &root.mobius_directory.join("artifacts/blobs"),
        ManagedPath::ArtifactBlobs,
        ExpectedKind::Directory,
    )?;
    validate_if_present(
        &root.mobius_directory.join("artifacts/staging"),
        ManagedPath::ArtifactStaging,
        ExpectedKind::Directory,
    )?;
    validate_if_present(
        &root.mobius_directory.join("views"),
        ManagedPath::Views,
        ExpectedKind::Directory,
    )?;
    validate_if_present(
        &root.mobius_directory.join(".gitignore"),
        ManagedPath::GitIgnore,
        ExpectedKind::File,
    )
}

#[derive(Clone, Copy)]
enum ExpectedKind {
    Directory,
    File,
}

fn validate_if_present(
    path: &Path,
    label: ManagedPath,
    expected: ExpectedKind,
) -> Result<(), AdmissionError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(AdmissionError::io("inspect managed path", error)),
    };
    if metadata.file_type().is_symlink() {
        return Err(AdmissionError::Symlink(label));
    }
    let right_kind = match expected {
        ExpectedKind::Directory => metadata.is_dir(),
        ExpectedKind::File => metadata.is_file(),
    };
    if !right_kind {
        return Err(AdmissionError::WrongKind(label));
    }
    Ok(())
}

fn reject_symlink(path: &Path, label: ManagedPath) -> Result<(), AdmissionError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| AdmissionError::io("inspect admitted path", error))?;
    if metadata.file_type().is_symlink() {
        return Err(AdmissionError::Symlink(label));
    }
    Ok(())
}

fn create_managed_directory(path: &Path, label: ManagedPath) -> Result<(), AdmissionError> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(AdmissionError::io("create managed directory", error)),
    }
    validate_if_present(path, label, ExpectedKind::Directory)
}

fn ensure_gitignore(path: &Path) -> Result<(), AdmissionError> {
    if !path.exists() {
        let parent = path.parent().ok_or_else(|| {
            AdmissionError::io(
                "resolve .mobius/.gitignore parent",
                io::Error::new(io::ErrorKind::InvalidInput, "missing parent"),
            )
        })?;
        let temporary = parent.join(format!(".gitignore-{}.tmp", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| AdmissionError::io("create temporary ignore file", error))?;
        let result = (|| {
            file.write_all(GITIGNORE_CONTENT)
                .map_err(|error| AdmissionError::io("write .mobius/.gitignore", error))?;
            file.sync_all()
                .map_err(|error| AdmissionError::io("sync .mobius/.gitignore", error))?;
            drop(file);
            match fs::hard_link(&temporary, path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                Err(error) => Err(AdmissionError::io("install .mobius/.gitignore", error)),
            }
        })();
        let cleanup = fs::remove_file(&temporary);
        result?;
        if let Err(error) = cleanup {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(AdmissionError::io("remove temporary ignore file", error));
            }
        }
    }
    validate_if_present(path, ManagedPath::GitIgnore, ExpectedKind::File)?;
    let mut content = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut content))
        .map_err(|error| AdmissionError::io("read .mobius/.gitignore", error))?;
    if content != GITIGNORE_CONTENT {
        return Err(AdmissionError::InvalidGitIgnore);
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), AdmissionError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| AdmissionError::io("sync managed directory", error))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), AdmissionError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("mobius-admission-{}", Uuid::new_v4()));
            fs::create_dir(&path).expect("create test project");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_an_exact_allowed_project_root_is_admitted() {
        let workspace = TestDirectory::new();
        let child = workspace.path().join("child");
        fs::create_dir(&child).unwrap();

        let admitted =
            admit_project_root(workspace.path(), &[workspace.path().to_owned()]).unwrap();
        assert_eq!(admitted.canonical_root(), workspace.path());
        assert_eq!(
            admitted.database_path(),
            workspace.path().join(".mobius/mobius.sqlite3")
        );
        assert_eq!(admitted.canonical_root_digest().len(), 64);
        assert!(
            admitted
                .canonical_root_digest()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert!(matches!(
            admit_project_root(&child, &[workspace.path().to_owned()]),
            Err(AdmissionError::ProjectRootNotAllowed)
        ));
        assert!(matches!(
            admit_project_root(
                &workspace.path().join("child/.."),
                &[workspace.path().to_owned()]
            ),
            Err(AdmissionError::PathTraversal)
        ));
    }

    #[test]
    fn post_commit_layout_is_idempotent_and_private() {
        let workspace = TestDirectory::new();
        let admitted =
            admit_project_root(workspace.path(), &[workspace.path().to_owned()]).unwrap();
        fs::create_dir(&admitted.mobius_directory).unwrap();

        admitted.ensure_post_commit_layout().unwrap();
        admitted.ensure_post_commit_layout().unwrap();

        assert!(admitted.mobius_directory.join("artifacts/blobs").is_dir());
        assert!(admitted.mobius_directory.join("artifacts/staging").is_dir());
        assert!(admitted.mobius_directory.join("views").is_dir());
        assert_eq!(
            fs::read(admitted.mobius_directory.join(".gitignore")).unwrap(),
            GITIGNORE_CONTENT
        );
        assert_eq!(
            GITIGNORE_CONTENT, b"*\n",
            "the private policy must ignore itself so repeated default git clean cannot peel it"
        );

        fs::write(admitted.mobius_directory.join("mobius.sqlite3"), b"state").unwrap();
        fs::write(workspace.path().join("ordinary.txt"), b"ordinary").unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(workspace.path())
                .status()
                .unwrap()
                .success()
        );
        let negative_exclude = std::process::Command::new("git")
            .args(["clean", "-nd", "-e", "!*"])
            .current_dir(workspace.path())
            .output()
            .unwrap();
        assert!(negative_exclude.status.success());
        assert!(
            String::from_utf8(negative_exclude.stdout)
                .unwrap()
                .contains("Would remove .mobius/"),
            "a negative clean exclude must remain an explicit Hook risk"
        );
        for _ in 0..2 {
            assert!(
                std::process::Command::new("git")
                    .args(["clean", "-fd"])
                    .current_dir(workspace.path())
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        assert!(admitted.mobius_directory.join(".gitignore").is_file());
        assert!(admitted.mobius_directory.join("mobius.sqlite3").is_file());
        assert!(!workspace.path().join("ordinary.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn project_and_managed_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        let workspace = TestDirectory::new();
        let alias_parent = TestDirectory::new();
        let alias = alias_parent.path().join("project");
        symlink(workspace.path(), &alias).unwrap();
        assert!(matches!(
            admit_project_root(&alias, &[workspace.path().to_owned()]),
            Err(AdmissionError::Symlink(ManagedPath::ProjectRoot))
        ));

        let external = TestDirectory::new();
        symlink(external.path(), workspace.path().join(".mobius")).unwrap();
        assert!(matches!(
            admit_project_root(workspace.path(), &[workspace.path().to_owned()]),
            Err(AdmissionError::Symlink(ManagedPath::MobiusDirectory))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn every_existing_managed_root_or_file_is_checked_without_following_it() {
        use std::os::unix::fs::symlink;

        for relative in [
            "mobius.sqlite3",
            "mobius.sqlite3-wal",
            "mobius.sqlite3-shm",
            "artifacts",
            "artifacts/blobs",
            "artifacts/staging",
            "views",
            ".gitignore",
        ] {
            let workspace = TestDirectory::new();
            let external = TestDirectory::new();
            let managed = workspace.path().join(".mobius");
            fs::create_dir(&managed).unwrap();
            if relative.starts_with("artifacts/") {
                fs::create_dir(managed.join("artifacts")).unwrap();
            }
            let target = if relative.starts_with("mobius.sqlite3") || relative == ".gitignore" {
                let file = external.path().join("external-file");
                fs::write(&file, b"external").unwrap();
                file
            } else {
                external.path().to_owned()
            };
            symlink(target, managed.join(relative)).unwrap();

            assert!(
                matches!(
                    admit_project_root(workspace.path(), &[workspace.path().to_owned()]),
                    Err(AdmissionError::Symlink(_))
                ),
                "managed symlink {relative} must fail closed"
            );
        }
    }

    #[test]
    fn database_family_sidecars_must_be_regular_files_when_present() {
        for (relative, expected_path) in [
            ("mobius.sqlite3-wal", ManagedPath::DatabaseWal),
            ("mobius.sqlite3-shm", ManagedPath::DatabaseShm),
        ] {
            let workspace = TestDirectory::new();
            let managed = workspace.path().join(".mobius");
            fs::create_dir(&managed).unwrap();
            fs::create_dir(managed.join(relative)).unwrap();

            assert!(
                matches!(
                    admit_project_root(workspace.path(), &[workspace.path().to_owned()]),
                    Err(AdmissionError::WrongKind(actual_path)) if actual_path == expected_path
                ),
                "managed database-family path {relative} must be a regular file"
            );
        }
    }
}
