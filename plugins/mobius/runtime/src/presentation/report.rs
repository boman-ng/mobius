//! One-way presentation rendering.
//!
//! The application layer supplies immutable report snapshots, while transport may supply one
//! interaction summary after Core admission. This adapter owns view paths and encoding. CSV is
//! never business input; the summary remains advisory Route-design context.

use std::collections::BTreeSet;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::application::service::{ReportCell, ReportHeads, ReportRows, ReportSnapshot};
use crate::domain::ObjectiveId;

const MOBIUS_DIRECTORY: &str = ".mobius";
const VIEWS_DIRECTORY: &str = "views";
const RUNS_DIRECTORY: &str = "runs";
const INTERACTIONS_DIRECTORY: &str = "interactions";
const GENERATIONS_DIRECTORY: &str = "generations";
const CURRENT_FILE: &str = "current.csv";
const INTERACTION_FILE: &str = "interaction.md";
const INVALID_CURRENT_PREFIX: &str = ".invalid-current-";
const REPORT_SCHEMA: &str = "mobius.report.v1";
const CURRENT_COLUMNS: [&str; 4] = [
    "generation",
    "project_seq",
    "objective_seq",
    "report_schema",
];
const META_COLUMNS: [&str; 2] = ["key", "value"];
const REPORT_FILES: [&str; 9] = [
    "meta.csv",
    "overview.csv",
    "stage-view.csv",
    "criterion-view.csv",
    "route-view.csv",
    "attempt-view.csv",
    "evidence-view.csv",
    "review-view.csv",
    "timeline.csv",
];

/// Host-side presentation input.  It never enters a transition or the format-neutral snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportScope {
    pub(crate) session_ref: String,
    pub(crate) slug: String,
}

/// Agent-authored presentation input. It is removed by transport before Core admission.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct InteractionSummary {
    interpreted_intent: String,
    confirmed_boundaries: String,
    verified_facts: String,
    challenges_and_resolutions: String,
    route_notes: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InteractionAction {
    Activate,
    Revise,
}

impl InteractionAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Activate => "activate",
            Self::Revise => "revise",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CurrentReportState {
    Missing,
    Fresh { generation_relative: String },
    Stale { current_heads: ReportHeads },
    Incomplete { reason: String },
    Invalid { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportPublication {
    pub(crate) generation_path: PathBuf,
    pub(crate) current_path: PathBuf,
    pub(crate) source_heads: ReportHeads,
    pub(crate) previous_state: CurrentReportState,
}

#[derive(Clone, Debug)]
pub(crate) struct ReportRenderer {
    views: PathBuf,
}

#[derive(Debug)]
pub(crate) enum ReportError {
    InvalidProjectRoot(String),
    MissingManagedDirectory(PathBuf),
    ManagedPathIsSymlink(PathBuf),
    ManagedPathIsNotDirectory(PathBuf),
    ManagedPathIsNotFile(PathBuf),
    InvalidPathComponent(String),
    PathEscapedViews(PathBuf),
    InvalidRows(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl Display for ReportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProjectRoot(message) => {
                write!(formatter, "invalid project root: {message}")
            }
            Self::MissingManagedDirectory(path) => {
                write!(
                    formatter,
                    "managed report directory is missing: {}",
                    path.display()
                )
            }
            Self::ManagedPathIsSymlink(path) => {
                write!(
                    formatter,
                    "managed report path is a symlink: {}",
                    path.display()
                )
            }
            Self::ManagedPathIsNotDirectory(path) => write!(
                formatter,
                "managed report path is not a directory: {}",
                path.display()
            ),
            Self::ManagedPathIsNotFile(path) => write!(
                formatter,
                "managed report path is not a regular file: {}",
                path.display()
            ),
            Self::InvalidPathComponent(value) => {
                write!(formatter, "invalid empty report path component: {value:?}")
            }
            Self::PathEscapedViews(path) => {
                write!(
                    formatter,
                    "report path escaped the views root: {}",
                    path.display()
                )
            }
            Self::InvalidRows(message) => {
                write!(formatter, "invalid report snapshot rows: {message}")
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "report {operation} failed for {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ReportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl ReportRenderer {
    pub(crate) fn initialize(project_root: &Path) -> Result<Self, ReportError> {
        let project_root = canonical_project_root(project_root)?;
        let mobius = project_root.join(MOBIUS_DIRECTORY);
        require_real_directory(&mobius)?;
        let views = mobius.join(VIEWS_DIRECTORY);
        create_or_verify_directory(&views)?;
        let renderer = Self { views };
        renderer.validate_root()?;
        Ok(renderer)
    }

    /// Atomically replaces one session-and-revision-local interaction summary. The file is
    /// presentation-only and is never read by this renderer or Core.
    pub(crate) fn write_interaction(
        &self,
        scope: &ReportScope,
        objective: &ObjectiveId,
        revision: u64,
        action: InteractionAction,
        summary: &InteractionSummary,
    ) -> Result<PathBuf, ReportError> {
        self.validate_root()?;
        let session = self.session_path(scope)?;
        let interactions = session.join(INTERACTIONS_DIRECTORY);
        let objective_interactions = interactions.join(format!(
            "{}--{}",
            encode_component(&scope.slug)?,
            objective_id_component(objective)?
        ));
        let revision_interaction = objective_interactions.join(format!("revision-{revision}"));
        ensure_below(&self.views, &revision_interaction)?;

        create_or_verify_directory(&session)?;
        create_or_verify_directory(&interactions)?;
        create_or_verify_directory(&objective_interactions)?;
        create_or_verify_directory(&revision_interaction)?;

        let path = revision_interaction.join(INTERACTION_FILE);
        write_atomic_text(
            &path,
            &render_interaction(objective, revision, action, summary),
        )?;
        Ok(path)
    }

    /// Diagnoses the current pointer against the supplied heads and meta file.  It never reads a
    /// business table back into Core.
    pub(crate) fn assess_current(
        &self,
        scope: &ReportScope,
        snapshot: &ReportSnapshot,
    ) -> Result<CurrentReportState, ReportError> {
        self.validate_root()?;
        validate_snapshot(snapshot)?;
        let run = self.run_path(scope, snapshot)?;
        self.assess_run(&run, snapshot)
    }

    /// Creates the automatic activation report only when this exact run directory does not yet
    /// exist. Existing runs, including missing or unsafe current pointers, are left for explicit
    /// repair. The directory creation is the one-shot claim between concurrent initializers.
    pub(crate) fn initialize_run_if_absent(
        &self,
        scope: &ReportScope,
        snapshot: &ReportSnapshot,
    ) -> Result<(), ReportError> {
        self.validate_root()?;
        validate_snapshot(snapshot)?;
        let (session, runs, run) = self.scoped_run_paths(scope, snapshot)?;
        create_or_verify_directory(&session)?;
        create_or_verify_directory(&runs)?;
        match fs::create_dir(&run) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
            Err(source) => return Err(io_error("create report run", &run, source)),
        }
        create_or_verify_directory(&run.join(GENERATIONS_DIRECTORY))?;
        self.publish(&run, snapshot, CurrentReportState::Missing)?;
        Ok(())
    }

    fn assess_run(
        &self,
        run: &Path,
        snapshot: &ReportSnapshot,
    ) -> Result<CurrentReportState, ReportError> {
        match validate_directory_chain(&self.views, run) {
            Ok(true) => {}
            Ok(false) => return Ok(CurrentReportState::Missing),
            Err(error) => {
                return Ok(CurrentReportState::Invalid {
                    reason: error.to_string(),
                });
            }
        }
        let current = run.join(CURRENT_FILE);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(CurrentReportState::Missing);
            }
            Err(source) => return Err(io_error("inspect current pointer", &current, source)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Ok(CurrentReportState::Invalid {
                reason: "current pointer is not a regular file".to_owned(),
            });
        }

        let current_text = match fs::read_to_string(&current) {
            Ok(input) => input,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return Ok(CurrentReportState::Invalid {
                    reason: "current pointer is not valid UTF-8".to_owned(),
                });
            }
            Err(source) => return Err(io_error("read current pointer", &current, source)),
        };
        let pointer = match parse_current(&current_text) {
            Ok(pointer) => pointer,
            Err(reason) => return Ok(CurrentReportState::Invalid { reason }),
        };
        if pointer.schema != REPORT_SCHEMA {
            return Ok(CurrentReportState::Invalid {
                reason: "current pointer has an unsupported report schema".to_owned(),
            });
        }

        let generation = match safe_generation_path(run, &pointer.generation_relative) {
            Ok(path) => path,
            Err(reason) => return Ok(CurrentReportState::Invalid { reason }),
        };
        match validate_directory_chain(&self.views, &generation) {
            Ok(true) => {}
            Ok(false) => {
                return Ok(CurrentReportState::Incomplete {
                    reason: "current generation directory is missing".to_owned(),
                });
            }
            Err(error) => {
                return Ok(CurrentReportState::Invalid {
                    reason: error.to_string(),
                });
            }
        }
        for filename in REPORT_FILES {
            let path = generation.join(filename);
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(CurrentReportState::Incomplete {
                        reason: format!("current generation is missing {filename}"),
                    });
                }
                Err(source) => return Err(io_error("inspect report file", &path, source)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Ok(CurrentReportState::Incomplete {
                    reason: format!("current generation has unsafe {filename}"),
                });
            }
        }

        for (filename, expected_rows) in report_body_tables(snapshot) {
            let path = generation.join(filename);
            let input = match fs::read_to_string(&path) {
                Ok(input) => input,
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    return Ok(CurrentReportState::Incomplete {
                        reason: format!("{filename} is not valid UTF-8"),
                    });
                }
                Err(source) => return Err(io_error("read report table", &path, source)),
            };
            if let Err(reason) = validate_report_table(&input, expected_rows, filename) {
                return Ok(CurrentReportState::Incomplete { reason });
            }
        }

        let meta_path = generation.join("meta.csv");
        let meta_text = match fs::read_to_string(&meta_path) {
            Ok(input) => input,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return Ok(CurrentReportState::Incomplete {
                    reason: "meta.csv is not valid UTF-8".to_owned(),
                });
            }
            Err(source) => return Err(io_error("read report meta", &meta_path, source)),
        };
        if pointer.heads != snapshot.heads {
            let Some(trail_digest) = snapshot.trail_prefix_digests.get(&pointer.heads) else {
                return Ok(CurrentReportState::Incomplete {
                    reason: "current pointer heads are not an exact historical Trail prefix"
                        .to_owned(),
                });
            };
            return match validate_meta_for_heads(
                &meta_text,
                snapshot.objective_id.as_str(),
                pointer.heads,
                trail_digest,
            ) {
                Ok(()) => Ok(CurrentReportState::Stale {
                    current_heads: pointer.heads,
                }),
                Err(reason) => Ok(CurrentReportState::Incomplete { reason }),
            };
        }
        match validate_meta(&meta_text, snapshot) {
            Ok(()) => Ok(CurrentReportState::Fresh {
                generation_relative: pointer.generation_relative,
            }),
            Err(reason) => Ok(CurrentReportState::Incomplete { reason }),
        }
    }

    /// Creates a fresh generation on every explicit refresh.  All nine files are closed before
    /// the `current.csv` temp file is atomically renamed; concurrent refreshes use last-finisher
    /// pointer semantics and never share mutable generation contents.
    pub(crate) fn refresh(
        &self,
        scope: &ReportScope,
        snapshot: &ReportSnapshot,
    ) -> Result<ReportPublication, ReportError> {
        self.validate_root()?;
        validate_snapshot(snapshot)?;
        let previous_state = self.assess_current(scope, snapshot)?;
        let run = self.ensure_run_path(scope, snapshot)?;
        self.prepare_current_for_explicit_refresh(&run)?;
        self.publish(&run, snapshot, previous_state)
    }

    /// Refreshes only previously complete, Objective-bound runs whose valid current generation is
    /// stale. Missing, invalid, incomplete, and fresh runs remain untouched for explicit repair.
    /// Discovery never creates a session, run, or generations directory, and individual
    /// presentation failures are isolated from other valid stale runs.
    pub(crate) fn refresh_existing_runs(
        &self,
        snapshot: &ReportSnapshot,
    ) -> Result<(), ReportError> {
        self.validate_root()?;
        validate_snapshot(snapshot)?;
        for run in self.existing_run_paths(snapshot)? {
            let previous_state = match self.assess_run(&run, snapshot) {
                Ok(state @ CurrentReportState::Stale { .. }) => state,
                _ => continue,
            };
            if !matches!(validate_directory_chain(&self.views, &run), Ok(true))
                || !matches!(
                    validate_directory_chain(&self.views, &run.join(GENERATIONS_DIRECTORY)),
                    Ok(true)
                )
            {
                continue;
            }
            let _ = self.publish(&run, snapshot, previous_state);
        }
        Ok(())
    }

    fn existing_run_paths(&self, snapshot: &ReportSnapshot) -> Result<Vec<PathBuf>, ReportError> {
        let objective_suffix = format!("--{}", objective_component(snapshot)?);
        let sessions = fs::read_dir(&self.views)
            .map_err(|source| io_error("list report sessions", &self.views, source))?;
        let mut runs_for_objective = Vec::new();

        for session in sessions.flatten() {
            let Some(session_name) = session.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(session_component) = session_name.strip_prefix("codex-session-") else {
                continue;
            };
            if !is_encoded_component(session_component) {
                continue;
            }
            let session = session.path();
            if !matches!(validate_directory_chain(&self.views, &session), Ok(true)) {
                continue;
            }
            let runs = session.join(RUNS_DIRECTORY);
            if !matches!(validate_directory_chain(&self.views, &runs), Ok(true)) {
                continue;
            }
            let Ok(entries) = fs::read_dir(&runs) else {
                continue;
            };
            for entry in entries.flatten() {
                let Some(run_name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let Some(slug_component) = run_name
                    .strip_prefix("run-")
                    .and_then(|name| name.strip_suffix(&objective_suffix))
                else {
                    continue;
                };
                if !is_encoded_component(slug_component) {
                    continue;
                }
                let run = entry.path();
                if matches!(validate_directory_chain(&self.views, &run), Ok(true))
                    && matches!(
                        validate_directory_chain(&self.views, &run.join(GENERATIONS_DIRECTORY)),
                        Ok(true)
                    )
                {
                    runs_for_objective.push(run);
                }
            }
        }

        runs_for_objective.sort();
        Ok(runs_for_objective)
    }

    fn publish(
        &self,
        run: &Path,
        snapshot: &ReportSnapshot,
        previous_state: CurrentReportState,
    ) -> Result<ReportPublication, ReportError> {
        let generations = run.join(GENERATIONS_DIRECTORY);

        let generation_id = Uuid::new_v4().to_string();
        let generation_name = format!("generation-{}", encode_component(&generation_id)?);
        let generation = generations.join(&generation_name);
        ensure_below(&self.views, &generation)?;
        fs::create_dir(&generation)
            .map_err(|source| io_error("create report generation", &generation, source))?;
        #[cfg(test)]
        report_test_crash_checkpoint("after_generation_created");

        self.write_snapshot_files(&generation, snapshot)?;

        let generation_relative = format!("{GENERATIONS_DIRECTORY}/{generation_name}");
        let current_rows = ReportRows::new(
            CURRENT_COLUMNS,
            vec![vec![
                ReportCell::Text(generation_relative),
                ReportCell::Integer(snapshot.heads.project_seq.into()),
                ReportCell::Integer(snapshot.heads.objective_seq.into()),
                ReportCell::Text(REPORT_SCHEMA.to_owned()),
            ]],
        );
        let current_temp = run.join(format!(".current-{}.tmp", Uuid::new_v4()));
        write_table(&current_temp, &current_rows)?;
        #[cfg(test)]
        report_test_crash_checkpoint("after_current_temp_written");

        let current = run.join(CURRENT_FILE);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if !metadata.file_type().is_symlink() && !metadata.is_file() {
                return Err(ReportError::ManagedPathIsNotFile(current));
            }
        }
        fs::rename(&current_temp, &current)
            .map_err(|source| io_error("publish current pointer", &current, source))?;
        #[cfg(test)]
        report_test_crash_checkpoint("after_current_renamed");

        Ok(ReportPublication {
            generation_path: generation,
            current_path: current,
            source_heads: snapshot.heads,
            previous_state,
        })
    }

    fn write_snapshot_files(
        &self,
        generation: &Path,
        snapshot: &ReportSnapshot,
    ) -> Result<(), ReportError> {
        for (filename, rows) in report_body_tables(snapshot) {
            write_table(&generation.join(filename), rows)?;
        }

        // Meta is written last and acts as the generation-complete marker checked by readers.
        write_table(&generation.join("meta.csv"), &meta_rows(snapshot))?;
        #[cfg(test)]
        report_test_crash_checkpoint("after_meta_written");
        Ok(())
    }

    fn prepare_current_for_explicit_refresh(&self, run: &Path) -> Result<(), ReportError> {
        let current = run.join(CURRENT_FILE);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(io_error("inspect current pointer", &current, source)),
        };
        if metadata.is_file() {
            return Ok(());
        }

        let quarantine = run.join(format!("{INVALID_CURRENT_PREFIX}{}", Uuid::new_v4()));
        ensure_below(&self.views, &quarantine)?;
        fs::rename(&current, &quarantine)
            .map_err(|source| io_error("quarantine invalid current pointer", &current, source))
    }

    fn ensure_run_path(
        &self,
        scope: &ReportScope,
        snapshot: &ReportSnapshot,
    ) -> Result<PathBuf, ReportError> {
        let (session, runs, run) = self.scoped_run_paths(scope, snapshot)?;
        create_or_verify_directory(&session)?;
        create_or_verify_directory(&runs)?;
        create_or_verify_directory(&run)?;
        create_or_verify_directory(&run.join(GENERATIONS_DIRECTORY))?;
        Ok(run)
    }

    fn scoped_run_paths(
        &self,
        scope: &ReportScope,
        snapshot: &ReportSnapshot,
    ) -> Result<(PathBuf, PathBuf, PathBuf), ReportError> {
        let session = self.session_path(scope)?;
        let runs = session.join(RUNS_DIRECTORY);
        let run = runs.join(format!(
            "run-{}--{}",
            encode_component(&scope.slug)?,
            objective_component(snapshot)?
        ));
        ensure_below(&self.views, &run)?;
        Ok((session, runs, run))
    }

    fn session_path(&self, scope: &ReportScope) -> Result<PathBuf, ReportError> {
        let session = self.views.join(format!(
            "codex-session-{}",
            encode_component(&scope.session_ref)?
        ));
        ensure_below(&self.views, &session)?;
        Ok(session)
    }

    fn run_path(
        &self,
        scope: &ReportScope,
        snapshot: &ReportSnapshot,
    ) -> Result<PathBuf, ReportError> {
        self.scoped_run_paths(scope, snapshot)
            .map(|(_, _, run)| run)
    }

    fn validate_root(&self) -> Result<(), ReportError> {
        require_real_directory(&self.views)
    }
}

#[derive(Debug)]
struct CurrentPointer {
    generation_relative: String,
    heads: ReportHeads,
    schema: String,
}

fn parse_current(input: &str) -> Result<CurrentPointer, String> {
    let rows = parse_csv(input)?;
    if rows.len() != 2 || rows[0] != CURRENT_COLUMNS {
        return Err("current pointer has an unexpected schema".to_owned());
    }
    let row = &rows[1];
    if row.len() != CURRENT_COLUMNS.len() {
        return Err("current pointer row has an unexpected width".to_owned());
    }
    let project_seq = row[1]
        .parse::<u64>()
        .map_err(|_| "current project head is invalid".to_owned())?;
    let objective_seq = row[2]
        .parse::<u64>()
        .map_err(|_| "current objective head is invalid".to_owned())?;
    Ok(CurrentPointer {
        generation_relative: row[0].clone(),
        heads: ReportHeads {
            project_seq,
            objective_seq,
        },
        schema: row[3].clone(),
    })
}

fn validate_meta(input: &str, snapshot: &ReportSnapshot) -> Result<(), String> {
    validate_meta_for_heads(
        input,
        snapshot.objective_id.as_str(),
        snapshot.heads,
        &snapshot.trail_digest,
    )
}

fn validate_meta_for_heads(
    input: &str,
    objective_id_expected: &str,
    heads: ReportHeads,
    trail_digest_expected: &str,
) -> Result<(), String> {
    let rows = parse_csv(input)?;
    if rows
        .first()
        .is_none_or(|header| !header.iter().map(String::as_str).eq(META_COLUMNS))
    {
        return Err("meta has an unexpected schema".to_owned());
    }
    let mut schema = None;
    let mut objective_id = None;
    let mut project_seq = None;
    let mut objective_seq = None;
    let mut trail_digest = None;
    let mut files = BTreeSet::new();
    for row in rows.iter().skip(1) {
        if row.len() != 2 {
            return Err("meta row has an unexpected width".to_owned());
        }
        match row[0].as_str() {
            "report_schema" if schema.replace(row[1].clone()).is_none() => {}
            "objective_id" if objective_id.replace(row[1].clone()).is_none() => {}
            "project_seq" if project_seq.replace(row[1].clone()).is_none() => {}
            "objective_seq" if objective_seq.replace(row[1].clone()).is_none() => {}
            "trail_digest" if trail_digest.replace(row[1].clone()).is_none() => {}
            "file" if files.insert(row[1].clone()) => {}
            _ => return Err("meta contains a duplicate or unknown key".to_owned()),
        }
    }
    let expected_files = REPORT_FILES
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let rendered_objective_id = neutralize_formula(objective_id_expected);
    let project_seq_expected = heads.project_seq.to_string();
    let objective_seq_expected = heads.objective_seq.to_string();
    if schema.as_deref() != Some(REPORT_SCHEMA)
        || objective_id.as_deref() != Some(rendered_objective_id.as_str())
        || project_seq.as_deref() != Some(project_seq_expected.as_str())
        || objective_seq.as_deref() != Some(objective_seq_expected.as_str())
        || trail_digest.as_deref() != Some(trail_digest_expected)
        || files != expected_files
    {
        return Err(
            "meta does not match the Objective, pointer heads, digest, or complete file list"
                .to_owned(),
        );
    }
    Ok(())
}

fn meta_rows(snapshot: &ReportSnapshot) -> ReportRows {
    let mut rows = vec![
        vec!["report_schema".into(), REPORT_SCHEMA.into()],
        vec![
            "objective_id".into(),
            ReportCell::Text(snapshot.objective_id.as_str().to_owned()),
        ],
        vec![
            "project_seq".into(),
            ReportCell::Integer(snapshot.heads.project_seq.into()),
        ],
        vec![
            "objective_seq".into(),
            ReportCell::Integer(snapshot.heads.objective_seq.into()),
        ],
        vec![
            "trail_digest".into(),
            ReportCell::Text(snapshot.trail_digest.clone()),
        ],
    ];
    rows.extend(REPORT_FILES.map(|filename| vec!["file".into(), filename.into()]));
    ReportRows::new(META_COLUMNS, rows)
}

fn report_body_tables(snapshot: &ReportSnapshot) -> [(&'static str, &ReportRows); 8] {
    [
        ("overview.csv", &snapshot.overview),
        ("stage-view.csv", &snapshot.stages),
        ("criterion-view.csv", &snapshot.criteria),
        ("route-view.csv", &snapshot.routes),
        ("attempt-view.csv", &snapshot.attempts),
        ("evidence-view.csv", &snapshot.evidence),
        ("review-view.csv", &snapshot.reviews),
        ("timeline.csv", &snapshot.timeline),
    ]
}

fn validate_report_table(
    input: &str,
    expected_rows: &ReportRows,
    filename: &str,
) -> Result<(), String> {
    let records = parse_csv(input)?;
    let expected_columns = expected_rows
        .columns
        .iter()
        .map(|column| neutralize_formula(column))
        .collect::<Vec<_>>();
    if records.first() != Some(&expected_columns) {
        return Err(format!("{filename} has an unexpected schema"));
    }
    if let Some((index, row)) = records
        .iter()
        .skip(1)
        .enumerate()
        .find(|(_, row)| row.len() != expected_columns.len())
    {
        return Err(format!(
            "{filename} row {} has width {}, expected {}",
            index + 1,
            row.len(),
            expected_columns.len()
        ));
    }
    Ok(())
}

fn validate_snapshot(snapshot: &ReportSnapshot) -> Result<(), ReportError> {
    if snapshot.objective_id.as_str().is_empty() {
        return Err(ReportError::InvalidRows(
            "Objective identity must not be empty".to_owned(),
        ));
    }
    if snapshot.trail_digest.is_empty() {
        return Err(ReportError::InvalidRows(
            "trail digest must not be empty".to_owned(),
        ));
    }
    if snapshot.trail_prefix_digests.get(&snapshot.heads) != Some(&snapshot.trail_digest) {
        return Err(ReportError::InvalidRows(
            "current heads must identify the exact current Trail prefix digest".to_owned(),
        ));
    }
    for (name, rows) in [
        ("overview", &snapshot.overview),
        ("stages", &snapshot.stages),
        ("criteria", &snapshot.criteria),
        ("routes", &snapshot.routes),
        ("attempts", &snapshot.attempts),
        ("evidence", &snapshot.evidence),
        ("reviews", &snapshot.reviews),
        ("timeline", &snapshot.timeline),
    ] {
        validate_rows(name, rows)?;
    }
    Ok(())
}

fn validate_rows(name: &str, rows: &ReportRows) -> Result<(), ReportError> {
    if rows.columns.is_empty() || rows.columns.iter().any(String::is_empty) {
        return Err(ReportError::InvalidRows(format!(
            "{name} must have non-empty columns"
        )));
    }
    let unique = rows.columns.iter().collect::<BTreeSet<_>>();
    if unique.len() != rows.columns.len() {
        return Err(ReportError::InvalidRows(format!(
            "{name} has duplicate columns"
        )));
    }
    if let Some((index, row)) = rows
        .rows
        .iter()
        .enumerate()
        .find(|(_, row)| row.len() != rows.columns.len())
    {
        return Err(ReportError::InvalidRows(format!(
            "{name} row {index} has width {}, expected {}",
            row.len(),
            rows.columns.len()
        )));
    }
    Ok(())
}

fn render_interaction(
    objective: &ObjectiveId,
    revision: u64,
    action: InteractionAction,
    summary: &InteractionSummary,
) -> String {
    format!(
        "# Mobius Copilot Interaction\n\n\
- Objective: {}\n\
- Revision: {revision}\n\
- Action: {}\n\n\
## Interpreted Intent\n\
{}\n\n\
## Confirmed Boundaries\n\
{}\n\n\
## Verified Facts\n\
{}\n\n\
## Challenges and Resolutions\n\
{}\n\n\
## Route Notes\n\
{}\n",
        objective.as_str(),
        action.as_str(),
        summary.interpreted_intent.trim(),
        summary.confirmed_boundaries.trim(),
        summary.verified_facts.trim(),
        summary.challenges_and_resolutions.trim(),
        summary.route_notes.trim(),
    )
}

fn write_atomic_text(path: &Path, contents: &str) -> Result<(), ReportError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(ReportError::ManagedPathIsSymlink(path.to_path_buf()));
        }
        if !metadata.is_file() {
            return Err(ReportError::ManagedPathIsNotFile(path.to_path_buf()));
        }
    }

    let parent = path
        .parent()
        .ok_or_else(|| ReportError::PathEscapedViews(path.to_path_buf()))?;
    let temporary = parent.join(format!(".interaction-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| io_error("create interaction temp file", &temporary, source))?;
        file.write_all(contents.as_bytes())
            .and_then(|()| file.flush())
            .map_err(|source| io_error("write interaction temp file", &temporary, source))?;
        drop(file);
        fs::rename(&temporary, path)
            .map_err(|source| io_error("publish interaction file", path, source))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_table(path: &Path, rows: &ReportRows) -> Result<(), ReportError> {
    validate_rows(&path.display().to_string(), rows)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| io_error("create CSV file", path, source))?;
    write_csv_record(
        &mut file,
        rows.columns.iter().map(|column| neutralize_formula(column)),
    )
    .map_err(|source| io_error("write CSV header", path, source))?;
    #[cfg(test)]
    if path.file_name().and_then(|name| name.to_str()) == Some("overview.csv") {
        report_test_crash_checkpoint("after_partial_table");
    }
    for row in &rows.rows {
        write_csv_record(
            &mut file,
            row.iter().map(|cell| match cell {
                ReportCell::Empty => String::new(),
                ReportCell::Text(value) => neutralize_formula(value),
                ReportCell::Integer(value) => value.to_string(),
                ReportCell::Boolean(value) => value.to_string(),
            }),
        )
        .map_err(|source| io_error("write CSV row", path, source))?;
    }
    file.flush()
        .map_err(|source| io_error("flush CSV file", path, source))?;
    drop(file);
    Ok(())
}

#[cfg(test)]
fn report_test_crash_checkpoint(checkpoint: &str) {
    if std::env::var("MOBIUS_REPORT_TEST_CRASH_AT").as_deref() == Ok(checkpoint) {
        std::process::exit(86);
    }
}

fn write_csv_record(
    writer: &mut impl Write,
    fields: impl IntoIterator<Item = String>,
) -> io::Result<()> {
    let mut first = true;
    for field in fields {
        if !first {
            writer.write_all(b",")?;
        }
        first = false;
        if field
            .chars()
            .any(|character| matches!(character, ',' | '"' | '\n' | '\r'))
        {
            writer.write_all(b"\"")?;
            writer.write_all(field.replace('"', "\"\"").as_bytes())?;
            writer.write_all(b"\"")?;
        } else {
            writer.write_all(field.as_bytes())?;
        }
    }
    writer.write_all(b"\n")
}

fn neutralize_formula(value: &str) -> String {
    let trigger = value
        .chars()
        .find(|character| !(character.is_whitespace() || character.is_control()));
    if trigger.is_some_and(|character| matches!(character, '=' | '+' | '-' | '@')) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}

fn parse_csv(input: &str) -> Result<Vec<Vec<String>>, String> {
    let mut chars = input.chars().peekable();
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut field_started = false;

    while let Some(character) = chars.next() {
        if quoted {
            if character == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    if chars
                        .peek()
                        .is_some_and(|next| !matches!(next, ',' | '\n' | '\r'))
                    {
                        return Err("CSV has bytes after a closing quote".to_owned());
                    }
                }
            } else {
                field.push(character);
            }
            continue;
        }

        match character {
            '"' if !field_started => {
                quoted = true;
                field_started = true;
            }
            '"' => return Err("CSV has a quote inside an unquoted field".to_owned()),
            ',' => {
                record.push(std::mem::take(&mut field));
                field_started = false;
            }
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
                field_started = false;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
                field_started = false;
            }
            _ => {
                field_started = true;
                field.push(character);
            }
        }
    }
    if quoted {
        return Err("CSV has an unterminated quoted field".to_owned());
    }
    if field_started || !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

fn safe_generation_path(run: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    let components = relative.components().collect::<Vec<_>>();
    if components.len() != 2 || components[0] != Component::Normal(GENERATIONS_DIRECTORY.as_ref()) {
        return Err("current generation path is not a safe two-component relative path".to_owned());
    }
    let Component::Normal(generation) = components[1] else {
        return Err("current generation path has an unsafe component".to_owned());
    };
    let Some(generation) = generation.to_str() else {
        return Err("current generation path is not UTF-8".to_owned());
    };
    if !generation.starts_with("generation-")
        || generation
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'%')))
    {
        return Err("current generation identity is invalid".to_owned());
    }
    Ok(run.join(relative))
}

fn objective_component(snapshot: &ReportSnapshot) -> Result<String, ReportError> {
    objective_id_component(&snapshot.objective_id)
}

fn objective_id_component(objective: &ObjectiveId) -> Result<String, ReportError> {
    let identity = objective.as_str();
    if identity.is_empty() {
        return Err(ReportError::InvalidPathComponent(
            "Objective identity must not be empty".to_owned(),
        ));
    }
    let short = identity.chars().take(12).collect::<String>();
    let digest = Sha256::digest(identity.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(format!("{}-{hex}", encode_component(&short)?))
}

fn encode_component(value: &str) -> Result<String, ReportError> {
    if value.is_empty() {
        return Err(ReportError::InvalidPathComponent(value.to_owned()));
    }
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
            encoded.push(*byte as char);
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    Ok(encoded)
}

fn is_encoded_component(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') => {
                decoded.push(byte);
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let Some(high) = hex_value(bytes[index + 1]) else {
                    return false;
                };
                let Some(low) = hex_value(bytes[index + 2]) else {
                    return false;
                };
                decoded.push((high << 4) | low);
                index += 3;
            }
            _ => return false,
        }
    }
    String::from_utf8(decoded)
        .ok()
        .and_then(|decoded| encode_component(&decoded).ok())
        .is_some_and(|canonical| canonical == value)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn canonical_project_root(path: &Path) -> Result<PathBuf, ReportError> {
    let canonical = fs::canonicalize(path).map_err(|source| {
        ReportError::InvalidProjectRoot(format!("{}: {source}", path.display()))
    })?;
    if !canonical.is_dir() {
        return Err(ReportError::InvalidProjectRoot(format!(
            "{} is not a directory",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn create_or_verify_directory(path: &Path) -> Result<(), ReportError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ReportError::ManagedPathIsSymlink(path.to_path_buf()))
        }
        Ok(metadata) if !metadata.is_dir() => {
            Err(ReportError::ManagedPathIsNotDirectory(path.to_path_buf()))
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

fn require_real_directory(path: &Path) -> Result<(), ReportError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ReportError::MissingManagedDirectory(path.to_path_buf()));
        }
        Err(source) => return Err(io_error("inspect managed directory", path, source)),
    };
    if metadata.file_type().is_symlink() {
        return Err(ReportError::ManagedPathIsSymlink(path.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(ReportError::ManagedPathIsNotDirectory(path.to_path_buf()));
    }
    Ok(())
}

fn validate_directory_chain(root: &Path, target: &Path) -> Result<bool, ReportError> {
    ensure_below(root, target)?;
    require_real_directory(root)?;
    let relative = target
        .strip_prefix(root)
        .map_err(|_| ReportError::PathEscapedViews(target.to_path_buf()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(ReportError::PathEscapedViews(target.to_path_buf()));
        };
        current.push(component);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(source) => return Err(io_error("inspect report directory", &current, source)),
        };
        if metadata.file_type().is_symlink() {
            return Err(ReportError::ManagedPathIsSymlink(current));
        }
        if !metadata.is_dir() {
            return Err(ReportError::ManagedPathIsNotDirectory(current));
        }
    }
    Ok(true)
}

fn ensure_below(root: &Path, path: &Path) -> Result<(), ReportError> {
    if path.starts_with(root) && path != root {
        Ok(())
    } else {
        Err(ReportError::PathEscapedViews(path.to_path_buf()))
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> ReportError {
    ReportError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::process::Command;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use crate::application::service::{CoreService, ProjectInitRequest};
    use rusqlite::Connection;

    use super::*;

    struct TestProject {
        root: PathBuf,
    }

    impl TestProject {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("mobius-report-test-{}", Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join(MOBIUS_DIRECTORY)).unwrap();
            Self { root }
        }
    }

    impl Drop for TestProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn scope() -> ReportScope {
        ReportScope {
            session_ref: "session/../one".to_owned(),
            slug: "objective / safety".to_owned(),
        }
    }

    fn rows(name: &str) -> ReportRows {
        ReportRows::new(
            ["kind", "value"],
            vec![vec![name.into(), format!("{name}, \"value\"\nline").into()]],
        )
    }

    fn snapshot(project_seq: u64, objective_seq: u64) -> ReportSnapshot {
        snapshot_for("objective-123", project_seq, objective_seq)
    }

    fn snapshot_for(objective_id: &str, project_seq: u64, objective_seq: u64) -> ReportSnapshot {
        let trail_prefix_digests = (1..=project_seq)
            .flat_map(|historical_project_seq| {
                (1..=objective_seq.min(historical_project_seq)).map(
                    move |historical_objective_seq| {
                        (
                            ReportHeads {
                                project_seq: historical_project_seq,
                                objective_seq: historical_objective_seq,
                            },
                            format!(
                                "sha256:trail-{historical_project_seq}-{historical_objective_seq}"
                            ),
                        )
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        ReportSnapshot {
            objective_id: crate::domain::ObjectiveId::new(objective_id),
            heads: ReportHeads {
                project_seq,
                objective_seq,
            },
            trail_digest: format!("sha256:trail-{project_seq}-{objective_seq}"),
            trail_prefix_digests,
            overview: rows("overview"),
            stages: rows("stage"),
            criteria: rows("criterion"),
            routes: rows("route"),
            attempts: rows("attempt"),
            evidence: rows("evidence"),
            reviews: rows("review"),
            timeline: rows("timeline"),
        }
    }

    #[test]
    fn report_refresh_crash_child() {
        let Some(project_root) = std::env::var_os("MOBIUS_REPORT_TEST_PROJECT_ROOT") else {
            return;
        };
        assert!(std::env::var_os("MOBIUS_REPORT_TEST_CRASH_AT").is_some());
        let renderer = ReportRenderer::initialize(Path::new(&project_root)).unwrap();
        let _ = renderer.refresh(&scope(), &snapshot(8, 5));
        panic!("configured report crash checkpoint was not reached");
    }

    #[test]
    fn process_loss_at_each_publication_checkpoint_is_recoverable_and_core_inert() {
        const CHILD_TEST: &str = "presentation::report::tests::report_refresh_crash_child";
        let checkpoints = [
            "after_generation_created",
            "after_partial_table",
            "after_meta_written",
            "after_current_temp_written",
            "after_current_renamed",
        ];

        for checkpoint in checkpoints {
            let project = TestProject::new();
            let service = CoreService::new(vec![project.root.clone()]);
            service
                .project_init(ProjectInitRequest {
                    project_root: project.root.clone(),
                    request_id: format!("report-crash-bootstrap-{checkpoint}"),
                })
                .unwrap();
            let read_heads = || {
                Connection::open(project.root.join(".mobius/mobius.sqlite3"))
                    .unwrap()
                    .query_row(
                        "SELECT project_seq FROM schema_meta WHERE singleton = 1",
                        [],
                        |row| row.get::<_, u64>(0),
                    )
                    .unwrap()
            };
            let heads_before = read_heads();

            let renderer = ReportRenderer::initialize(&project.root).unwrap();
            let baseline = snapshot(7, 4);
            renderer.refresh(&scope(), &baseline).unwrap();

            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", CHILD_TEST, "--nocapture"])
                .env("MOBIUS_REPORT_TEST_PROJECT_ROOT", &project.root)
                .env("MOBIUS_REPORT_TEST_CRASH_AT", checkpoint)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(86),
                "checkpoint {checkpoint} did not terminate the child as expected: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(read_heads(), heads_before, "Core changed at {checkpoint}");

            let next = snapshot(8, 5);
            if checkpoint == "after_current_renamed" {
                assert!(matches!(
                    renderer.assess_current(&scope(), &next).unwrap(),
                    CurrentReportState::Fresh { .. }
                ));
            } else {
                assert!(matches!(
                    renderer.assess_current(&scope(), &baseline).unwrap(),
                    CurrentReportState::Fresh { .. }
                ));
                assert!(matches!(
                    renderer.assess_current(&scope(), &next).unwrap(),
                    CurrentReportState::Stale { .. }
                ));
            }

            let rebuilt = renderer.refresh(&scope(), &next).unwrap();
            assert!(rebuilt.generation_path.is_dir());
            assert!(matches!(
                renderer.assess_current(&scope(), &next).unwrap(),
                CurrentReportState::Fresh { .. }
            ));
            assert_eq!(
                read_heads(),
                heads_before,
                "rebuild changed Core at {checkpoint}"
            );
        }
    }

    #[test]
    fn full_objective_identity_keeps_same_slug_runs_isolated() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let first = renderer
            .refresh(&scope(), &snapshot_for("same-prefix-objective-a", 1, 1))
            .unwrap();
        let second = renderer
            .refresh(&scope(), &snapshot_for("same-prefix-objective-b", 2, 1))
            .unwrap();
        assert_ne!(
            first.generation_path.parent().and_then(Path::parent),
            second.generation_path.parent().and_then(Path::parent)
        );
    }

    #[test]
    fn terminal_refresh_updates_only_complete_objective_bound_stale_runs() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let report_scope = |name: &str| ReportScope {
            session_ref: format!("session-{name}"),
            slug: format!("{name}-view"),
        };
        let generation_count = |publication: &ReportPublication| {
            fs::read_dir(
                publication
                    .generation_path
                    .parent()
                    .expect("generation parent"),
            )
            .unwrap()
            .count()
        };

        let stale_scope = report_scope("stale");
        let fresh_scope = report_scope("fresh");
        let missing_scope = report_scope("missing");
        let malformed_scope = report_scope("malformed");
        let malformed_body_scope = report_scope("malformed-body");
        let incomplete_scope = report_scope("incomplete");
        let wrong_objective_scope = report_scope("wrong-objective");
        let invalid_scope = report_scope("invalid");
        let old = snapshot(1, 1);
        let terminal = snapshot(5, 5);
        let stale = renderer.refresh(&stale_scope, &old).unwrap();
        let fresh = renderer.refresh(&fresh_scope, &terminal).unwrap();
        let missing = renderer.refresh(&missing_scope, &old).unwrap();
        let malformed = renderer.refresh(&malformed_scope, &old).unwrap();
        let malformed_body = renderer.refresh(&malformed_body_scope, &old).unwrap();
        let incomplete = renderer.refresh(&incomplete_scope, &old).unwrap();
        let wrong_objective = renderer.refresh(&wrong_objective_scope, &old).unwrap();
        let invalid = renderer.refresh(&invalid_scope, &old).unwrap();

        fs::remove_file(&missing.current_path).unwrap();
        let malformed_bytes = b"not,a,valid,current\n";
        fs::write(&malformed.current_path, malformed_bytes).unwrap();
        let malformed_body_path = malformed_body.generation_path.join("overview.csv");
        let malformed_body_bytes = b"kind,value\n\"unterminated";
        fs::write(&malformed_body_path, malformed_body_bytes).unwrap();
        let malformed_body_current = fs::read(&malformed_body.current_path).unwrap();
        fs::remove_file(incomplete.generation_path.join("timeline.csv")).unwrap();
        let wrong_meta_path = wrong_objective.generation_path.join("meta.csv");
        let wrong_meta = fs::read_to_string(&wrong_meta_path)
            .unwrap()
            .replace("objective_id,objective-123", "objective_id,objective-other");
        fs::write(&wrong_meta_path, &wrong_meta).unwrap();
        fs::remove_file(&invalid.current_path).unwrap();
        fs::create_dir(&invalid.current_path).unwrap();

        assert!(matches!(
            renderer.assess_current(&stale_scope, &terminal).unwrap(),
            CurrentReportState::Stale { .. }
        ));
        assert!(matches!(
            renderer.assess_current(&fresh_scope, &terminal).unwrap(),
            CurrentReportState::Fresh { .. }
        ));
        assert!(matches!(
            renderer.assess_current(&missing_scope, &terminal).unwrap(),
            CurrentReportState::Missing
        ));
        assert!(matches!(
            renderer
                .assess_current(&malformed_scope, &terminal)
                .unwrap(),
            CurrentReportState::Invalid { .. }
        ));
        assert!(matches!(
            renderer
                .assess_current(&malformed_body_scope, &terminal)
                .unwrap(),
            CurrentReportState::Incomplete { .. }
        ));
        assert!(matches!(
            renderer
                .assess_current(&incomplete_scope, &terminal)
                .unwrap(),
            CurrentReportState::Incomplete { .. }
        ));
        assert!(matches!(
            renderer
                .assess_current(&wrong_objective_scope, &terminal)
                .unwrap(),
            CurrentReportState::Incomplete { .. }
        ));
        assert!(matches!(
            renderer.assess_current(&invalid_scope, &terminal).unwrap(),
            CurrentReportState::Invalid { .. }
        ));

        let empty_runs = renderer
            .views
            .join("codex-session-empty-session")
            .join(RUNS_DIRECTORY);
        fs::create_dir_all(&empty_runs).unwrap();
        let incomplete_run = renderer
            .views
            .join("codex-session-incomplete-session")
            .join(RUNS_DIRECTORY)
            .join(format!(
                "run-incomplete--{}",
                objective_component(&terminal).unwrap()
            ));
        fs::create_dir_all(&incomplete_run).unwrap();

        renderer.refresh_existing_runs(&terminal).unwrap();

        assert!(matches!(
            renderer.assess_current(&stale_scope, &terminal).unwrap(),
            CurrentReportState::Fresh { .. }
        ));
        assert!(matches!(
            renderer.assess_current(&fresh_scope, &terminal).unwrap(),
            CurrentReportState::Fresh { .. }
        ));
        assert_eq!(generation_count(&stale), 2);
        assert_eq!(generation_count(&fresh), 1);
        assert!(!missing.current_path.exists());
        assert_eq!(generation_count(&missing), 1);
        assert_eq!(fs::read(&malformed.current_path).unwrap(), malformed_bytes);
        assert_eq!(generation_count(&malformed), 1);
        assert_eq!(
            fs::read(&malformed_body.current_path).unwrap(),
            malformed_body_current
        );
        assert_eq!(
            fs::read(&malformed_body_path).unwrap(),
            malformed_body_bytes
        );
        assert_eq!(generation_count(&malformed_body), 1);
        assert!(!incomplete.generation_path.join("timeline.csv").exists());
        assert_eq!(generation_count(&incomplete), 1);
        assert_eq!(fs::read_to_string(&wrong_meta_path).unwrap(), wrong_meta);
        assert_eq!(generation_count(&wrong_objective), 1);
        assert!(invalid.current_path.is_dir());
        assert_eq!(generation_count(&invalid), 1);
        assert_eq!(fs::read_dir(&empty_runs).unwrap().count(), 0);
        assert!(
            !incomplete_run.join(GENERATIONS_DIRECTORY).exists(),
            "fan-out must not repair or create an incomplete run structure"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let symlink_scope = report_scope("symlink");
            let symlinked = renderer.refresh(&symlink_scope, &old).unwrap();
            let external = project.root.join("external-current.csv");
            let external_bytes = b"external derived view remains untouched";
            fs::write(&external, external_bytes).unwrap();
            fs::remove_file(&symlinked.current_path).unwrap();
            symlink(&external, &symlinked.current_path).unwrap();

            renderer.refresh_existing_runs(&terminal).unwrap();

            assert!(
                fs::symlink_metadata(&symlinked.current_path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(fs::read(&external).unwrap(), external_bytes);
            assert_eq!(generation_count(&symlinked), 1);
        }
    }

    #[test]
    fn terminal_refresh_preserves_unproven_history_until_explicit_repair() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let report_scope = |name: &str| ReportScope {
            session_ref: format!("session-{name}"),
            slug: format!("{name}-view"),
        };
        let generation_count = |publication: &ReportPublication| {
            fs::read_dir(
                publication
                    .generation_path
                    .parent()
                    .expect("generation parent"),
            )
            .unwrap()
            .count()
        };

        let old = snapshot(1, 1);
        let terminal = snapshot(5, 5);
        let tampered_scope = report_scope("tampered-old-digest");
        let future_scope = report_scope("fabricated-future-heads");
        let tampered = renderer.refresh(&tampered_scope, &old).unwrap();
        let future = renderer.refresh(&future_scope, &old).unwrap();

        let tampered_meta_path = tampered.generation_path.join("meta.csv");
        let tampered_meta = fs::read_to_string(&tampered_meta_path).unwrap().replacen(
            "trail_digest,sha256:trail-1-1",
            "trail_digest,sha256:tampered-old-prefix",
            1,
        );
        fs::write(&tampered_meta_path, &tampered_meta).unwrap();
        let tampered_current_bytes = fs::read(&tampered.current_path).unwrap();

        let future_current = fs::read_to_string(&future.current_path).unwrap().replacen(
            ",1,1,mobius.report.v1",
            ",99,99,mobius.report.v1",
            1,
        );
        fs::write(&future.current_path, &future_current).unwrap();
        let future_meta_path = future.generation_path.join("meta.csv");
        let future_meta = fs::read_to_string(&future_meta_path)
            .unwrap()
            .replacen("project_seq,1", "project_seq,99", 1)
            .replacen("objective_seq,1", "objective_seq,99", 1);
        fs::write(&future_meta_path, &future_meta).unwrap();
        let future_current_bytes = fs::read(&future.current_path).unwrap();

        assert!(matches!(
            renderer.assess_current(&tampered_scope, &terminal).unwrap(),
            CurrentReportState::Incomplete { .. }
        ));
        assert!(matches!(
            renderer.assess_current(&future_scope, &terminal).unwrap(),
            CurrentReportState::Incomplete { .. }
        ));

        renderer.refresh_existing_runs(&terminal).unwrap();

        assert_eq!(
            fs::read(&tampered.current_path).unwrap(),
            tampered_current_bytes
        );
        assert_eq!(
            fs::read(&future.current_path).unwrap(),
            future_current_bytes
        );
        assert_eq!(
            fs::read_to_string(&tampered_meta_path).unwrap(),
            tampered_meta
        );
        assert_eq!(fs::read_to_string(&future_meta_path).unwrap(), future_meta);
        assert_eq!(generation_count(&tampered), 1);
        assert_eq!(generation_count(&future), 1);

        renderer.refresh(&tampered_scope, &terminal).unwrap();
        renderer.refresh(&future_scope, &terminal).unwrap();
        assert!(matches!(
            renderer.assess_current(&tampered_scope, &terminal).unwrap(),
            CurrentReportState::Fresh { .. }
        ));
        assert!(matches!(
            renderer.assess_current(&future_scope, &terminal).unwrap(),
            CurrentReportState::Fresh { .. }
        ));
        assert_eq!(generation_count(&tampered), 2);
        assert_eq!(generation_count(&future), 2);
    }

    #[test]
    fn refresh_writes_exactly_nine_complete_files_and_fresh_pointer() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let snapshot = snapshot(7, 4);
        let publication = renderer.refresh(&scope(), &snapshot).unwrap();

        let names = fs::read_dir(&publication.generation_path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(names, REPORT_FILES.map(str::to_owned).into());
        assert!(matches!(
            renderer.assess_current(&scope(), &snapshot).unwrap(),
            CurrentReportState::Fresh { .. }
        ));
    }

    #[test]
    fn every_text_cell_is_formula_neutralized_after_whitespace_or_control_prefixes() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let dangerous = ["=A1", "+A1", "-A1", "@A1", "  =A1", "\t+A1", "\u{0001}-A1"];
        let mut snapshot = snapshot(1, 1);
        snapshot.overview = ReportRows::new(
            dangerous,
            vec![
                dangerous
                    .iter()
                    .map(|value| ReportCell::Text((*value).to_owned()))
                    .collect(),
            ],
        );
        let publication = renderer.refresh(&scope(), &snapshot).unwrap();
        let text = fs::read_to_string(publication.generation_path.join("overview.csv")).unwrap();
        let parsed = parse_csv(&text).unwrap();
        for record in parsed {
            assert!(
                record.iter().all(|field| field.starts_with('\'')),
                "{record:?}"
            );
        }
    }

    #[test]
    fn formula_safe_objective_ids_still_validate_as_fresh_meta() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();

        for objective_id in ["-objective", " =objective"] {
            let snapshot = snapshot_for(objective_id, 1, 1);
            let publication = renderer.refresh(&scope(), &snapshot).unwrap();
            let meta = fs::read_to_string(publication.generation_path.join("meta.csv")).unwrap();
            let rows = parse_csv(&meta).unwrap();
            let objective_row = rows
                .iter()
                .find(|row| row.first().is_some_and(|key| key == "objective_id"))
                .expect("meta contains the Objective identity");
            assert_eq!(objective_row[1], neutralize_formula(objective_id));
            assert!(matches!(
                renderer.assess_current(&scope(), &snapshot).unwrap(),
                CurrentReportState::Fresh { .. }
            ));
        }
    }

    #[test]
    fn path_components_are_encoded_and_contained() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let publication = renderer.refresh(&scope(), &snapshot(1, 1)).unwrap();
        assert!(publication.generation_path.starts_with(&renderer.views));
        let rendered = publication.generation_path.to_string_lossy();
        assert!(rendered.contains("session%2F%2E%2E%2Fone"));
        assert!(rendered.contains("objective%20%2F%20safety"));
        assert!(!rendered.contains("/../"));

        let interaction = renderer
            .write_interaction(
                &scope(),
                &ObjectiveId::new("objective/../interaction"),
                3,
                InteractionAction::Revise,
                &InteractionSummary {
                    interpreted_intent: "intent".to_owned(),
                    confirmed_boundaries: "boundaries".to_owned(),
                    verified_facts: "facts".to_owned(),
                    challenges_and_resolutions: "challenges".to_owned(),
                    route_notes: "notes".to_owned(),
                },
            )
            .unwrap();
        assert!(interaction.starts_with(&renderer.views));
        let interaction_path = interaction.to_string_lossy();
        assert!(interaction_path.contains("session%2F%2E%2E%2Fone"));
        assert!(interaction_path.contains("objective%20%2F%20safety--"));
        assert!(interaction_path.contains("/revision-3/interaction.md"));
        assert!(!interaction_path.contains("/../"));
        let markdown = fs::read_to_string(&interaction).unwrap();
        for section in [
            "# Mobius Copilot Interaction",
            "- Objective: objective/../interaction",
            "- Revision: 3",
            "- Action: revise",
            "## Interpreted Intent",
            "## Confirmed Boundaries",
            "## Verified Facts",
            "## Challenges and Resolutions",
            "## Route Notes",
        ] {
            assert!(markdown.contains(section), "interaction omitted {section}");
        }

        let newer = renderer
            .write_interaction(
                &scope(),
                &ObjectiveId::new("objective/../interaction"),
                4,
                InteractionAction::Revise,
                &InteractionSummary {
                    interpreted_intent: "newer intent".to_owned(),
                    confirmed_boundaries: "boundaries".to_owned(),
                    verified_facts: "facts".to_owned(),
                    challenges_and_resolutions: "challenges".to_owned(),
                    route_notes: "notes".to_owned(),
                },
            )
            .unwrap();
        let newer_markdown = fs::read_to_string(&newer).unwrap();
        let delayed_older = renderer
            .write_interaction(
                &scope(),
                &ObjectiveId::new("objective/../interaction"),
                3,
                InteractionAction::Revise,
                &InteractionSummary {
                    interpreted_intent: "delayed older intent".to_owned(),
                    confirmed_boundaries: "boundaries".to_owned(),
                    verified_facts: "facts".to_owned(),
                    challenges_and_resolutions: "challenges".to_owned(),
                    route_notes: "notes".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(delayed_older, interaction);
        assert_ne!(newer, interaction);
        assert_eq!(fs::read_to_string(newer).unwrap(), newer_markdown);
        assert!(
            fs::read_to_string(delayed_older)
                .unwrap()
                .contains("delayed older intent")
        );
    }

    #[test]
    fn incomplete_or_tampered_current_is_detected_and_explicit_refresh_rebuilds() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let snapshot = snapshot(3, 2);
        let first = renderer.refresh(&scope(), &snapshot).unwrap();
        fs::remove_file(first.generation_path.join("timeline.csv")).unwrap();
        assert!(matches!(
            renderer.assess_current(&scope(), &snapshot).unwrap(),
            CurrentReportState::Incomplete { .. }
        ));

        let rebuilt = renderer.refresh(&scope(), &snapshot).unwrap();
        assert_ne!(rebuilt.generation_path, first.generation_path);
        assert!(matches!(
            renderer.assess_current(&scope(), &snapshot).unwrap(),
            CurrentReportState::Fresh { .. }
        ));

        fs::write(
            &rebuilt.current_path,
            "generation,project_seq\n../../escape,3\n",
        )
        .unwrap();
        assert!(matches!(
            renderer.assess_current(&scope(), &snapshot).unwrap(),
            CurrentReportState::Invalid { .. }
        ));
    }

    #[test]
    fn invalid_utf8_view_files_are_classified_and_explicitly_repairable() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let report_scope = |name: &str| ReportScope {
            session_ref: format!("session-{name}"),
            slug: format!("{name}-view"),
        };
        let snapshot = snapshot(3, 2);
        let current_scope = report_scope("invalid-utf8-current");
        let body_scope = report_scope("invalid-utf8-body");
        let meta_scope = report_scope("invalid-utf8-meta");
        let current = renderer.refresh(&current_scope, &snapshot).unwrap();
        let body = renderer.refresh(&body_scope, &snapshot).unwrap();
        let meta = renderer.refresh(&meta_scope, &snapshot).unwrap();

        fs::write(&current.current_path, [0xff]).unwrap();
        fs::write(body.generation_path.join("overview.csv"), [0xff]).unwrap();
        fs::write(meta.generation_path.join("meta.csv"), [0xff]).unwrap();

        assert!(matches!(
            renderer.assess_current(&current_scope, &snapshot).unwrap(),
            CurrentReportState::Invalid { .. }
        ));
        assert!(matches!(
            renderer.assess_current(&body_scope, &snapshot).unwrap(),
            CurrentReportState::Incomplete { .. }
        ));
        assert!(matches!(
            renderer.assess_current(&meta_scope, &snapshot).unwrap(),
            CurrentReportState::Incomplete { .. }
        ));

        for scope in [&current_scope, &body_scope, &meta_scope] {
            renderer.refresh(scope, &snapshot).unwrap();
            assert!(matches!(
                renderer.assess_current(scope, &snapshot).unwrap(),
                CurrentReportState::Fresh { .. }
            ));
        }
    }

    #[test]
    fn old_heads_are_stale_and_same_heads_still_create_a_new_generation() {
        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let old = snapshot(4, 2);
        let first = renderer.refresh(&scope(), &old).unwrap();
        assert!(matches!(
            renderer.assess_current(&scope(), &snapshot(5, 3)).unwrap(),
            CurrentReportState::Stale { .. }
        ));
        let second = renderer.refresh(&scope(), &old).unwrap();
        assert_ne!(first.generation_path, second.generation_path);
        assert_eq!(
            fs::read(first.generation_path.join("overview.csv")).unwrap(),
            fs::read(second.generation_path.join("overview.csv")).unwrap()
        );
    }

    #[test]
    fn concurrent_refreshes_publish_only_complete_immutable_inputs() {
        let project = TestProject::new();
        let renderer = Arc::new(ReportRenderer::initialize(&project.root).unwrap());
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for snapshot in [snapshot(10, 5), snapshot(11, 6)] {
            let renderer = Arc::clone(&renderer);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                let publication = renderer.refresh(&scope(), &snapshot).unwrap();
                (publication, snapshot)
            }));
        }
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();

        assert_ne!(results[0].0.generation_path, results[1].0.generation_path);
        for (publication, snapshot) in &results {
            assert_eq!(publication.source_heads, snapshot.heads);
            for filename in REPORT_FILES {
                assert!(publication.generation_path.join(filename).is_file());
            }
        }
        let fresh_count = results
            .iter()
            .filter(|(_, snapshot)| {
                matches!(
                    renderer.assess_current(&scope(), snapshot).unwrap(),
                    CurrentReportState::Fresh { .. }
                )
            })
            .count();
        assert_eq!(fresh_count, 1, "the last completed pointer must win");
    }

    #[cfg(unix)]
    #[test]
    fn managed_view_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        let project = TestProject::new();
        let outside = project.root.join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(
            &outside,
            project.root.join(MOBIUS_DIRECTORY).join(VIEWS_DIRECTORY),
        )
        .unwrap();
        assert!(matches!(
            ReportRenderer::initialize(&project.root),
            Err(ReportError::ManagedPathIsSymlink(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn current_assessment_rejects_symlinked_generation_ancestors() {
        use std::os::unix::fs::symlink;

        let project = TestProject::new();
        let renderer = ReportRenderer::initialize(&project.root).unwrap();
        let snapshot = snapshot(2, 1);
        let publication = renderer.refresh(&scope(), &snapshot).unwrap();
        let outside = project.root.join("outside-generation");
        fs::create_dir(&outside).unwrap();
        fs::remove_dir_all(&publication.generation_path).unwrap();
        symlink(&outside, &publication.generation_path).unwrap();

        assert!(matches!(
            renderer.assess_current(&scope(), &snapshot).unwrap(),
            CurrentReportState::Invalid { .. }
        ));
    }

    #[test]
    fn csv_codec_quotes_utf8_commas_quotes_and_newlines_stably() {
        let mut output = Vec::new();
        write_csv_record(
            &mut output,
            ["plain", "逗号,", "quote\"", "two\nlines"].map(str::to_owned),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            parse_csv(&output).unwrap(),
            vec![vec!["plain", "逗号,", "quote\"", "two\nlines"]]
        );
    }
}
