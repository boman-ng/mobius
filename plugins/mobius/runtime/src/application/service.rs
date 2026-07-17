//! The single application-service boundary shared by every Mobius adapter.
//!
//! The DTOs in this module are deliberately domain-typed.  Transport adapters may serialize
//! them, but they may not replace them with prose commands, target states, SQL, or patches.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};

use crate::application::admission::{AdmittedProjectRoot, admit_project_root};
use crate::application::commands::ApplyTransitionRequest;
use crate::application::commands::{MutationCommand, SealAttemptCommand};
use crate::domain::{
    CarryVerdict, CoreSnapshot, DomainConfiguration, Evidence, EvidenceId, FirstClassObject,
    FrozenObservation, HeadBinding, LifecycleProjection, NavState, ObjectIdentity, ObjectiveId,
    ObjectiveSpecId, ObjectiveState, ProjectId, ReviewDecisionId, ReviewPacket, ReviewPacketId,
    SealAttemptInput, StageId, TRAIL_EVENT_SCHEMA, TrailFact, TransitionInput, TransitionKind,
    audit_invariants, current_proofs, decode_trail_fact, dependency_view, encode_canonical,
    encode_trail_fact, evidence_universe, initial_configuration, reduce, replay,
};
use crate::infrastructure::artifacts::{ArtifactError, ArtifactStore};
use crate::infrastructure::sqlite::{
    AppendEvent, BootstrapRequest, EventMetadata, EventRow, ObjectProjectionRow,
    ObjectiveProjectionRow, ReadTransaction, SqliteStore, StoreError, WriteTransaction,
};

const OBJECTIVE_PROJECTION_SCHEMA: &str = "mobius.objective-projection.v1";
const OBJECT_PROJECTION_SCHEMA: &str = "mobius.object-projection.v1";
pub const DEFAULT_AUDIT_ISSUE_LIMIT: u32 = 20;
pub const MAX_AUDIT_ISSUE_LIMIT: u32 = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectInitRequest {
    pub project_root: PathBuf,
    pub request_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectBinding {
    pub project_root: PathBuf,
    pub project_id: ProjectId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectInitResponse {
    pub project_id: ProjectId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureArtifactRequest {
    pub binding: ProjectBinding,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyTransitionResponse {
    pub objective_id: ObjectiveId,
    pub transition: TransitionKind,
    pub committed_project_seq: u64,
    pub committed_objective_seq: u64,
    pub event_digest: String,
}

pub(crate) struct ApplyTransitionOutcome {
    pub(crate) response: ApplyTransitionResponse,
    pub(crate) newly_committed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceAction {
    RebuildProjection,
    ArtifactGc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceRequest {
    pub action: MaintenanceAction,
    pub expected_project_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditRequest {
    pub binding: ProjectBinding,
    pub maintenance: Option<MaintenanceRequest>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    Healthy,
    Degraded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditIssue {
    pub code: String,
    pub objective_id: Option<ObjectiveId>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditIssuePage {
    pub returned: u32,
    pub total: u64,
    pub complete: bool,
    pub items: Vec<AuditIssue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditResponse {
    pub status: AuditStatus,
    pub project_seq: u64,
    pub checked_objectives: usize,
    pub issues: AuditIssuePage,
    pub maintenance_applied: Option<MaintenanceAction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoctorResponse {
    pub project_id: ProjectId,
    pub project_seq: u64,
    pub healthy: bool,
    pub issues: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ReportHeads {
    pub(crate) project_seq: u64,
    pub(crate) objective_seq: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReportCell {
    Empty,
    Text(String),
    Integer(i128),
    Boolean(bool),
}

impl From<&str> for ReportCell {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for ReportCell {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportRows {
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<Vec<ReportCell>>,
}

impl ReportRows {
    pub(crate) fn new(
        columns: impl IntoIterator<Item = impl Into<String>>,
        rows: Vec<Vec<ReportCell>>,
    ) -> Self {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
            rows,
        }
    }
}

/// One read-transaction result. Presentation paths and CSV concepts stay outside this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportSnapshot {
    pub(crate) objective_id: ObjectiveId,
    pub(crate) heads: ReportHeads,
    pub(crate) trail_digest: String,
    pub(crate) trail_prefix_digests: BTreeMap<ReportHeads, String>,
    pub(crate) overview: ReportRows,
    pub(crate) stages: ReportRows,
    pub(crate) criteria: ReportRows,
    pub(crate) routes: ReportRows,
    pub(crate) attempts: ReportRows,
    pub(crate) evidence: ReportRows,
    pub(crate) reviews: ReportRows,
    pub(crate) timeline: ReportRows,
}

/// All adapters hold the same service value.  Its concrete store and artifact coordination are
/// implemented below these DTOs once project admission has produced a canonical allowed root.
#[derive(Clone, Debug)]
pub struct CoreService {
    allowed_workspace_roots: Vec<PathBuf>,
}

impl CoreService {
    pub fn new(allowed_workspace_roots: Vec<PathBuf>) -> Self {
        Self {
            allowed_workspace_roots,
        }
    }

    pub fn project_init(
        &self,
        request: ProjectInitRequest,
    ) -> Result<ProjectInitResponse, ServiceError> {
        let admitted = self.admit(&request.project_root)?;
        let payload_hash = sha256_text(admitted.canonical_root_digest().as_bytes());
        let binding = SqliteStore::bootstrap(
            &admitted,
            BootstrapRequest {
                request_id: &request.request_id,
                payload_hash: &payload_hash,
            },
        )
        .map_err(store_error)?;
        ArtifactStore::initialize(admitted.canonical_root()).map_err(artifact_error)?;
        Ok(ProjectInitResponse {
            project_id: binding.project_id,
        })
    }

    pub(crate) fn objective_state(
        &self,
        binding: &ProjectBinding,
        objective: &ObjectiveId,
    ) -> Result<Option<ObjectiveState>, ServiceError> {
        let (_admitted, mut store) = self.open_bound(binding)?;
        let transaction = store.begin_read().map_err(store_error)?;
        let active = transaction.active_objective().map_err(store_error)?;
        let state = load_objective_projection_document(&transaction, objective, active.as_ref())?
            .map(|(document, _)| document.objective_state);
        transaction.commit().map_err(store_error)?;
        Ok(state)
    }

    pub fn capture_artifact(
        &self,
        request: CaptureArtifactRequest,
    ) -> Result<CoreSnapshot, ServiceError> {
        let (admitted, mut store) = self.open_bound(&request.binding)?;
        let artifact_store =
            ArtifactStore::open(admitted.canonical_root()).map_err(artifact_error)?;
        let transaction = store.begin_write().map_err(store_error)?;
        let snapshot = artifact_store
            .capture(&request.bytes)
            .map_err(artifact_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(snapshot)
    }

    pub(crate) fn apply_transition(
        &self,
        request: ApplyTransitionRequest,
    ) -> Result<ApplyTransitionOutcome, ServiceError> {
        let admitted = self.admit(&request.project_root)?;
        let mut store =
            SqliteStore::open_bound(admitted.clone(), &request.project_id).map_err(store_error)?;
        let payload_hash = sha256_text(
            &encode_canonical(&MutationPayloadRef {
                project_id: &request.project_id,
                expected_heads: &request.expected_heads,
                command: &request.command,
            })
            .map_err(codec_error)?,
        );
        let transaction = store.begin_write().map_err(store_error)?;

        if let Some(existing) = transaction
            .lookup_request(&request.request_id, &payload_hash)
            .map_err(store_error)?
        {
            let response = existing_response(&transaction, &existing)?;
            transaction.commit().map_err(store_error)?;
            return Ok(ApplyTransitionOutcome {
                response,
                newly_committed: false,
            });
        }

        let indexed_active = transaction.active_objective().map_err(store_error)?;
        let objective = command_objective(&request.command, indexed_active.as_ref())?;
        transaction
            .check_heads(&objective, &request.expected_heads)
            .map_err(store_error)?;
        validate_global_trail_write(&transaction)?;

        let project = load_project_write(&transaction, Some(&objective))?;
        if request.command.kind() == TransitionKind::ActivateObjective && project.active.is_some() {
            return Err(ServiceError::new(
                "active_objective_exists",
                "another Objective is already active in this project",
            ));
        }
        let before = project
            .objectives
            .get(&objective)
            .map(|value| value.configuration.clone())
            .unwrap_or_else(initial_configuration);
        validate_live_confirmation(
            &request.command,
            &request.project_id,
            &request.expected_heads,
        )?;

        let transition = match request.command.clone().into_direct_transition() {
            Ok(input) => input,
            Err(seal) => TransitionInput::SealAttempt(materialize_seal(&before, &seal)?),
        };
        let material_snapshots = transition_material_snapshots(&before, &transition)?;
        if !material_snapshots.is_empty() {
            let artifact_store =
                ArtifactStore::open(admitted.canonical_root()).map_err(artifact_error)?;
            for snapshot in material_snapshots {
                artifact_store.verify(&snapshot).map_err(artifact_error)?;
            }
        }
        crate::domain::validate_transition(&before, &transition)
            .map_err(|error| ServiceError::new("transition_rejected", error.to_string()))?;

        let fact = TrailFact {
            objective: objective.clone(),
            input: transition,
        };
        let event_bytes = encode_trail_fact(&fact).map_err(codec_error)?;
        let received_at = received_at();
        let metadata = transaction
            .append_event(AppendEvent {
                objective_id: &objective,
                expected_heads: &request.expected_heads,
                request_id: &request.request_id,
                request_payload_hash: &payload_hash,
                event_schema: TRAIL_EVENT_SCHEMA,
                event_bytes: &event_bytes,
                received_at: &received_at,
            })
            .map_err(store_error)?;

        let after = reduce(&before, &fact.input)
            .map_err(|error| ServiceError::new("transition_rejected", error.to_string()))?;
        audit_invariants(&after).map_err(|violations| {
            ServiceError::new(
                "invariant_violation",
                format!("reduced state violates invariants: {violations:?}"),
            )
        })?;
        let prior_rows = project
            .objectives
            .get(&objective)
            .map(|value| value.object_rows.as_slice())
            .unwrap_or(&[]);
        let (objective_row, object_rows) =
            projection_rows_after_append(&objective, &after, &metadata, prior_rows)?;
        transaction
            .replace_objective_projection(&objective_row)
            .map_err(store_error)?;
        transaction
            .replace_object_projections(&objective, &object_rows)
            .map_err(store_error)?;
        transaction.commit().map_err(store_error)?;

        Ok(ApplyTransitionOutcome {
            response: ApplyTransitionResponse {
                objective_id: objective,
                transition: fact.transition(),
                committed_project_seq: metadata.project_seq,
                committed_objective_seq: metadata.objective_seq,
                event_digest: sha256_text(&event_bytes),
            },
            newly_committed: true,
        })
    }

    pub fn audit(&self, request: AuditRequest) -> Result<AuditResponse, ServiceError> {
        match request.maintenance {
            None => self.audit_read_only(&request.binding, request.limit),
            Some(maintenance) => {
                self.audit_maintenance(&request.binding, maintenance, request.limit)
            }
        }
    }

    pub fn doctor(&self, project_root: PathBuf) -> Result<DoctorResponse, ServiceError> {
        let admitted = self.admit(&project_root)?;
        let binding = SqliteStore::inspect_binding(&admitted).map_err(store_error)?;
        let mut store =
            SqliteStore::open_bound(admitted.clone(), &binding.project_id).map_err(store_error)?;
        let transaction = store.begin_read().map_err(store_error)?;
        let mut issues = transaction
            .integrity_issues()
            .map_err(store_error)?
            .into_iter()
            .map(|issue| issue.to_string())
            .collect::<Vec<_>>();
        if let Err(error) = read_validated_global_trail(&transaction) {
            issues.push(error.to_string());
        }
        if let Err(error) = ArtifactStore::open(admitted.canonical_root()) {
            issues.push(error.to_string());
        }
        let views = admitted.canonical_root().join(".mobius/views");
        match std::fs::symlink_metadata(&views) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => issues.push("managed report root is not a real directory".to_owned()),
            Err(error) => issues.push(format!("managed report root is unavailable: {error}")),
        }
        let project_seq = transaction.project_head().map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(DoctorResponse {
            project_id: binding.project_id,
            project_seq,
            healthy: issues.is_empty(),
            issues,
        })
    }

    pub(crate) fn report_snapshot(
        &self,
        binding: &ProjectBinding,
        objective: &ObjectiveId,
    ) -> Result<ReportSnapshot, ServiceError> {
        let (_admitted, mut store) = self.open_bound(binding)?;
        let transaction = store.begin_read().map_err(store_error)?;
        let all_rows = read_validated_global_trail(&transaction)?;
        let heads = transaction.heads(objective).map_err(store_error)?;
        if heads.objective_seq == 0 {
            return Err(ServiceError::new(
                "objective_not_found",
                "the requested Objective does not have a Trail stream",
            ));
        }
        let rows = all_rows
            .iter()
            .filter(|row| &row.metadata.objective_id == objective)
            .cloned()
            .collect::<Vec<_>>();
        let replayed = replay_event_rows(objective, rows.clone())?;
        compare_projection(
            &replayed,
            transaction
                .objective_projection(objective)
                .map_err(store_error)?,
            transaction
                .object_projections(objective)
                .map_err(store_error)?,
        )?;
        let snapshot = build_report_snapshot(&replayed.configuration, &rows, &all_rows, &heads)?;
        transaction.commit().map_err(store_error)?;
        Ok(snapshot)
    }

    fn admit(&self, project_root: &std::path::Path) -> Result<AdmittedProjectRoot, ServiceError> {
        admit_project_root(project_root, &self.allowed_workspace_roots)
            .map_err(|error| ServiceError::new("project_admission_failed", error.to_string()))
    }

    fn open_bound(
        &self,
        binding: &ProjectBinding,
    ) -> Result<(AdmittedProjectRoot, SqliteStore), ServiceError> {
        let admitted = self.admit(&binding.project_root)?;
        let store =
            SqliteStore::open_bound(admitted.clone(), &binding.project_id).map_err(store_error)?;
        Ok((admitted, store))
    }

    fn audit_read_only(
        &self,
        binding: &ProjectBinding,
        limit: Option<u32>,
    ) -> Result<AuditResponse, ServiceError> {
        let (admitted, mut store) = self.open_bound(binding)?;
        let artifact_store = ArtifactStore::open(admitted.canonical_root());
        let transaction = store.begin_read().map_err(store_error)?;
        let project_seq = transaction.project_head().map_err(store_error)?;
        let objective_ids = transaction.objective_ids().map_err(store_error)?;
        let mut issues = Vec::new();
        if let Err(error) = read_validated_global_trail(&transaction) {
            issues.push(audit_issue(error.code, None, error.message));
        }
        for detail in transaction.integrity_issues().map_err(store_error)? {
            issues.push(audit_issue(
                "sqlite_integrity_failed",
                None,
                detail.to_string(),
            ));
        }
        let mut replayed = BTreeMap::new();

        for objective in &objective_ids {
            let rows = transaction
                .trail_events(Some(objective))
                .map_err(store_error)?;
            match replay_event_rows(objective, rows) {
                Ok(value) => {
                    if let Err(error) = compare_projection(
                        &value,
                        transaction
                            .objective_projection(objective)
                            .map_err(store_error)?,
                        transaction
                            .object_projections(objective)
                            .map_err(store_error)?,
                    ) {
                        issues.push(audit_issue(
                            "projection_mismatch",
                            Some(objective.clone()),
                            error.message,
                        ));
                    }
                    if let Err(violations) = audit_invariants(&value.configuration) {
                        issues.push(audit_issue(
                            "invariant_violation",
                            Some(objective.clone()),
                            format!("{violations:?}"),
                        ));
                    }
                    replayed.insert(objective.clone(), value);
                }
                Err(error) => issues.push(audit_issue(
                    error.code,
                    Some(objective.clone()),
                    error.message,
                )),
            }
        }

        let active = replayed
            .iter()
            .filter(|(_, value)| is_active(value.configuration.objective_state()))
            .map(|(objective, _)| objective.clone())
            .collect::<Vec<_>>();
        if active.len() > 1 {
            issues.push(audit_issue(
                "single_active_violation",
                None,
                "Trail replay produced more than one active Objective",
            ));
        } else if replayed.len() == objective_ids.len()
            && active.first()
                != transaction
                    .active_objective()
                    .map_err(store_error)?
                    .as_ref()
        {
            issues.push(audit_issue(
                "projection_mismatch",
                None,
                "the indexed active Objective does not match Trail replay",
            ));
        }

        match artifact_store {
            Ok(artifacts) => {
                for (objective, value) in &replayed {
                    for snapshot in referenced_snapshots(&value.configuration) {
                        if let Err(error) = artifacts.verify(&snapshot) {
                            issues.push(audit_issue(
                                "artifact_integrity_failed",
                                Some(objective.clone()),
                                error.to_string(),
                            ));
                        }
                    }
                }
            }
            Err(error) => issues.push(audit_issue(
                "artifact_store_unavailable",
                None,
                error.to_string(),
            )),
        }
        transaction.commit().map_err(store_error)?;
        let status = audit_status(&issues);
        let issues = audit_issue_page(issues, limit)?;
        Ok(AuditResponse {
            status,
            project_seq,
            checked_objectives: objective_ids.len(),
            issues,
            maintenance_applied: None,
        })
    }

    fn audit_maintenance(
        &self,
        binding: &ProjectBinding,
        maintenance: MaintenanceRequest,
        limit: Option<u32>,
    ) -> Result<AuditResponse, ServiceError> {
        let (admitted, mut store) = self.open_bound(binding)?;
        let transaction = store.begin_write().map_err(store_error)?;
        let project_seq = transaction.project_head().map_err(store_error)?;
        if project_seq != maintenance.expected_project_seq {
            return Err(ServiceError::new(
                "stale_heads",
                format!(
                    "stale project head: expected {}, found {project_seq}",
                    maintenance.expected_project_seq
                ),
            ));
        }
        validate_global_trail_write(&transaction)?;
        let integrity_issues = transaction.integrity_issues().map_err(store_error)?;
        let fatal_integrity_issues = integrity_issues
            .iter()
            .filter(|issue| {
                maintenance.action != MaintenanceAction::RebuildProjection
                    || !issue.is_projection_foreign_key_violation()
            })
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !fatal_integrity_issues.is_empty() {
            return Err(ServiceError::new(
                "sqlite_integrity_failed",
                fatal_integrity_issues.join("; "),
            ));
        }

        let objective_ids = transaction.objective_ids().map_err(store_error)?;
        let mut replayed = BTreeMap::new();
        let mut reachable = BTreeSet::new();
        for objective in &objective_ids {
            let value = replay_event_rows(
                objective,
                transaction
                    .trail_events(Some(objective))
                    .map_err(store_error)?,
            )?;
            audit_invariants(&value.configuration).map_err(|violations| {
                ServiceError::new(
                    "invariant_violation",
                    format!("Trail replay violates invariants: {violations:?}"),
                )
            })?;
            reachable.extend(referenced_snapshots(&value.configuration));
            replayed.insert(objective.clone(), value);
        }
        if replayed
            .values()
            .filter(|value| is_active(value.configuration.objective_state()))
            .count()
            > 1
        {
            return Err(ServiceError::new(
                "single_active_violation",
                "Trail replay produced more than one active Objective",
            ));
        }
        let mut issues = Vec::new();
        match maintenance.action {
            MaintenanceAction::RebuildProjection => {
                transaction.clear_projections().map_err(store_error)?;
                for (objective, value) in &replayed {
                    let row = value.objective_row.as_ref().ok_or_else(|| {
                        ServiceError::new(
                            "invalid_trail",
                            "Objective stream has no projection head",
                        )
                    })?;
                    transaction
                        .replace_objective_projection(row)
                        .map_err(store_error)?;
                    transaction
                        .replace_object_projections(objective, &value.object_rows)
                        .map_err(store_error)?;
                }
                let remaining_integrity = transaction.integrity_issues().map_err(store_error)?;
                if !remaining_integrity.is_empty() {
                    return Err(ServiceError::new(
                        "sqlite_integrity_failed",
                        remaining_integrity
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; "),
                    ));
                }
                match ArtifactStore::open(admitted.canonical_root()) {
                    Ok(artifacts) => {
                        for snapshot in &reachable {
                            if let Err(error) = artifacts.verify(snapshot) {
                                issues.push(audit_issue(
                                    "artifact_integrity_failed",
                                    None,
                                    error.to_string(),
                                ));
                            }
                        }
                    }
                    Err(error) => issues.push(audit_issue(
                        "artifact_store_unavailable",
                        None,
                        error.to_string(),
                    )),
                }
            }
            MaintenanceAction::ArtifactGc => {
                let artifacts =
                    ArtifactStore::open(admitted.canonical_root()).map_err(artifact_error)?;
                for (objective, value) in &replayed {
                    if let Err(error) = compare_projection(
                        value,
                        transaction
                            .objective_projection(objective)
                            .map_err(store_error)?,
                        transaction
                            .object_projections(objective)
                            .map_err(store_error)?,
                    ) {
                        issues.push(audit_issue(
                            "projection_mismatch",
                            Some(objective.clone()),
                            error.message,
                        ));
                    }
                }
                if !issues.is_empty() {
                    return Err(ServiceError::new(
                        "projection_mismatch",
                        "artifact GC is blocked until projection rebuild succeeds",
                    ));
                }
                artifacts.gc(&reachable).map_err(artifact_error)?;
            }
        }
        transaction.commit().map_err(store_error)?;
        let status = audit_status(&issues);
        let issues = audit_issue_page(issues, limit)?;
        Ok(AuditResponse {
            status,
            project_seq,
            checked_objectives: objective_ids.len(),
            issues,
            maintenance_applied: Some(maintenance.action),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceError {
    pub code: &'static str,
    pub message: String,
}

impl ServiceError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ObjectiveProjectionDocumentRef<'a> {
    objective_state: &'a ObjectiveState,
    lifecycle: &'a LifecycleProjection,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectiveProjectionDocument {
    objective_state: ObjectiveState,
    lifecycle: LifecycleProjection,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct MutationPayloadRef<'a> {
    project_id: &'a ProjectId,
    expected_heads: &'a HeadBinding,
    command: &'a MutationCommand,
}

struct ReplayedObjective {
    configuration: DomainConfiguration,
    objective_row: Option<ObjectiveProjectionRow>,
    object_rows: Vec<ObjectProjectionRow>,
}

struct ProjectReplay {
    objectives: BTreeMap<ObjectiveId, ReplayedObjective>,
    active: Option<ObjectiveId>,
}

fn load_objective_projection_document(
    transaction: &ReadTransaction<'_>,
    objective: &ObjectiveId,
    indexed_active: Option<&ObjectiveId>,
) -> Result<Option<(ObjectiveProjectionDocument, u64)>, ServiceError> {
    let Some(stream) = transaction.stream_head(objective).map_err(store_error)? else {
        return Ok(None);
    };
    let objective_row = transaction
        .objective_projection(objective)
        .map_err(store_error)?
        .ok_or_else(|| {
            ServiceError::new(
                "projection_mismatch",
                "Objective stream has no current projection",
            )
        })?;
    if &objective_row.objective_id != objective
        || objective_row.project_seq != stream.last_project_seq
        || objective_row.objective_seq != stream.objective_seq
        || objective_row.projection_schema != OBJECTIVE_PROJECTION_SCHEMA
    {
        return Err(ServiceError::new(
            "projection_mismatch",
            "Objective projection does not match its stream head or schema",
        ));
    }
    let document: ObjectiveProjectionDocument =
        decode_projection(&objective_row.projection_bytes, "Objective projection")?;
    if objective_row.is_active != is_active(&document.objective_state)
        || objective_row.is_active != (indexed_active == Some(objective))
    {
        return Err(ServiceError::new(
            "projection_mismatch",
            "Objective projection active state does not match the active index",
        ));
    }
    Ok(Some((document, stream.last_project_seq)))
}

fn decode_projection<T>(bytes: &[u8], label: &str) -> Result<T, ServiceError>
where
    T: DeserializeOwned + Serialize,
{
    let value = serde_json::from_slice(bytes).map_err(|error| {
        ServiceError::new("invalid_projection", format!("{label} is invalid: {error}"))
    })?;
    if encode_canonical(&value).map_err(codec_error)? != bytes {
        return Err(ServiceError::new(
            "invalid_projection",
            format!("{label} is not canonical"),
        ));
    }
    Ok(value)
}

fn command_objective(
    command: &MutationCommand,
    active: Option<&ObjectiveId>,
) -> Result<ObjectiveId, ServiceError> {
    let explicit = match command {
        MutationCommand::ActivateObjective(input) => Some(&input.objective_spec.objective),
        MutationCommand::InstallMap(input) => Some(&input.map.objective_spec.objective),
        MutationCommand::ReviseObjective(input) => Some(&input.objective_spec.objective),
        MutationCommand::Abandon(_) => None,
        MutationCommand::AddRoute(_)
        | MutationCommand::SelectRoute(_)
        | MutationCommand::StartAttempt(_)
        | MutationCommand::RecordEvidence(_)
        | MutationCommand::SealAttempt(_)
        | MutationCommand::Decision(_)
        | MutationCommand::CheckWait(_)
        | MutationCommand::RequestRemap(_) => None,
    };
    explicit
        .cloned()
        .or_else(|| active.cloned())
        .ok_or_else(|| {
            ServiceError::new(
                "no_active_objective",
                "the command requires an active Objective",
            )
        })
}

fn load_project_write(
    transaction: &WriteTransaction<'_>,
    include: Option<&ObjectiveId>,
) -> Result<ProjectReplay, ServiceError> {
    let mut objective_ids = transaction.objective_ids().map_err(store_error)?;
    include_missing_objective(&mut objective_ids, include);
    let mut objectives = BTreeMap::new();
    for objective in objective_ids {
        objectives.insert(
            objective.clone(),
            load_replayed_write(transaction, &objective)?,
        );
    }
    finish_project_replay(
        objectives,
        transaction.active_objective().map_err(store_error)?,
    )
}

fn read_validated_global_trail(
    transaction: &ReadTransaction<'_>,
) -> Result<Vec<EventRow>, ServiceError> {
    let objective_ids = transaction.objective_ids().map_err(store_error)?;
    let stream_heads = objective_ids
        .iter()
        .map(|objective| {
            Ok((
                objective.clone(),
                transaction
                    .stream_head(objective)
                    .map_err(store_error)?
                    .ok_or_else(|| {
                        ServiceError::new(
                            "trail_head_mismatch",
                            "Objective stream identity has no head row",
                        )
                    })?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ServiceError>>()?;
    let rows = transaction.trail_events(None).map_err(store_error)?;
    validate_global_trail_metadata(
        transaction.project_head().map_err(store_error)?,
        &objective_ids,
        &stream_heads,
        &rows,
    )?;
    Ok(rows)
}

fn validate_global_trail_write(transaction: &WriteTransaction<'_>) -> Result<(), ServiceError> {
    let objective_ids = transaction.objective_ids().map_err(store_error)?;
    let stream_heads = objective_ids
        .iter()
        .map(|objective| {
            Ok((
                objective.clone(),
                transaction
                    .stream_head(objective)
                    .map_err(store_error)?
                    .ok_or_else(|| {
                        ServiceError::new(
                            "trail_head_mismatch",
                            "Objective stream identity has no head row",
                        )
                    })?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ServiceError>>()?;
    validate_global_trail_metadata(
        transaction.project_head().map_err(store_error)?,
        &objective_ids,
        &stream_heads,
        &transaction.trail_events(None).map_err(store_error)?,
    )
}

fn validate_global_trail_metadata(
    project_head: u64,
    objective_ids: &[ObjectiveId],
    stream_heads: &BTreeMap<ObjectiveId, crate::infrastructure::sqlite::ObjectiveStreamHead>,
    rows: &[EventRow],
) -> Result<(), ServiceError> {
    let row_count = u64::try_from(rows.len())
        .map_err(|_| ServiceError::new("trail_overflow", "Trail row count exceeds u64"))?;
    if project_head != row_count {
        return Err(ServiceError::new(
            "trail_head_mismatch",
            format!("project head is {project_head}, but Trail has {row_count} rows"),
        ));
    }
    let mut derived = BTreeMap::<ObjectiveId, (u64, u64, u64)>::new();
    for (index, row) in rows.iter().enumerate() {
        let expected_project_seq = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| ServiceError::new("trail_overflow", "Trail index overflow"))?;
        if row.metadata.project_seq != expected_project_seq {
            return Err(ServiceError::new(
                "trail_head_mismatch",
                "project Trail sequence is not contiguous",
            ));
        }
        let entry = derived.entry(row.metadata.objective_id.clone()).or_insert((
            0,
            row.metadata.project_seq,
            0,
        ));
        let expected_objective_seq = entry
            .0
            .checked_add(1)
            .ok_or_else(|| ServiceError::new("trail_overflow", "Objective head overflow"))?;
        if row.metadata.objective_seq != expected_objective_seq {
            return Err(ServiceError::new(
                "trail_head_mismatch",
                "Objective Trail sequence is not contiguous",
            ));
        }
        entry.0 = expected_objective_seq;
        entry.2 = row.metadata.project_seq;
    }
    let derived_ids = derived.keys().cloned().collect::<BTreeSet<_>>();
    let stored_ids = objective_ids.iter().cloned().collect::<BTreeSet<_>>();
    if derived_ids != stored_ids {
        return Err(ServiceError::new(
            "trail_head_mismatch",
            "Objective stream identities do not match Trail rows",
        ));
    }
    for (objective, (objective_seq, created_project_seq, last_project_seq)) in derived {
        let head = stream_heads.get(&objective).ok_or_else(|| {
            ServiceError::new("trail_head_mismatch", "Trail Objective has no stream head")
        })?;
        if head.objective_seq != objective_seq
            || head.created_project_seq != created_project_seq
            || head.last_project_seq != last_project_seq
        {
            return Err(ServiceError::new(
                "trail_head_mismatch",
                format!(
                    "Objective {:?} stream head does not match Trail",
                    objective.as_str()
                ),
            ));
        }
    }
    Ok(())
}

fn include_missing_objective(objectives: &mut Vec<ObjectiveId>, include: Option<&ObjectiveId>) {
    if let Some(include) = include {
        if !objectives.contains(include) {
            objectives.push(include.clone());
            objectives.sort();
        }
    }
}

fn finish_project_replay(
    objectives: BTreeMap<ObjectiveId, ReplayedObjective>,
    indexed_active: Option<ObjectiveId>,
) -> Result<ProjectReplay, ServiceError> {
    let active = objectives
        .iter()
        .filter(|(_, value)| is_active(value.configuration.objective_state()))
        .map(|(objective, _)| objective.clone())
        .collect::<Vec<_>>();
    if active.len() > 1 {
        return Err(ServiceError::new(
            "single_active_violation",
            "Trail replay produced more than one active Objective",
        ));
    }
    let active = active.into_iter().next();
    if active != indexed_active {
        return Err(ServiceError::new(
            "projection_mismatch",
            "the indexed active Objective does not match Trail replay",
        ));
    }
    Ok(ProjectReplay { objectives, active })
}

fn load_replayed_write(
    transaction: &WriteTransaction<'_>,
    objective: &ObjectiveId,
) -> Result<ReplayedObjective, ServiceError> {
    let replayed = replay_event_rows(
        objective,
        transaction
            .trail_events(Some(objective))
            .map_err(store_error)?,
    )?;
    compare_projection(
        &replayed,
        transaction
            .objective_projection(objective)
            .map_err(store_error)?,
        transaction
            .object_projections(objective)
            .map_err(store_error)?,
    )?;
    Ok(replayed)
}

fn replay_event_rows(
    objective: &ObjectiveId,
    rows: Vec<EventRow>,
) -> Result<ReplayedObjective, ServiceError> {
    let mut facts = Vec::with_capacity(rows.len());
    let mut previous_project_seq = 0_u64;
    for (index, row) in rows.iter().enumerate() {
        let expected_objective_seq = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| ServiceError::new("trail_overflow", "Trail index overflow"))?;
        if &row.metadata.objective_id != objective
            || row.metadata.objective_seq != expected_objective_seq
            || row.metadata.project_seq <= previous_project_seq
        {
            return Err(ServiceError::new(
                "invalid_trail_order",
                "stored Trail metadata is not a contiguous Objective stream",
            ));
        }
        if row.event_schema != TRAIL_EVENT_SCHEMA {
            return Err(ServiceError::new(
                "unsupported_event_schema",
                format!("unsupported stored event schema {:?}", row.event_schema),
            ));
        }
        let fact = decode_trail_fact(&row.event_bytes).map_err(codec_error)?;
        if &fact.objective != objective {
            return Err(ServiceError::new(
                "objective_binding_mismatch",
                "stored event bytes do not match their Objective stream",
            ));
        }
        previous_project_seq = row.metadata.project_seq;
        facts.push(fact);
    }

    let configuration = replay(&facts)
        .map_err(|error| ServiceError::new("trail_replay_failed", error.to_string()))?;
    let accepted = accepted_object_events(&facts, &rows)?;
    let (objective_row, object_rows) = projection_rows(
        objective,
        &configuration,
        rows.last().map(|row| &row.metadata),
        &accepted,
    )?;
    Ok(ReplayedObjective {
        configuration,
        objective_row,
        object_rows,
    })
}

fn accepted_object_events(
    facts: &[TrailFact],
    rows: &[EventRow],
) -> Result<BTreeMap<ObjectIdentity, u64>, ServiceError> {
    let mut accepted = BTreeMap::new();
    let mut configuration = initial_configuration();
    for (fact, row) in facts.iter().zip(rows) {
        let before = configuration
            .objects()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        configuration = reduce(&configuration, &fact.input)
            .map_err(|error| ServiceError::new("trail_replay_failed", error.to_string()))?;
        for identity in configuration.objects().keys() {
            if !before.contains(identity) {
                accepted.insert(identity.clone(), row.metadata.project_seq);
            }
        }
    }
    Ok(accepted)
}

fn projection_rows(
    objective: &ObjectiveId,
    configuration: &DomainConfiguration,
    head: Option<&EventMetadata>,
    accepted: &BTreeMap<ObjectIdentity, u64>,
) -> Result<(Option<ObjectiveProjectionRow>, Vec<ObjectProjectionRow>), ServiceError> {
    let Some(head) = head else {
        if !configuration.objects().is_empty()
            || !matches!(configuration.objective_state(), ObjectiveState::Idle)
        {
            return Err(ServiceError::new(
                "invalid_projection",
                "an empty Trail replayed to non-empty state",
            ));
        }
        return Ok((None, Vec::new()));
    };
    let objective_row = ObjectiveProjectionRow {
        objective_id: objective.clone(),
        project_seq: head.project_seq,
        objective_seq: head.objective_seq,
        is_active: is_active(configuration.objective_state()),
        projection_schema: OBJECTIVE_PROJECTION_SCHEMA.to_owned(),
        projection_bytes: encode_canonical(&ObjectiveProjectionDocumentRef {
            objective_state: configuration.objective_state(),
            lifecycle: configuration.lifecycle(),
        })
        .map_err(codec_error)?,
    };
    let mut object_rows = configuration
        .objects()
        .iter()
        .map(|(identity, object)| {
            let accepted_project_seq = accepted.get(identity).copied().ok_or_else(|| {
                ServiceError::new(
                    "invalid_projection",
                    "a replayed object has no accepting Trail event",
                )
            })?;
            object_projection_row(objective, object, accepted_project_seq)
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    sort_object_rows(&mut object_rows);
    Ok((Some(objective_row), object_rows))
}

fn projection_rows_after_append(
    objective: &ObjectiveId,
    configuration: &DomainConfiguration,
    metadata: &EventMetadata,
    prior_rows: &[ObjectProjectionRow],
) -> Result<(ObjectiveProjectionRow, Vec<ObjectProjectionRow>), ServiceError> {
    let prior_accepted = prior_rows
        .iter()
        .map(|row| {
            (
                (row.object_kind.clone(), row.object_id.clone()),
                row.accepted_project_seq,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut object_rows = Vec::with_capacity(configuration.objects().len());
    for object in configuration.objects().values() {
        let mut row = object_projection_row(objective, object, metadata.project_seq)?;
        if let Some(accepted) =
            prior_accepted.get(&(row.object_kind.clone(), row.object_id.clone()))
        {
            row.accepted_project_seq = *accepted;
        }
        object_rows.push(row);
    }
    sort_object_rows(&mut object_rows);
    let objective_row = ObjectiveProjectionRow {
        objective_id: objective.clone(),
        project_seq: metadata.project_seq,
        objective_seq: metadata.objective_seq,
        is_active: is_active(configuration.objective_state()),
        projection_schema: OBJECTIVE_PROJECTION_SCHEMA.to_owned(),
        projection_bytes: encode_canonical(&ObjectiveProjectionDocumentRef {
            objective_state: configuration.objective_state(),
            lifecycle: configuration.lifecycle(),
        })
        .map_err(codec_error)?,
    };
    Ok((objective_row, object_rows))
}

fn object_projection_row(
    objective: &ObjectiveId,
    object: &FirstClassObject,
    accepted_project_seq: u64,
) -> Result<ObjectProjectionRow, ServiceError> {
    let identity = String::from_utf8(encode_canonical(&object.identity()).map_err(codec_error)?)
        .map_err(|error| ServiceError::new("serialization_error", error.to_string()))?;
    Ok(ObjectProjectionRow {
        objective_id: objective.clone(),
        object_kind: object.kind().schema_name().to_owned(),
        object_id: identity,
        accepted_project_seq,
        projection_schema: OBJECT_PROJECTION_SCHEMA.to_owned(),
        projection_bytes: encode_canonical(object).map_err(codec_error)?,
    })
}

fn sort_object_rows(rows: &mut [ObjectProjectionRow]) {
    rows.sort_by(|left, right| {
        (&left.object_kind, &left.object_id).cmp(&(&right.object_kind, &right.object_id))
    });
}

fn compare_projection(
    expected: &ReplayedObjective,
    actual_objective: Option<ObjectiveProjectionRow>,
    mut actual_objects: Vec<ObjectProjectionRow>,
) -> Result<(), ServiceError> {
    sort_object_rows(&mut actual_objects);
    if expected.objective_row != actual_objective || expected.object_rows != actual_objects {
        return Err(ServiceError::new(
            "projection_mismatch",
            "stored projection does not equal deterministic Trail replay",
        ));
    }
    Ok(())
}

fn is_active(state: &ObjectiveState) -> bool {
    matches!(
        state,
        ObjectiveState::Mapping { .. } | ObjectiveState::Navigating { .. }
    )
}

fn existing_response(
    transaction: &WriteTransaction<'_>,
    metadata: &EventMetadata,
) -> Result<ApplyTransitionResponse, ServiceError> {
    let row = transaction
        .trail_events(Some(&metadata.objective_id))
        .map_err(store_error)?
        .into_iter()
        .find(|row| row.metadata.project_seq == metadata.project_seq)
        .ok_or_else(|| {
            ServiceError::new(
                "invalid_trail",
                "idempotency metadata references a missing Trail event",
            )
        })?;
    if row.event_schema != TRAIL_EVENT_SCHEMA {
        return Err(ServiceError::new(
            "unsupported_event_schema",
            "idempotent event uses an unsupported schema",
        ));
    }
    let fact = decode_trail_fact(&row.event_bytes).map_err(codec_error)?;
    if fact.objective != metadata.objective_id {
        return Err(ServiceError::new(
            "objective_binding_mismatch",
            "idempotent event bytes do not match their Objective metadata",
        ));
    }
    Ok(ApplyTransitionResponse {
        objective_id: metadata.objective_id.clone(),
        transition: fact.transition(),
        committed_project_seq: metadata.project_seq,
        committed_objective_seq: metadata.objective_seq,
        event_digest: sha256_text(&row.event_bytes),
    })
}

fn audit_issue_limit(requested: Option<u32>) -> Result<usize, ServiceError> {
    let limit = requested.unwrap_or(DEFAULT_AUDIT_ISSUE_LIMIT);
    if limit == 0 || limit > MAX_AUDIT_ISSUE_LIMIT {
        return Err(ServiceError::new(
            "invalid_page_limit",
            format!("audit issue limit must be between 1 and {MAX_AUDIT_ISSUE_LIMIT}"),
        ));
    }
    Ok(limit as usize)
}

fn build_report_snapshot(
    configuration: &DomainConfiguration,
    event_rows: &[EventRow],
    all_event_rows: &[EventRow],
    heads: &crate::infrastructure::sqlite::HeadSnapshot,
) -> Result<ReportSnapshot, ServiceError> {
    let facts = event_rows
        .iter()
        .map(|row| decode_trail_fact(&row.event_bytes).map_err(codec_error))
        .collect::<Result<Vec<_>, _>>()?;
    let objective = objective_id(configuration).ok_or_else(|| {
        ServiceError::new(
            "invalid_projection",
            "a non-empty Objective Trail replayed without an Objective identity",
        )
    })?;
    let current_map = current_map(configuration);
    let current_stage = current_stage(configuration);
    let current_route = current_route(configuration);
    let proofs = current_proofs(configuration)
        .map_err(|error| ServiceError::new("invalid_projection", error.to_string()))?;
    let mut maps = configuration
        .objects()
        .values()
        .filter_map(|object| match object {
            FirstClassObject::MapRevision(map) => Some(map),
            _ => None,
        })
        .collect::<Vec<_>>();
    maps.sort_by(|left, right| {
        (left.objective_spec.revision, left.revision)
            .cmp(&(right.objective_spec.revision, right.revision))
    });
    let mut specifications = configuration
        .objects()
        .values()
        .filter_map(|object| match object {
            FirstClassObject::ObjectiveSpec(specification) => Some(specification),
            _ => None,
        })
        .collect::<Vec<_>>();
    specifications.sort_by_key(|specification| specification.revision);

    let (spec_revision, map_revision) = match configuration.objective_state() {
        ObjectiveState::Mapping { objective_spec, .. } => (Some(objective_spec.revision), None),
        ObjectiveState::Navigating { map, .. } | ObjectiveState::Achieved { map, .. } => (
            current_map.map(|value| value.objective_spec.revision),
            Some(map.revision),
        ),
        ObjectiveState::Idle | ObjectiveState::Abandoned { .. } => (None, None),
    };
    let spec_revision = spec_revision.or_else(|| {
        configuration
            .objects()
            .values()
            .filter_map(|object| match object {
                FirstClassObject::ObjectiveSpec(spec) if spec.objective == objective => {
                    Some(spec.revision)
                }
                _ => None,
            })
            .max()
    });
    let overview = ReportRows::new(
        ["key", "value"],
        vec![
            text_pair("objective_id", objective.as_str()),
            text_pair("state", &json_value_text(configuration.objective_state())?),
            integer_pair("project_seq", heads.project_seq),
            integer_pair("objective_seq", heads.objective_seq),
            vec![
                "active".into(),
                ReportCell::Boolean(is_active(configuration.objective_state())),
            ],
            optional_integer_pair("objective_spec_revision", spec_revision),
            optional_integer_pair("map_revision", map_revision),
            integer_pair("object_count", configuration.objects().len() as u64),
        ],
    );

    let mut stage_rows = Vec::new();
    for map in &maps {
        let map_current = current_map == Some(*map);
        for stage in map.stages.values() {
            let state = if current_stage == Some(&stage.id) {
                "current"
            } else if proofs.contains_key(&stage.id) {
                "achieved"
            } else if current_map.is_some_and(|map| map.stages.contains_key(&stage.id)) {
                "queued"
            } else {
                "retired"
            };
            stage_rows.push(vec![
                ReportCell::Integer(i128::from(map.objective_spec.revision)),
                ReportCell::Integer(i128::from(map.revision)),
                ReportCell::Boolean(map_current),
                stage.id.as_str().into(),
                stage.name.clone().into(),
                json_value_text(&stage.kind)?.into(),
                state.into(),
                stage.outcome.clone().into(),
                stage.output.clone().into(),
                optional_integer(map.priorities.get(&stage.id).copied()),
            ]);
        }
    }
    let stages = ReportRows::new(
        [
            "objective_spec_revision",
            "map_revision",
            "map_current",
            "stage_id",
            "name",
            "kind",
            "state",
            "outcome",
            "output",
            "priority",
        ],
        stage_rows,
    );

    let mut criterion_rows = Vec::new();
    let mut mapped_criteria = BTreeSet::new();
    for map in &maps {
        let map_current = current_map == Some(*map);
        for criterion in map.criteria.values() {
            mapped_criteria.insert((map.objective_spec.clone(), criterion.id.clone()));
            criterion_rows.push(vec![
                ReportCell::Integer(i128::from(map.objective_spec.revision)),
                ReportCell::Integer(i128::from(map.revision)),
                ReportCell::Boolean(map_current),
                criterion.id.as_str().into(),
                criterion.statement.clone().into(),
                criterion.verification_rule.clone().into(),
                json_value_text(&criterion.scope)?.into(),
                map.owners
                    .get(&criterion.id)
                    .map_or(ReportCell::Empty, |stage| stage.as_str().into()),
            ]);
        }
    }
    for specification in specifications {
        let specification_id = ObjectiveSpecId {
            objective: specification.objective.clone(),
            revision: specification.revision,
        };
        for criterion in specification.criteria.values() {
            if !mapped_criteria.contains(&(specification_id.clone(), criterion.id.clone())) {
                criterion_rows.push(vec![
                    ReportCell::Integer(i128::from(specification.revision)),
                    ReportCell::Empty,
                    ReportCell::Boolean(false),
                    criterion.id.as_str().into(),
                    criterion.statement.clone().into(),
                    criterion.verification_rule.clone().into(),
                    json_value_text(&criterion.scope)?.into(),
                    ReportCell::Empty,
                ]);
            }
        }
    }
    let criteria = ReportRows::new(
        [
            "objective_spec_revision",
            "map_revision",
            "map_current",
            "criterion_id",
            "statement",
            "verification_rule",
            "scope",
            "owner_stage",
        ],
        criterion_rows,
    );

    let mut route_rows = Vec::new();
    for object in configuration.objects().values() {
        let FirstClassObject::Route(route) = object else {
            continue;
        };
        let assumptions = optional_strings(&route.assumptions);
        for assumption in assumptions {
            route_rows.push(vec![
                route.id.as_str().into(),
                route.stage.as_str().into(),
                configuration
                    .lifecycle()
                    .route_status
                    .get(&route.id)
                    .map(json_value_text)
                    .transpose()?
                    .map_or(ReportCell::Empty, ReportCell::Text),
                ReportCell::Boolean(current_route == Some(&route.id)),
                route.hypothesis.clone().into(),
                assumption,
                route.rationale.clone().into(),
            ]);
        }
    }
    let routes = ReportRows::new(
        [
            "route_id",
            "stage_id",
            "status",
            "route_current",
            "hypothesis",
            "assumption",
            "rationale",
        ],
        route_rows,
    );

    let attempt_history = attempt_report_history(configuration, &facts)?;
    let mut attempt_rows = Vec::new();
    for object in configuration.objects().values() {
        let FirstClassObject::Attempt(attempt) = object else {
            continue;
        };
        let history = attempt_history.get(attempt.id.as_str());
        for bound in attempt_bound_values(&attempt.bound)? {
            attempt_rows.push(vec![
                attempt.id.as_str().into(),
                attempt.route.as_str().into(),
                ReportCell::Integer(i128::from(attempt.ordinal)),
                configuration
                    .lifecycle()
                    .attempt_state
                    .get(&attempt.id)
                    .map(json_value_text)
                    .transpose()?
                    .map_or(ReportCell::Empty, ReportCell::Text),
                bound,
                history
                    .and_then(|value| value.termination.clone())
                    .map_or(ReportCell::Empty, ReportCell::Text),
                history
                    .and_then(|value| value.close_reason.clone())
                    .map_or(ReportCell::Empty, ReportCell::Text),
                history
                    .and_then(|value| value.action.clone())
                    .map_or(ReportCell::Empty, ReportCell::Text),
            ]);
        }
    }
    let attempts = ReportRows::new(
        [
            "attempt_id",
            "route_id",
            "ordinal",
            "state",
            "bound",
            "termination",
            "close_reason",
            "action",
        ],
        attempt_rows,
    );

    let mut evidence_rows = Vec::new();
    for object in configuration.objects().values() {
        let FirstClassObject::Evidence(evidence) = object else {
            continue;
        };
        let claims = if evidence.claims.is_empty() {
            vec![(ReportCell::Empty, ReportCell::Empty)]
        } else {
            evidence
                .claims
                .iter()
                .map(|(criterion, claim)| {
                    Ok((
                        ReportCell::Text(criterion.as_str().to_owned()),
                        ReportCell::Text(json_value_text(claim)?),
                    ))
                })
                .collect::<Result<Vec<_>, ServiceError>>()?
        };
        let (observation_kind, digest, size, observation) = match &evidence.observation {
            FrozenObservation::Inline(value) => (
                "inline".to_owned(),
                ReportCell::Empty,
                ReportCell::Empty,
                json_value_text(value)?,
            ),
            FrozenObservation::CoreSnapshot(snapshot) => (
                "core_snapshot".to_owned(),
                snapshot.digest.0.clone().into(),
                ReportCell::Integer(i128::from(snapshot.size_bytes)),
                String::new(),
            ),
        };
        for (criterion, claim) in claims {
            evidence_rows.push(vec![
                evidence.id.as_str().into(),
                json_value_text(&evidence.subject)?.into(),
                json_value_text(&evidence.purpose)?.into(),
                criterion,
                claim,
                observation_kind.clone().into(),
                digest.clone(),
                size.clone(),
                observation.clone().into(),
                json_value_text(&evidence.provenance)?.into(),
            ]);
        }
    }
    let evidence = ReportRows::new(
        [
            "evidence_id",
            "subject",
            "purpose",
            "criterion_id",
            "claim",
            "observation_kind",
            "digest",
            "size_bytes",
            "inline_observation",
            "provenance",
        ],
        evidence_rows,
    );

    let decisions = configuration
        .objects()
        .values()
        .filter_map(|object| match object {
            FirstClassObject::ReviewDecision(decision) => Some((decision.packet.clone(), decision)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut review_rows = Vec::new();
    for object in configuration.objects().values() {
        let FirstClassObject::ReviewPacket(packet) = object else {
            continue;
        };
        let decision = decisions.get(&packet.id).copied();
        let findings = decision.map_or_else(
            || vec![ReportCell::Empty],
            |decision| optional_strings(&decision.findings),
        );
        for criterion in &packet.context.structural.contract.criteria {
            let [supports, contradicts, unknown, unassessed] =
                packet_claim_counts(configuration, packet, criterion)?;
            for finding in &findings {
                review_rows.push(vec![
                    packet.id.as_str().into(),
                    packet.attempt.as_str().into(),
                    packet.stage.as_str().into(),
                    criterion.as_str().into(),
                    decision
                        .map(|value| value.id.as_str().to_owned())
                        .map_or(ReportCell::Empty, ReportCell::Text),
                    decision
                        .and_then(|value| value.judgments.get(criterion))
                        .map(json_value_text)
                        .transpose()?
                        .map_or(ReportCell::Empty, ReportCell::Text),
                    decision
                        .map(|value| json_value_text(&value.action))
                        .transpose()?
                        .map_or(ReportCell::Empty, ReportCell::Text),
                    finding.clone(),
                    ReportCell::Integer(packet.evidence_set.len() as i128),
                    ReportCell::Integer(supports as i128),
                    ReportCell::Integer(contradicts as i128),
                    ReportCell::Integer(unknown as i128),
                    ReportCell::Integer(unassessed as i128),
                ]);
            }
        }
    }
    let reviews = ReportRows::new(
        [
            "packet_id",
            "attempt_id",
            "stage_id",
            "criterion_id",
            "decision_id",
            "judgment",
            "action",
            "finding",
            "evidence_count",
            "supports_count",
            "contradicts_count",
            "unknown_count",
            "unassessed_count",
        ],
        review_rows,
    );

    let timeline = ReportRows::new(
        [
            "project_seq",
            "objective_seq",
            "transition",
            "summary",
            "event_digest",
            "received_at",
        ],
        event_rows
            .iter()
            .zip(&facts)
            .map(|(row, fact)| {
                Ok(vec![
                    ReportCell::Integer(i128::from(row.metadata.project_seq)),
                    ReportCell::Integer(i128::from(row.metadata.objective_seq)),
                    fact.transition().schema_name().into(),
                    transition_summary(&fact.input)?.into(),
                    sha256_text(&row.event_bytes).into(),
                    row.received_at.clone().into(),
                ])
            })
            .collect::<Result<Vec<_>, ServiceError>>()?,
    );

    let report_heads = ReportHeads {
        project_seq: heads.project_seq,
        objective_seq: heads.objective_seq,
    };
    let trail_digest = trail_digest(event_rows);
    let trail_prefix_digests = trail_prefix_digests(&objective, all_event_rows);
    if trail_prefix_digests.get(&report_heads) != Some(&trail_digest) {
        return Err(ServiceError::new(
            "trail_head_mismatch",
            "current report heads do not identify the exact Objective Trail prefix",
        ));
    }

    Ok(ReportSnapshot {
        objective_id: objective,
        heads: report_heads,
        trail_digest,
        trail_prefix_digests,
        overview,
        stages,
        criteria,
        routes,
        attempts,
        evidence,
        reviews,
        timeline,
    })
}

fn current_map(configuration: &DomainConfiguration) -> Option<&crate::domain::MapRevision> {
    let identity = match configuration.objective_state() {
        ObjectiveState::Navigating { map, .. } | ObjectiveState::Achieved { map, .. } => map,
        ObjectiveState::Idle
        | ObjectiveState::Mapping { .. }
        | ObjectiveState::Abandoned { .. } => {
            return None;
        }
    };
    match configuration
        .objects()
        .get(&ObjectIdentity::MapRevision(identity.clone()))
    {
        Some(FirstClassObject::MapRevision(map)) => Some(map),
        _ => None,
    }
}

fn current_stage(configuration: &DomainConfiguration) -> Option<&StageId> {
    match configuration.objective_state() {
        ObjectiveState::Navigating { navigation, .. } => Some(match navigation {
            NavState::SeekingRoute { stage }
            | NavState::Ready { stage, .. }
            | NavState::Attempting { stage, .. }
            | NavState::Reviewing { stage, .. }
            | NavState::Waiting { stage, .. } => stage,
        }),
        ObjectiveState::Idle
        | ObjectiveState::Mapping { .. }
        | ObjectiveState::Achieved { .. }
        | ObjectiveState::Abandoned { .. } => None,
    }
}

fn current_route(configuration: &DomainConfiguration) -> Option<&crate::domain::RouteId> {
    match configuration.objective_state() {
        ObjectiveState::Navigating { navigation, .. } => match navigation {
            NavState::Ready { route, .. }
            | NavState::Attempting { route, .. }
            | NavState::Reviewing { route, .. }
            | NavState::Waiting { route, .. } => Some(route),
            NavState::SeekingRoute { .. } => None,
        },
        ObjectiveState::Idle
        | ObjectiveState::Mapping { .. }
        | ObjectiveState::Achieved { .. }
        | ObjectiveState::Abandoned { .. } => None,
    }
}

fn packet_claim_counts(
    configuration: &DomainConfiguration,
    packet: &ReviewPacket,
    criterion: &crate::domain::CriterionId,
) -> Result<[usize; 4], ServiceError> {
    let mut counts = [0; 4];
    for evidence_id in &packet.evidence_set {
        let evidence = match configuration
            .objects()
            .get(&ObjectIdentity::Evidence(evidence_id.clone()))
        {
            Some(FirstClassObject::Evidence(evidence)) => evidence,
            _ => {
                return Err(ServiceError::new(
                    "invalid_projection",
                    format!(
                        "ReviewPacket {} references missing Evidence {}",
                        packet.id.as_str(),
                        evidence_id.as_str()
                    ),
                ));
            }
        };
        match evidence.claims.get(criterion) {
            Some(crate::domain::EvidenceClaim::Supports) => counts[0] += 1,
            Some(crate::domain::EvidenceClaim::Contradicts) => counts[1] += 1,
            Some(crate::domain::EvidenceClaim::Unknown) => counts[2] += 1,
            None => counts[3] += 1,
        }
    }
    Ok(counts)
}

#[derive(Default)]
struct AttemptReportHistory {
    termination: Option<String>,
    close_reason: Option<String>,
    action: Option<String>,
}

fn attempt_report_history(
    configuration: &DomainConfiguration,
    facts: &[TrailFact],
) -> Result<BTreeMap<String, AttemptReportHistory>, ServiceError> {
    let packet_attempt = configuration
        .objects()
        .values()
        .filter_map(|object| match object {
            FirstClassObject::ReviewPacket(packet) => {
                Some((packet.id.clone(), packet.attempt.as_str().to_owned()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut history = BTreeMap::<String, AttemptReportHistory>::new();
    let mut state = initial_configuration();
    for fact in facts {
        match &fact.input {
            TransitionInput::SealAttempt(input) => {
                history
                    .entry(input.packet.attempt.as_str().to_owned())
                    .or_default()
                    .termination = Some(json_value_text(&input.seal_reason)?);
            }
            TransitionInput::Decision(input) => {
                if let Some(attempt) = packet_attempt.get(&input.decision.packet) {
                    history.entry(attempt.clone()).or_default().action =
                        Some(json_value_text(&input.decision.action)?);
                }
            }
            _ => {}
        }
        let next = reduce(&state, &fact.input)
            .map_err(|error| ServiceError::new("trail_replay_failed", error.to_string()))?;
        for (attempt, next_state) in &next.lifecycle().attempt_state {
            if *next_state == crate::domain::AttemptState::Closed
                && state.lifecycle().attempt_state.get(attempt)
                    != Some(&crate::domain::AttemptState::Closed)
            {
                let close_reason = match &fact.input {
                    TransitionInput::Decision(_) => "reviewed",
                    TransitionInput::RequestRemap(_) | TransitionInput::ReviseObjective(_) => {
                        "remapped"
                    }
                    TransitionInput::Abandon(_) => "abandoned",
                    _ => {
                        return Err(ServiceError::new(
                            "invalid_trail",
                            "Attempt closed from a transition without a model close reason",
                        ));
                    }
                };
                history
                    .entry(attempt.as_str().to_owned())
                    .or_default()
                    .close_reason = Some(close_reason.to_owned());
            }
        }
        state = next;
    }
    Ok(history)
}

fn attempt_bound_values(
    bound: &crate::domain::AttemptBound,
) -> Result<Vec<ReportCell>, ServiceError> {
    Ok(match bound {
        crate::domain::AttemptBound::VerificationScope(values) => optional_strings(values),
        crate::domain::AttemptBound::ResourceBudget { measure, limit } => {
            vec![format!("{measure}:{limit}").into()]
        }
        crate::domain::AttemptBound::TerminationCondition(value) => vec![value.clone().into()],
    })
}

fn optional_strings(values: &BTreeSet<String>) -> Vec<ReportCell> {
    if values.is_empty() {
        vec![ReportCell::Empty]
    } else {
        values.iter().cloned().map(ReportCell::Text).collect()
    }
}

fn transition_summary(input: &TransitionInput) -> Result<String, ServiceError> {
    Ok(match input {
        TransitionInput::ActivateObjective(value) => format!(
            "activate {} revision {}",
            value.objective_spec.objective.as_str(),
            value.objective_spec.revision
        ),
        TransitionInput::InstallMap(value) => {
            format!("install map revision {}", value.map.revision)
        }
        TransitionInput::AddRoute(value) => format!("add route {}", value.route.id.as_str()),
        TransitionInput::SelectRoute(value) => format!("select route {}", value.route.as_str()),
        TransitionInput::StartAttempt(value) => {
            format!("start attempt {}", value.attempt.id.as_str())
        }
        TransitionInput::RecordEvidence(value) => {
            format!("record evidence {}", value.evidence.id.as_str())
        }
        TransitionInput::SealAttempt(value) => format!(
            "seal attempt {} ({})",
            value.packet.attempt.as_str(),
            json_value_text(&value.seal_reason)?
        ),
        TransitionInput::Decision(value) => format!(
            "decision {} ({})",
            value.decision.id.as_str(),
            json_value_text(&value.decision.action)?
        ),
        TransitionInput::CheckWait(value) => format!(
            "check wait {} ({})",
            value.wait_condition.as_str(),
            json_value_text(&value.judgment.direction)?
        ),
        TransitionInput::RequestRemap(value) => format!("request remap: {}", value.reason),
        TransitionInput::ReviseObjective(value) => format!(
            "revise {} to revision {}",
            value.objective_spec.objective.as_str(),
            value.objective_spec.revision
        ),
        TransitionInput::Abandon(value) => format!("abandon: {}", value.reason),
    })
}

fn json_value_text(value: &impl Serialize) -> Result<String, ServiceError> {
    let value = serde_json::to_value(value)
        .map_err(|error| ServiceError::new("serialization_error", error.to_string()))?;
    Ok(match value {
        serde_json::Value::String(value) => value,
        value => value.to_string(),
    })
}

fn text_pair(key: &str, value: &str) -> Vec<ReportCell> {
    vec![key.into(), value.into()]
}

fn integer_pair(key: &str, value: u64) -> Vec<ReportCell> {
    vec![key.into(), ReportCell::Integer(i128::from(value))]
}

fn optional_integer_pair(key: &str, value: Option<u64>) -> Vec<ReportCell> {
    vec![key.into(), optional_integer(value)]
}

fn optional_integer(value: Option<u64>) -> ReportCell {
    value.map_or(ReportCell::Empty, |value| {
        ReportCell::Integer(i128::from(value))
    })
}

fn trail_digest(rows: &[EventRow]) -> String {
    let mut hasher = Sha256::new();
    for row in rows {
        update_trail_digest(&mut hasher, row);
    }
    format_sha256(hasher.finalize())
}

fn trail_prefix_digests(
    objective: &ObjectiveId,
    rows: &[EventRow],
) -> BTreeMap<ReportHeads, String> {
    let mut hasher = Sha256::new();
    let mut objective_seq = 0;
    let mut prefixes = BTreeMap::new();
    for row in rows {
        if &row.metadata.objective_id == objective {
            update_trail_digest(&mut hasher, row);
            objective_seq = row.metadata.objective_seq;
        }
        if objective_seq != 0 {
            prefixes.insert(
                ReportHeads {
                    project_seq: row.metadata.project_seq,
                    objective_seq,
                },
                format_sha256(hasher.clone().finalize()),
            );
        }
    }
    prefixes
}

fn update_trail_digest(hasher: &mut Sha256, row: &EventRow) {
    hasher.update((row.event_bytes.len() as u64).to_be_bytes());
    hasher.update(&row.event_bytes);
}

fn transition_material_snapshots(
    configuration: &DomainConfiguration,
    transition: &TransitionInput,
) -> Result<BTreeSet<CoreSnapshot>, ServiceError> {
    let mut evidence = BTreeMap::<EvidenceId, &Evidence>::new();
    let mut required = BTreeSet::new();
    match transition {
        TransitionInput::RecordEvidence(input) => {
            evidence.insert(input.evidence.id.clone(), &input.evidence);
            required.insert(input.evidence.id.clone());
        }
        TransitionInput::InstallMap(input) => {
            let proofs = current_proofs(configuration)
                .map_err(|error| ServiceError::new("invalid_projection", error.to_string()))?;
            for stage in input
                .carry
                .iter()
                .filter_map(|(stage, verdict)| (*verdict == CarryVerdict::Valid).then_some(stage))
            {
                let Some(decision_id) = proofs.get(stage) else {
                    // The domain carry guard owns malformed-input errors. Only an actual current
                    // proof contributes admission material here.
                    continue;
                };
                let decision = match configuration
                    .objects()
                    .get(&ObjectIdentity::ReviewDecision(decision_id.clone()))
                {
                    Some(FirstClassObject::ReviewDecision(decision)) => decision,
                    _ => {
                        return Err(ServiceError::new(
                            "invalid_projection",
                            "current proof references a missing ReviewDecision",
                        ));
                    }
                };
                let packet = match configuration
                    .objects()
                    .get(&ObjectIdentity::ReviewPacket(decision.packet.clone()))
                {
                    Some(FirstClassObject::ReviewPacket(packet)) => packet,
                    _ => {
                        return Err(ServiceError::new(
                            "invalid_projection",
                            "current proof references a missing ReviewPacket",
                        ));
                    }
                };
                let dependencies = dependency_view(configuration, packet)
                    .map_err(|error| ServiceError::new("invalid_projection", error.to_string()))?;
                required.extend(review_evidence_ids(configuration, packet, &dependencies)?);
            }
        }
        TransitionInput::CheckWait(input) => {
            evidence.extend(input.evidence.iter().map(|(id, value)| (id.clone(), value)));
            required.extend(input.judgment.evidence_set.iter().cloned());
        }
        TransitionInput::SealAttempt(input) => {
            required.extend(input.packet.evidence_set.iter().cloned());
            let dependencies = dependency_view(configuration, &input.packet)
                .map_err(|error| ServiceError::new("invalid_projection", error.to_string()))?;
            required.extend(review_evidence_ids(
                configuration,
                &input.packet,
                &dependencies,
            )?);
        }
        TransitionInput::Decision(input) => {
            let packet = configuration
                .objects()
                .get(&ObjectIdentity::ReviewPacket(input.decision.packet.clone()));
            let Some(FirstClassObject::ReviewPacket(packet)) = packet else {
                return Err(ServiceError::new(
                    "invalid_projection",
                    "Decision references a missing ReviewPacket",
                ));
            };
            required.extend(packet.evidence_set.iter().cloned());
            let dependencies = dependency_view(configuration, packet)
                .map_err(|error| ServiceError::new("invalid_projection", error.to_string()))?;
            required.extend(review_evidence_ids(configuration, packet, &dependencies)?);
        }
        TransitionInput::ActivateObjective(_)
        | TransitionInput::AddRoute(_)
        | TransitionInput::SelectRoute(_)
        | TransitionInput::StartAttempt(_)
        | TransitionInput::RequestRemap(_)
        | TransitionInput::ReviseObjective(_)
        | TransitionInput::Abandon(_) => {}
    }
    for (identity, object) in configuration.objects().iter() {
        if let (ObjectIdentity::Evidence(id), FirstClassObject::Evidence(value)) =
            (identity, object)
        {
            if required.contains(id) {
                evidence.insert(id.clone(), value);
            }
        }
    }
    if evidence.keys().cloned().collect::<BTreeSet<_>>() != required {
        return Err(ServiceError::new(
            "invalid_projection",
            "transition material references missing Evidence",
        ));
    }
    Ok(evidence
        .values()
        .filter_map(|evidence| match &evidence.observation {
            FrozenObservation::CoreSnapshot(snapshot) => Some(snapshot.clone()),
            FrozenObservation::Inline(_) => None,
        })
        .collect())
}

fn sha256_text(bytes: &[u8]) -> String {
    format_sha256(Sha256::digest(bytes))
}

fn format_sha256(digest: impl AsRef<[u8]>) -> String {
    let digest = digest.as_ref();
    let mut result = String::with_capacity(7 + digest.len() * 2);
    result.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(result, "{byte:02x}").expect("writing to a String cannot fail");
    }
    result
}

fn received_at() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => format!("unix:{}.{:09}", duration.as_secs(), duration.subsec_nanos()),
        Err(error) => format!(
            "unix:-{}.{:09}",
            error.duration().as_secs(),
            error.duration().subsec_nanos()
        ),
    }
}

fn codec_error(error: impl std::fmt::Display) -> ServiceError {
    ServiceError::new("codec_error", error.to_string())
}

fn artifact_error(error: ArtifactError) -> ServiceError {
    let code = match error {
        ArtifactError::MissingBlob(_) | ArtifactError::IntegrityMismatch { .. } => {
            "artifact_integrity_failed"
        }
        _ => "artifact_error",
    };
    ServiceError::new(code, error.to_string())
}

fn audit_issue(
    code: impl Into<String>,
    objective_id: Option<ObjectiveId>,
    detail: impl Into<String>,
) -> AuditIssue {
    AuditIssue {
        code: code.into(),
        objective_id,
        detail: detail.into(),
    }
}

fn audit_status(issues: &[AuditIssue]) -> AuditStatus {
    if issues.is_empty() {
        AuditStatus::Healthy
    } else {
        AuditStatus::Degraded
    }
}

fn audit_issue_page(
    issues: Vec<AuditIssue>,
    requested_limit: Option<u32>,
) -> Result<AuditIssuePage, ServiceError> {
    let limit = audit_issue_limit(requested_limit)?;
    let total = issues.len() as u64;
    let items = issues.into_iter().take(limit).collect::<Vec<_>>();
    Ok(AuditIssuePage {
        returned: items.len() as u32,
        total,
        complete: items.len() as u64 == total,
        items,
    })
}

fn store_error(error: StoreError) -> ServiceError {
    let code = match error {
        StoreError::RequestConflict { .. } => "request_conflict",
        StoreError::StaleHeads { .. } => "stale_heads",
        StoreError::BindingMissing | StoreError::BindingMismatch => "project_binding_failed",
        StoreError::SchemaMismatch(_) => "schema_mismatch",
        StoreError::SingleActiveObjective { .. } => "active_objective_exists",
        StoreError::ProjectionHeadMismatch => "projection_mismatch",
        _ => "store_error",
    };
    ServiceError::new(code, error.to_string())
}

fn objective_id(configuration: &crate::domain::DomainConfiguration) -> Option<ObjectiveId> {
    match configuration.objective_state() {
        ObjectiveState::Idle => None,
        ObjectiveState::Mapping { objective, .. }
        | ObjectiveState::Navigating { objective, .. }
        | ObjectiveState::Achieved { objective, .. }
        | ObjectiveState::Abandoned { objective, .. } => Some(objective.clone()),
    }
}

fn review_evidence_ids(
    configuration: &DomainConfiguration,
    packet: &ReviewPacket,
    dependencies: &BTreeSet<ReviewDecisionId>,
) -> Result<BTreeSet<EvidenceId>, ServiceError> {
    let mut evidence = packet.evidence_set.clone();
    for decision_id in dependencies {
        let decision = match configuration
            .objects()
            .get(&ObjectIdentity::ReviewDecision(decision_id.clone()))
        {
            Some(FirstClassObject::ReviewDecision(decision)) => decision,
            _ => {
                return Err(ServiceError::new(
                    "invalid_projection",
                    "DependencyView references a missing ReviewDecision",
                ));
            }
        };
        let dependency_packet = match configuration
            .objects()
            .get(&ObjectIdentity::ReviewPacket(decision.packet.clone()))
        {
            Some(FirstClassObject::ReviewPacket(packet)) => packet,
            _ => {
                return Err(ServiceError::new(
                    "invalid_projection",
                    "DependencyView decision references a missing ReviewPacket",
                ));
            }
        };
        evidence.extend(dependency_packet.evidence_set.iter().cloned());
    }
    Ok(evidence)
}

fn referenced_snapshots(
    configuration: &crate::domain::DomainConfiguration,
) -> BTreeSet<CoreSnapshot> {
    configuration
        .objects()
        .values()
        .filter_map(|object| match object {
            FirstClassObject::Evidence(evidence) => match &evidence.observation {
                FrozenObservation::CoreSnapshot(snapshot) => Some(snapshot.clone()),
                FrozenObservation::Inline(_) => None,
            },
            _ => None,
        })
        .collect()
}

fn materialize_seal(
    configuration: &crate::domain::DomainConfiguration,
    command: &SealAttemptCommand,
) -> Result<SealAttemptInput, ServiceError> {
    let (stage, attempt) = match configuration.objective_state() {
        ObjectiveState::Navigating {
            navigation: NavState::Attempting { stage, attempt, .. },
            ..
        } => (stage, attempt),
        _ => {
            return Err(ServiceError::new(
                "wrong_state",
                "SealAttempt requires the locked Attempting prestate",
            ));
        }
    };
    if attempt != &command.attempt {
        return Err(ServiceError::new(
            "attempt_mismatch",
            "SealAttempt must bind the current Attempt",
        ));
    }
    let current_attempt = match configuration
        .objects()
        .get(&ObjectIdentity::Attempt(attempt.clone()))
    {
        Some(FirstClassObject::Attempt(value)) => value,
        _ => {
            return Err(ServiceError::new(
                "invalid_projection",
                "current Attempt object is missing",
            ));
        }
    };
    let evidence_set = evidence_universe(configuration, stage, &current_attempt.context)
        .map_err(|error| ServiceError::new("seal_materialization_failed", error.to_string()))?;
    let packet_id = loop {
        let candidate = ReviewPacketId::new(format!("packet-{}", uuid::Uuid::new_v4()));
        if !configuration
            .objects()
            .contains_key(&ObjectIdentity::ReviewPacket(candidate.clone()))
        {
            break candidate;
        }
    };
    Ok(SealAttemptInput {
        packet: ReviewPacket {
            id: packet_id,
            attempt: attempt.clone(),
            stage: stage.clone(),
            context: current_attempt.context.clone(),
            termination: command.seal_reason,
            evidence_set,
        },
        seal_reason: command.seal_reason,
    })
}

fn validate_live_confirmation(
    command: &MutationCommand,
    project_id: &ProjectId,
    heads: &HeadBinding,
) -> Result<(), ServiceError> {
    let mismatch = |message: &'static str| ServiceError::new("confirmation_mismatch", message);
    match command {
        MutationCommand::ActivateObjective(input) => {
            if &input.confirmation.project != project_id {
                return Err(mismatch("Activate confirmation project is not live"));
            }
            if &input.confirmation.heads != heads {
                return Err(mismatch("Activate confirmation heads are stale"));
            }
        }
        MutationCommand::ReviseObjective(input) => {
            if &input.confirmation.project != project_id {
                return Err(mismatch("Revise confirmation project is not live"));
            }
            if &input.confirmation.heads != heads {
                return Err(mismatch("Revise confirmation heads are stale"));
            }
        }
        MutationCommand::Abandon(input) => {
            if &input.confirmation.project != project_id {
                return Err(mismatch("Abandon confirmation project is not live"));
            }
            if &input.confirmation.heads != heads {
                return Err(mismatch("Abandon confirmation heads are stale"));
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod helper_tests {
    use super::*;
    use crate::domain::{
        AbandonConfirmation, AbandonInput, AcceptanceContext, ActivateObjectiveInput,
        AddRouteInput, Attempt, AttemptBound, CanonicalValue, CarryVerdict, Criterion, CriterionId,
        CriterionJudgment, CriterionScope, DecisionInput, EvidenceClaim, EvidencePurpose,
        EvidenceSubject, HasIdentity, InstallMapInput, MapRevision, ObjectiveConfirmation,
        ObjectiveConfirmationAction, ObjectiveSpec, RecordEvidenceInput, RequestRemapInput,
        ReviewAction, ReviewDecision, ReviseObjectiveInput, Route, RouteId, SealReason,
        SelectRouteInput, Stage, StageContract, StageDependency, StageKind, StartAttemptInput,
        StructuralContext, acceptance_context, initial_configuration, structural_context,
    };
    use rusqlite::Connection;
    use std::fs;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use uuid::Uuid;

    struct TestProject {
        root: PathBuf,
        service: CoreService,
        binding: ProjectBinding,
    }

    #[derive(Clone, Debug, Serialize)]
    struct ModelExternalCandidate {
        observation: String,
        source_note: String,
    }

    enum AchievedLane {
        DirectMainObservation,
        InspectedCandidate(ModelExternalCandidate),
    }

    struct TestCurrentContext {
        stage: StageId,
        context: AcceptanceContext,
    }

    impl TestProject {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("mobius-service-test-{}", Uuid::new_v4()));
            fs::create_dir(&root).expect("create service test project");
            let service = CoreService::new(vec![root.clone()]);
            let initialized = service
                .project_init(ProjectInitRequest {
                    project_root: root.clone(),
                    request_id: "project-init".to_owned(),
                })
                .expect("initialize project");
            let binding = ProjectBinding {
                project_root: root.clone(),
                project_id: initialized.project_id,
            };
            Self {
                root,
                service,
                binding,
            }
        }

        fn apply(
            &self,
            heads: HeadBinding,
            request_id: &str,
            command: MutationCommand,
        ) -> Result<ApplyTransitionResponse, ServiceError> {
            self.service
                .apply_transition(ApplyTransitionRequest {
                    project_root: self.root.clone(),
                    project_id: self.binding.project_id.clone(),
                    expected_heads: heads,
                    request_id: request_id.to_owned(),
                    command,
                })
                .map(|outcome| outcome.response)
        }

        fn trail(&self, objective: &ObjectiveId) -> Vec<TrailFact> {
            let connection = Connection::open(self.database_path()).expect("open test database");
            let mut statement = connection
                .prepare(
                    "SELECT event_bytes FROM trail_events
                     WHERE objective_id = ?1 ORDER BY objective_seq",
                )
                .expect("prepare Trail test read");
            statement
                .query_map([objective.as_str()], |row| row.get::<_, Vec<u8>>(0))
                .expect("query Trail test bytes")
                .map(|row| {
                    decode_trail_fact(&row.expect("read Trail test bytes"))
                        .expect("decode Trail test fact")
                })
                .collect()
        }

        fn database_path(&self) -> PathBuf {
            self.root.join(".mobius/mobius.sqlite3")
        }

        fn event_count(&self) -> u64 {
            let connection = Connection::open(self.database_path()).expect("open test database");
            connection
                .query_row("SELECT COUNT(*) FROM trail_events", [], |row| row.get(0))
                .expect("count Trail events")
        }

        fn heads(&self, objective: &ObjectiveId) -> HeadBinding {
            let connection = Connection::open(self.database_path()).expect("open test database");
            let project_seq = connection
                .query_row(
                    "SELECT project_seq FROM schema_meta WHERE singleton = 1",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .expect("read project head");
            let objective_seq = connection
                .query_row(
                    "SELECT objective_seq FROM objective_streams WHERE objective_id = ?1",
                    [objective.as_str()],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap_or(0);
            HeadBinding {
                expected_project_seq: project_seq,
                expected_objective_seq: objective_seq,
            }
        }

        fn blob_path(&self, snapshot: &CoreSnapshot) -> PathBuf {
            self.root
                .join(".mobius/artifacts/blobs")
                .join(snapshot.digest.canonical_sha256_hex().unwrap())
        }
    }

    impl Drop for TestProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn heads(project: u64, objective: u64) -> HeadBinding {
        HeadBinding {
            expected_project_seq: project,
            expected_objective_seq: objective,
        }
    }

    fn criterion(objective: &ObjectiveId) -> Criterion {
        Criterion {
            id: CriterionId::new(format!("criterion-{}", objective.as_str())),
            statement: "the assembled service result is observable".to_owned(),
            verification_rule: "inspect the frozen observation".to_owned(),
            scope: CriterionScope::Local,
        }
    }

    fn specification(objective: &ObjectiveId, outcome: &str) -> ObjectiveSpec {
        let criterion = criterion(objective);
        ObjectiveSpec {
            objective: objective.clone(),
            revision: 1,
            intended_outcome: outcome.to_owned(),
            criteria: BTreeMap::from([(criterion.identity(), criterion)]),
            boundaries: BTreeSet::from(["remain inside the admitted project".to_owned()]),
            excluded_claims: BTreeSet::from(["unverified completion".to_owned()]),
        }
    }

    fn activate_command(
        project: &ProjectId,
        objective: &ObjectiveId,
        expected: HeadBinding,
        outcome: &str,
    ) -> MutationCommand {
        let objective_spec = specification(objective, outcome);
        activate_spec_command(project, objective_spec, expected)
    }

    fn activate_spec_command(
        project: &ProjectId,
        objective_spec: ObjectiveSpec,
        expected: HeadBinding,
    ) -> MutationCommand {
        MutationCommand::ActivateObjective(ActivateObjectiveInput {
            confirmation: ObjectiveConfirmation {
                project: project.clone(),
                action: ObjectiveConfirmationAction::Activate,
                objective_spec: objective_spec.identity(),
                confirmed_payload: Box::new(objective_spec.clone()),
                heads: expected,
                confirmed: true,
            },
            objective_spec,
        })
    }

    fn revise_spec_command(
        project: &ProjectId,
        objective_spec: ObjectiveSpec,
        expected: HeadBinding,
    ) -> MutationCommand {
        MutationCommand::ReviseObjective(ReviseObjectiveInput {
            confirmation: ObjectiveConfirmation {
                project: project.clone(),
                action: ObjectiveConfirmationAction::Revise,
                objective_spec: objective_spec.identity(),
                confirmed_payload: Box::new(objective_spec.clone()),
                heads: expected,
                confirmed: true,
            },
            objective_spec,
        })
    }

    fn abandon_command(
        project: &ProjectId,
        objective: &ObjectiveId,
        expected: HeadBinding,
        reason: &str,
    ) -> MutationCommand {
        MutationCommand::Abandon(AbandonInput {
            reason: reason.to_owned(),
            confirmation: AbandonConfirmation {
                project: project.clone(),
                objective: objective.clone(),
                reason: reason.to_owned(),
                heads: expected,
                confirmed: true,
            },
        })
    }

    fn stage(objective: &ObjectiveId) -> Stage {
        Stage {
            id: StageId::new(format!("stage-{}", objective.as_str())),
            name: "Assembled service stage".to_owned(),
            outcome: "a verified service outcome".to_owned(),
            output: "service output".to_owned(),
            kind: StageKind::Ordinary,
        }
    }

    fn map_revision(objective: &ObjectiveId) -> MapRevision {
        let stage = stage(objective);
        let criterion = criterion(objective);
        let contract = StageContract {
            outcome: stage.outcome.clone(),
            criteria: BTreeSet::from([criterion.identity()]),
            objective_boundaries: BTreeSet::from(["remain inside the admitted project".to_owned()]),
            output: stage.output.clone(),
        };
        MapRevision {
            objective_spec: specification(objective, "complete the assembled flow").identity(),
            revision: 1,
            stages: BTreeMap::from([(stage.identity(), stage.clone())]),
            criteria: BTreeMap::from([(criterion.identity(), criterion.clone())]),
            dependencies: BTreeSet::new(),
            priorities: BTreeMap::from([(stage.identity(), 1)]),
            owners: BTreeMap::from([(criterion.identity(), stage.identity())]),
            contracts: BTreeMap::from([(stage.identity(), contract)]),
        }
    }

    fn historical_criterion(objective: &ObjectiveId, suffix: &str) -> Criterion {
        Criterion {
            id: CriterionId::new(format!("criterion-{}-{suffix}", objective.as_str())),
            statement: format!("historical criterion {suffix} is observable"),
            verification_rule: format!("inspect frozen observation {suffix}"),
            scope: CriterionScope::Local,
        }
    }

    fn historical_stage(objective: &ObjectiveId, suffix: &str) -> Stage {
        Stage {
            id: StageId::new(format!("stage-{}-{suffix}", objective.as_str())),
            name: format!("Historical Stage {suffix}"),
            outcome: format!("historical outcome {suffix}"),
            output: format!("historical output {suffix}"),
            kind: StageKind::Ordinary,
        }
    }

    fn historical_specification(objective: &ObjectiveId, revision: u64) -> ObjectiveSpec {
        let first = historical_criterion(objective, "a");
        let second = historical_criterion(objective, "b");
        ObjectiveSpec {
            objective: objective.clone(),
            revision,
            intended_outcome: "preserve revision-specific report structure".to_owned(),
            criteria: BTreeMap::from([(first.identity(), first), (second.identity(), second)]),
            boundaries: BTreeSet::from(["local report only".to_owned()]),
            excluded_claims: BTreeSet::from(["current structure is all history".to_owned()]),
        }
    }

    fn historical_map(
        objective: &ObjectiveId,
        objective_spec_revision: u64,
        revision: u64,
    ) -> MapRevision {
        let first_stage = historical_stage(objective, "a");
        let second_stage = historical_stage(objective, "b");
        let first_criterion = historical_criterion(objective, "a");
        let second_criterion = historical_criterion(objective, "b");
        let swapped = revision == 2;
        let first_owner = if swapped {
            second_stage.id.clone()
        } else {
            first_stage.id.clone()
        };
        let second_owner = if swapped {
            first_stage.id.clone()
        } else {
            second_stage.id.clone()
        };
        let first_contract = StageContract {
            outcome: first_stage.outcome.clone(),
            criteria: BTreeSet::from([if swapped {
                second_criterion.id.clone()
            } else {
                first_criterion.id.clone()
            }]),
            objective_boundaries: BTreeSet::from(["local report only".to_owned()]),
            output: first_stage.output.clone(),
        };
        let second_contract = StageContract {
            outcome: second_stage.outcome.clone(),
            criteria: BTreeSet::from([if swapped {
                first_criterion.id.clone()
            } else {
                second_criterion.id.clone()
            }]),
            objective_boundaries: BTreeSet::from(["local report only".to_owned()]),
            output: second_stage.output.clone(),
        };
        MapRevision {
            objective_spec: historical_specification(objective, objective_spec_revision).identity(),
            revision,
            stages: BTreeMap::from([
                (first_stage.identity(), first_stage.clone()),
                (second_stage.identity(), second_stage.clone()),
            ]),
            criteria: BTreeMap::from([
                (first_criterion.identity(), first_criterion.clone()),
                (second_criterion.identity(), second_criterion.clone()),
            ]),
            dependencies: BTreeSet::new(),
            priorities: BTreeMap::from([
                (first_stage.identity(), if swapped { 20 } else { 1 }),
                (second_stage.identity(), if swapped { 10 } else { 2 }),
            ]),
            owners: BTreeMap::from([
                (first_criterion.identity(), first_owner),
                (second_criterion.identity(), second_owner),
            ]),
            contracts: BTreeMap::from([
                (first_stage.identity(), first_contract),
                (second_stage.identity(), second_contract),
            ]),
        }
    }

    fn install_map_command(objective: &ObjectiveId) -> MutationCommand {
        let map = map_revision(objective);
        install_specific_map_command(map)
    }

    fn install_specific_map_command(map: MapRevision) -> MutationCommand {
        MutationCommand::InstallMap(InstallMapInput {
            cover: crate::domain::CoverJudgment {
                map: map.identity(),
                objective_spec: map.objective_spec.clone(),
                verdict: crate::domain::CoverVerdict::Covered,
                rationale: "the single Stage covers the confirmed Objective".to_owned(),
            },
            map,
            initial_routes: BTreeMap::new(),
            carry: BTreeMap::<StageId, CarryVerdict>::new(),
        })
    }

    fn current_context(project: &TestProject, objective: &ObjectiveId) -> TestCurrentContext {
        let configuration = configuration(project, objective);
        let stage = match configuration.objective_state() {
            ObjectiveState::Navigating { navigation, .. } => match navigation {
                NavState::SeekingRoute { stage }
                | NavState::Ready { stage, .. }
                | NavState::Attempting { stage, .. }
                | NavState::Reviewing { stage, .. }
                | NavState::Waiting { stage, .. } => stage.clone(),
            },
            state => panic!("expected navigating state, got {state:?}"),
        };
        let context = acceptance_context(&configuration, &stage).expect("derive current context");
        TestCurrentContext { stage, context }
    }

    fn configuration(project: &TestProject, objective: &ObjectiveId) -> DomainConfiguration {
        replay(&project.trail(objective)).expect("replay test Objective")
    }

    fn object_value(
        project: &TestProject,
        objective: &ObjectiveId,
        identity: ObjectIdentity,
    ) -> FirstClassObject {
        configuration(project, objective)
            .objects()
            .get(&identity)
            .cloned()
            .expect("read object from replayed Trail")
    }

    fn review_packet(project: &TestProject, objective: &ObjectiveId) -> ReviewPacket {
        let configuration = configuration(project, objective);
        let packet = match configuration.objective_state() {
            ObjectiveState::Navigating {
                navigation: NavState::Reviewing { packet, .. },
                ..
            } => packet.clone(),
            state => panic!("expected Reviewing state, got {state:?}"),
        };
        match object_value(project, objective, ObjectIdentity::ReviewPacket(packet)) {
            FirstClassObject::ReviewPacket(packet) => packet,
            object => panic!("expected ReviewPacket object, got {object:?}"),
        }
    }

    fn route_command(
        objective: &ObjectiveId,
        stage: &StageId,
        context: StructuralContext,
    ) -> (RouteId, MutationCommand) {
        let route = Route {
            id: RouteId::new(format!("route-{}", objective.as_str())),
            stage: stage.clone(),
            structural_context: context,
            hypothesis: "the bounded attempt reaches the Stage outcome".to_owned(),
            assumptions: BTreeSet::from(["the local test remains available".to_owned()]),
            rationale: "assembled CoreService fixture".to_owned(),
        };
        (
            route.id.clone(),
            MutationCommand::AddRoute(AddRouteInput { route }),
        )
    }

    fn assert_error_code<T>(result: Result<T, ServiceError>, expected: &'static str) {
        let error = result.err().expect("operation must fail closed");
        assert_eq!(error.code, expected, "unexpected service error: {error}");
    }

    fn run_main_agent_achieved_lane(lane: AchievedLane) {
        let objective = ObjectiveId::new(match &lane {
            AchievedLane::DirectMainObservation => "objective-achieved-direct",
            AchievedLane::InspectedCandidate(_) => "objective-achieved-candidate",
        });
        let project = TestProject::new();
        let mut mutation_wires = Vec::new();
        let (stage, decision_id) = {
            let mut apply = |expected: HeadBinding, request_id: &str, command: MutationCommand| {
                let response = project
                    .apply(expected, request_id, command)
                    .expect("main-agent mutation must commit");
                mutation_wires.push(serde_json::to_value(&response).unwrap());
                response
            };

            apply(
                heads(0, 0),
                "activate-achieved",
                activate_command(
                    &project.binding.project_id,
                    &objective,
                    heads(0, 0),
                    "reach an evidence-backed achieved state",
                ),
            );
            apply(
                heads(1, 1),
                "install-achieved-map",
                install_map_command(&objective),
            );
            let map = map_revision(&objective);
            let stage = stage(&objective);
            let (route, add_route) = route_command(
                &objective,
                &stage.id,
                structural_context(&map, &stage.id).expect("derive Route StructuralContext"),
            );
            apply(heads(2, 2), "add-achieved-route", add_route);
            apply(
                heads(3, 3),
                "select-achieved-route",
                MutationCommand::SelectRoute(SelectRouteInput {
                    route: route.clone(),
                }),
            );

            let context_value = current_context(&project, &objective).context;
            let attempt = crate::domain::AttemptId::new("attempt-achieved");
            apply(
                heads(4, 4),
                "start-achieved-attempt",
                MutationCommand::StartAttempt(StartAttemptInput {
                    attempt: Attempt {
                        id: attempt.clone(),
                        route,
                        ordinal: 1,
                        bound: AttemptBound::TerminationCondition(
                            "one inspected frozen observation exists".to_owned(),
                        ),
                        context: context_value.clone(),
                    },
                }),
            );

            let (observation, provenance) = match lane {
                AchievedLane::DirectMainObservation => (
                    FrozenObservation::Inline(CanonicalValue::String(
                        "direct main-agent observation".to_owned(),
                    )),
                    CanonicalValue::String("main agent inspected the result directly".to_owned()),
                ),
                AchievedLane::InspectedCandidate(candidate) => {
                    let inspected = candidate
                        .observation
                        .strip_prefix("observed: ")
                        .filter(|value| !value.trim().is_empty())
                        .expect("main agent rejects an uninspectable candidate");
                    assert_eq!(candidate.source_note, "model-external candidate");
                    (
                        FrozenObservation::Inline(CanonicalValue::String(inspected.to_owned())),
                        CanonicalValue::Object(BTreeMap::from([(
                            "source_note".to_owned(),
                            CanonicalValue::String(candidate.source_note),
                        )])),
                    )
                }
            };
            let evidence = EvidenceId::new("evidence-achieved");
            let criterion = criterion(&objective).identity();
            apply(
                heads(5, 5),
                "record-achieved-evidence",
                MutationCommand::RecordEvidence(RecordEvidenceInput {
                    evidence: Evidence {
                        id: evidence.clone(),
                        subject: EvidenceSubject::Attempt(attempt.clone()),
                        context: context_value,
                        purpose: EvidencePurpose::StageReview,
                        claims: BTreeMap::from([(criterion.clone(), EvidenceClaim::Supports)]),
                        observation,
                        provenance,
                    },
                }),
            );

            let seal_command = MutationCommand::SealAttempt(SealAttemptCommand {
                attempt: attempt.clone(),
                seal_reason: SealReason::Submitted,
            });
            let external_seal = serde_json::to_value(&seal_command).unwrap();
            assert_eq!(
                external_seal,
                serde_json::json!({
                    "seal_attempt": {
                        "attempt": "attempt-achieved",
                        "seal_reason": "submitted"
                    }
                })
            );
            apply(heads(6, 6), "seal-achieved-attempt", seal_command);

            assert_eq!(
                project.trail(&objective).last().map(TrailFact::transition),
                Some(TransitionKind::SealAttempt)
            );

            let packet = review_packet(&project, &objective);
            assert_eq!(packet.attempt, attempt);
            assert_eq!(packet.evidence_set, BTreeSet::from([evidence]));

            let judgments = packet
                .context
                .structural
                .contract
                .criteria
                .iter()
                .cloned()
                .map(|criterion| (criterion, CriterionJudgment::Satisfied))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(
                judgments.keys().cloned().collect::<BTreeSet<_>>(),
                packet.context.structural.contract.criteria
            );
            assert!(
                judgments
                    .values()
                    .all(|judgment| *judgment == CriterionJudgment::Satisfied)
            );
            let decision_id = ReviewDecisionId::new("decision-achieved");
            apply(
                heads(7, 7),
                "accept-achieved-packet",
                MutationCommand::Decision(DecisionInput {
                    decision: ReviewDecision {
                        id: decision_id.clone(),
                        packet: packet.id,
                        judgments,
                        findings: BTreeSet::new(),
                        action: ReviewAction::Accept,
                    },
                }),
            );
            (stage, decision_id)
        };

        assert_eq!(project.heads(&objective), heads(8, 8));
        match configuration(&project, &objective).objective_state() {
            ObjectiveState::Achieved {
                objective: achieved,
                manifest,
                ..
            } => {
                assert_eq!(achieved, &objective);
                assert_eq!(manifest.len(), 1);
                assert_eq!(manifest, &BTreeMap::from([(stage.id.clone(), decision_id)]));
            }
            state => panic!("expected fresh Achieved state, got {state:?}"),
        }

        for wire in &mutation_wires {
            let encoded = serde_json::to_string(wire).unwrap();
            for field in [
                "csv",
                "view",
                "views",
                "report",
                "refresh",
                "report_path",
                "generation_path",
                "current_path",
                "refresh_task",
                "file_list",
            ] {
                assert!(
                    !encoded.contains(&format!("\"{field}\":")),
                    "default Core response leaked presentation field {field}: {encoded}"
                );
            }
        }
    }

    #[test]
    fn evidence_purpose_remains_a_domain_value() {
        assert_ne!(
            EvidencePurpose::StageReview,
            EvidencePurpose::WaitResolution
        );
    }

    #[test]
    fn direct_main_agent_lane_reaches_fresh_achieved_through_core_materialized_review() {
        run_main_agent_achieved_lane(AchievedLane::DirectMainObservation);
    }

    #[test]
    fn inspected_model_external_candidate_requires_translation_then_reaches_achieved() {
        let candidate = ModelExternalCandidate {
            observation: "observed: candidate result inspected by main agent".to_owned(),
            source_note: "model-external candidate".to_owned(),
        };
        let candidate_wire = serde_json::to_value(&candidate).unwrap();
        assert_eq!(
            candidate_wire,
            serde_json::json!({
                "observation": "observed: candidate result inspected by main agent",
                "source_note": "model-external candidate"
            })
        );
        assert!(
            serde_json::from_value::<MutationCommand>(candidate_wire.clone()).is_err(),
            "a model-external candidate must not be a Core mutation"
        );
        assert!(
            serde_json::from_value::<Evidence>(candidate_wire).is_err(),
            "a model-external candidate must not already be typed Evidence"
        );

        run_main_agent_achieved_lane(AchievedLane::InspectedCandidate(candidate));
    }

    #[test]
    fn report_prefix_index_tracks_exact_global_heads_across_other_objectives() {
        let project = TestProject::new();
        let objective = ObjectiveId::new("objective-report-prefix-history");
        project
            .apply(
                heads(0, 0),
                "activate-report-prefix-history",
                activate_command(
                    &project.binding.project_id,
                    &objective,
                    heads(0, 0),
                    "retain exact report prefix history",
                ),
            )
            .expect("activate report-prefix Objective");
        project
            .apply(
                heads(1, 1),
                "abandon-report-prefix-history",
                abandon_command(
                    &project.binding.project_id,
                    &objective,
                    heads(1, 1),
                    "finish the report-prefix fixture",
                ),
            )
            .expect("abandon report-prefix Objective");
        let terminal = project
            .service
            .report_snapshot(&project.binding, &objective)
            .expect("snapshot terminal report prefix");

        let other = ObjectiveId::new("objective-report-prefix-other");
        project
            .apply(
                heads(2, 0),
                "activate-report-prefix-other",
                activate_command(
                    &project.binding.project_id,
                    &other,
                    heads(2, 0),
                    "advance the global head independently",
                ),
            )
            .expect("activate other Objective");
        let mut revised_other = specification(&other, "advance the global head again");
        revised_other.revision = 2;
        project
            .apply(
                heads(3, 1),
                "revise-report-prefix-other",
                revise_spec_command(&project.binding.project_id, revised_other, heads(3, 1)),
            )
            .expect("revise other Objective");

        let snapshot = project
            .service
            .report_snapshot(&project.binding, &objective)
            .expect("snapshot old Objective at the new global head");
        assert_eq!(
            snapshot.heads,
            ReportHeads {
                project_seq: 4,
                objective_seq: 2,
            }
        );
        for project_seq in [2, 3, 4] {
            assert_eq!(
                snapshot.trail_prefix_digests.get(&ReportHeads {
                    project_seq,
                    objective_seq: 2,
                }),
                Some(&terminal.trail_digest),
                "an unrelated event must preserve the exact old Objective prefix"
            );
        }
        assert_eq!(snapshot.trail_digest, terminal.trail_digest);
    }

    #[test]
    fn report_structure_is_revision_specific_and_routes_use_only_a_truthful_current_marker() {
        let project = TestProject::new();
        let objective = ObjectiveId::new("objective-report-history");
        project
            .apply(
                heads(0, 0),
                "activate-history",
                activate_spec_command(
                    &project.binding.project_id,
                    historical_specification(&objective, 1),
                    heads(0, 0),
                ),
            )
            .expect("activate historical report Objective");

        let mapping_snapshot = project
            .service
            .report_snapshot(&project.binding, &objective)
            .expect("report Mapping snapshot");
        assert_eq!(
            mapping_snapshot.criteria.columns,
            [
                "objective_spec_revision",
                "map_revision",
                "map_current",
                "criterion_id",
                "statement",
                "verification_rule",
                "scope",
                "owner_stage",
            ]
        );
        assert_eq!(mapping_snapshot.criteria.rows.len(), 2);
        assert!(mapping_snapshot.criteria.rows.iter().all(|row| {
            row[0] == ReportCell::Integer(1)
                && row[1] == ReportCell::Empty
                && row[2] == ReportCell::Boolean(false)
        }));

        let first_map = historical_map(&objective, 1, 1);
        project
            .apply(
                heads(1, 1),
                "install-history-map-1",
                install_specific_map_command(first_map),
            )
            .expect("install first historical Map");
        project
            .apply(
                heads(2, 2),
                "request-history-remap",
                MutationCommand::RequestRemap(RequestRemapInput {
                    reason: "exercise the historical view".to_owned(),
                }),
            )
            .expect("request remap");
        project
            .apply(
                heads(3, 3),
                "revise-history-spec",
                revise_spec_command(
                    &project.binding.project_id,
                    historical_specification(&objective, 2),
                    heads(3, 3),
                ),
            )
            .expect("revise ObjectiveSpec before the second Map");

        let revised_mapping_snapshot = project
            .service
            .report_snapshot(&project.binding, &objective)
            .expect("report revised Mapping snapshot before Map 2");
        assert_eq!(revised_mapping_snapshot.criteria.rows.len(), 4);
        assert_eq!(
            revised_mapping_snapshot
                .criteria
                .rows
                .iter()
                .filter(|row| {
                    row[0] == ReportCell::Integer(1)
                        && row[1] == ReportCell::Integer(1)
                        && row[2] == ReportCell::Boolean(false)
                })
                .count(),
            2,
            "revision 1 Criteria remain mapped to the historical Map"
        );
        assert_eq!(
            revised_mapping_snapshot
                .criteria
                .rows
                .iter()
                .filter(|row| {
                    row[0] == ReportCell::Integer(2)
                        && row[1] == ReportCell::Empty
                        && row[2] == ReportCell::Boolean(false)
                })
                .count(),
            2,
            "revision 2 Criteria remain map-empty before Map 2"
        );

        let second_map = historical_map(&objective, 2, 2);
        project
            .apply(
                heads(4, 4),
                "install-history-map-2",
                install_specific_map_command(second_map.clone()),
            )
            .expect("install second historical Map");

        let context = current_context(&project, &objective);
        let (route, add_route) = route_command(
            &objective,
            &context.stage,
            structural_context(&second_map, &context.stage)
                .expect("derive current Route StructuralContext"),
        );
        project
            .apply(heads(5, 5), "add-history-route", add_route)
            .expect("add current Route");
        project
            .apply(
                heads(6, 6),
                "select-history-route",
                MutationCommand::SelectRoute(SelectRouteInput {
                    route: route.clone(),
                }),
            )
            .expect("select current Route");

        let snapshot = project
            .service
            .report_snapshot(&project.binding, &objective)
            .expect("report historical structure");
        assert_eq!(
            snapshot.stages.columns,
            [
                "objective_spec_revision",
                "map_revision",
                "map_current",
                "stage_id",
                "name",
                "kind",
                "state",
                "outcome",
                "output",
                "priority",
            ]
        );
        assert_eq!(snapshot.stages.rows.len(), 4);
        let stage_a = historical_stage(&objective, "a").id;
        let revision_one_stage = snapshot
            .stages
            .rows
            .iter()
            .find(|row| {
                row[0] == ReportCell::Integer(1)
                    && row[1] == ReportCell::Integer(1)
                    && row[3] == ReportCell::Text(stage_a.as_str().to_owned())
            })
            .expect("revision one Stage row");
        assert_eq!(revision_one_stage[2], ReportCell::Boolean(false));
        assert_eq!(revision_one_stage[9], ReportCell::Integer(1));
        let revision_two_stage = snapshot
            .stages
            .rows
            .iter()
            .find(|row| {
                row[0] == ReportCell::Integer(2)
                    && row[1] == ReportCell::Integer(2)
                    && row[3] == ReportCell::Text(stage_a.as_str().to_owned())
            })
            .expect("revision two Stage row");
        assert_eq!(revision_two_stage[2], ReportCell::Boolean(true));
        assert_eq!(revision_two_stage[9], ReportCell::Integer(20));

        assert_eq!(snapshot.criteria.rows.len(), 4);
        let criterion_a = historical_criterion(&objective, "a").id;
        let revision_one_criterion = snapshot
            .criteria
            .rows
            .iter()
            .find(|row| {
                row[0] == ReportCell::Integer(1)
                    && row[1] == ReportCell::Integer(1)
                    && row[3] == ReportCell::Text(criterion_a.as_str().to_owned())
            })
            .expect("revision one Criterion row");
        assert_eq!(
            revision_one_criterion[7],
            ReportCell::Text(stage_a.as_str().to_owned())
        );
        let revision_two_criterion = snapshot
            .criteria
            .rows
            .iter()
            .find(|row| {
                row[0] == ReportCell::Integer(2)
                    && row[1] == ReportCell::Integer(2)
                    && row[3] == ReportCell::Text(criterion_a.as_str().to_owned())
            })
            .expect("revision two Criterion row");
        assert_eq!(
            revision_two_criterion[7],
            ReportCell::Text(historical_stage(&objective, "b").id.as_str().to_owned())
        );

        assert_eq!(
            snapshot.routes.columns,
            [
                "route_id",
                "stage_id",
                "status",
                "route_current",
                "hypothesis",
                "assumption",
                "rationale",
            ]
        );
        assert_eq!(snapshot.routes.rows.len(), 1);
        assert_eq!(snapshot.routes.rows[0][0], route.as_str().into());
        assert_eq!(snapshot.routes.rows[0][3], ReportCell::Boolean(true));
    }

    #[test]
    fn human_confirmation_binding_failures_are_atomic_at_the_service_boundary() {
        let project = TestProject::new();
        let objective = ObjectiveId::new("objective-confirmation-binding");
        let other_objective = ObjectiveId::new("objective-confirmation-other");
        let other_project = ProjectId::new("project-confirmation-other");
        let initial_heads = heads(0, 0);

        let assert_fail_closed =
            |expected_heads: &HeadBinding,
             cases: Vec<(&str, MutationCommand, &'static str, &'static str)>| {
                for (request_id, command, expected_code, expected_detail) in cases {
                    let before_events = project.event_count();
                    let before_heads = project.heads(&objective);
                    assert_eq!(
                        &before_heads, expected_heads,
                        "fixture heads drifted before {request_id}"
                    );

                    let error = project
                        .apply(expected_heads.clone(), request_id, command)
                        .expect_err("mismatched human confirmation must fail closed");
                    assert_eq!(
                        error.code, expected_code,
                        "unexpected rejection for {request_id}: {error}"
                    );
                    assert!(
                        error.message.contains(expected_detail),
                        "{request_id} did not report {expected_detail}: {error}"
                    );
                    assert_eq!(
                        project.event_count(),
                        before_events,
                        "{request_id} appended a Trail fact"
                    );
                    assert_eq!(
                        project.heads(&objective),
                        before_heads,
                        "{request_id} changed project or Objective heads"
                    );
                }
            };

        let activation = activate_command(
            &project.binding.project_id,
            &objective,
            initial_heads.clone(),
            "bind the confirmed activation exactly",
        );
        let mut activation_wrong_project = activation.clone();
        let MutationCommand::ActivateObjective(input) = &mut activation_wrong_project else {
            unreachable!("activation fixture")
        };
        input.confirmation.project = other_project.clone();
        let mut activation_wrong_heads = activation.clone();
        let MutationCommand::ActivateObjective(input) = &mut activation_wrong_heads else {
            unreachable!("activation fixture")
        };
        input.confirmation.heads = heads(7, 7);
        let mut activation_wrong_spec = activation.clone();
        let MutationCommand::ActivateObjective(input) = &mut activation_wrong_spec else {
            unreachable!("activation fixture")
        };
        input.confirmation.objective_spec.revision += 1;
        let mut activation_wrong_payload = activation.clone();
        let MutationCommand::ActivateObjective(input) = &mut activation_wrong_payload else {
            unreachable!("activation fixture")
        };
        input.confirmation.confirmed_payload.intended_outcome =
            "a payload the user did not confirm".to_owned();

        assert_fail_closed(
            &initial_heads,
            vec![
                (
                    "activate-wrong-confirmation-project",
                    activation_wrong_project,
                    "confirmation_mismatch",
                    "Activate confirmation project",
                ),
                (
                    "activate-wrong-confirmation-heads",
                    activation_wrong_heads,
                    "confirmation_mismatch",
                    "Activate confirmation heads",
                ),
                (
                    "activate-wrong-confirmation-spec",
                    activation_wrong_spec,
                    "transition_rejected",
                    "confirmation_identity_mismatch",
                ),
                (
                    "activate-wrong-confirmation-payload",
                    activation_wrong_payload,
                    "transition_rejected",
                    "confirmation_payload_mismatch",
                ),
            ],
        );

        project
            .apply(initial_heads, "activate-confirmation-fixture", activation)
            .expect("activate Objective for Revise and Abandon cases");
        let active_heads = heads(1, 1);

        let mut revised_spec =
            specification(&objective, "bind the confirmed Objective revision exactly");
        revised_spec.revision = 2;
        let revision = revise_spec_command(
            &project.binding.project_id,
            revised_spec,
            active_heads.clone(),
        );
        let mut revision_wrong_project = revision.clone();
        let MutationCommand::ReviseObjective(input) = &mut revision_wrong_project else {
            unreachable!("revision fixture")
        };
        input.confirmation.project = other_project.clone();
        let mut revision_wrong_heads = revision.clone();
        let MutationCommand::ReviseObjective(input) = &mut revision_wrong_heads else {
            unreachable!("revision fixture")
        };
        input.confirmation.heads = heads(8, 8);
        let mut revision_wrong_payload = revision;
        let MutationCommand::ReviseObjective(input) = &mut revision_wrong_payload else {
            unreachable!("revision fixture")
        };
        input.confirmation.confirmed_payload.intended_outcome =
            "an unconfirmed revision payload".to_owned();

        let abandonment = abandon_command(
            &project.binding.project_id,
            &objective,
            active_heads.clone(),
            "stop only after exact confirmation",
        );
        let mut abandonment_wrong_objective = abandonment.clone();
        let MutationCommand::Abandon(input) = &mut abandonment_wrong_objective else {
            unreachable!("abandonment fixture")
        };
        input.confirmation.objective = other_objective;
        let mut abandonment_wrong_reason = abandonment.clone();
        let MutationCommand::Abandon(input) = &mut abandonment_wrong_reason else {
            unreachable!("abandonment fixture")
        };
        input.confirmation.reason = "a reason the user did not confirm".to_owned();
        let mut abandonment_wrong_heads = abandonment.clone();
        let MutationCommand::Abandon(input) = &mut abandonment_wrong_heads else {
            unreachable!("abandonment fixture")
        };
        input.confirmation.heads = heads(9, 9);
        let mut abandonment_wrong_project = abandonment;
        let MutationCommand::Abandon(input) = &mut abandonment_wrong_project else {
            unreachable!("abandonment fixture")
        };
        input.confirmation.project = other_project;

        assert_fail_closed(
            &active_heads,
            vec![
                (
                    "revise-wrong-confirmation-project",
                    revision_wrong_project,
                    "confirmation_mismatch",
                    "Revise confirmation project",
                ),
                (
                    "revise-wrong-confirmation-heads",
                    revision_wrong_heads,
                    "confirmation_mismatch",
                    "Revise confirmation heads",
                ),
                (
                    "revise-wrong-confirmation-payload",
                    revision_wrong_payload,
                    "transition_rejected",
                    "confirmation_payload_mismatch",
                ),
                (
                    "abandon-wrong-confirmation-objective",
                    abandonment_wrong_objective,
                    "transition_rejected",
                    "confirmation_identity_mismatch",
                ),
                (
                    "abandon-wrong-confirmation-reason",
                    abandonment_wrong_reason,
                    "transition_rejected",
                    "confirmation_payload_mismatch",
                ),
                (
                    "abandon-wrong-confirmation-heads",
                    abandonment_wrong_heads,
                    "confirmation_mismatch",
                    "Abandon confirmation heads",
                ),
                (
                    "abandon-wrong-confirmation-project",
                    abandonment_wrong_project,
                    "confirmation_mismatch",
                    "Abandon confirmation project",
                ),
            ],
        );
    }

    #[test]
    fn concurrent_activations_on_independent_connections_commit_exactly_one_active_objective() {
        let project = TestProject::new();
        let project_id = project.binding.project_id.clone();
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();

        for (request_id, objective) in [
            (
                "activate-concurrent-left",
                ObjectiveId::new("objective-concurrent-left"),
            ),
            (
                "activate-concurrent-right",
                ObjectiveId::new("objective-concurrent-right"),
            ),
        ] {
            let service = CoreService::new(vec![project.root.clone()]);
            let project_root = project.root.clone();
            let project_id = project_id.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                let command = activate_command(
                    &project_id,
                    &objective,
                    heads(0, 0),
                    "serialize concurrent activation",
                );
                barrier.wait();
                service.apply_transition(ApplyTransitionRequest {
                    project_root,
                    project_id,
                    expected_heads: heads(0, 0),
                    request_id: request_id.to_owned(),
                    command,
                })
            }));
        }

        barrier.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().expect("activation worker must not panic"))
            .collect::<Vec<_>>();
        let committed = results
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .collect::<Vec<_>>();
        let rejected = results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .collect::<Vec<_>>();
        assert_eq!(committed.len(), 1, "exactly one activation must commit");
        assert_eq!(rejected.len(), 1, "the competing activation must fail");
        assert!(
            matches!(rejected[0].code, "stale_heads" | "active_objective_exists"),
            "unexpected concurrent rejection: {}",
            rejected[0]
        );
        assert_eq!(project.event_count(), 1);

        let committed_objective = &committed[0].response.objective_id;
        assert_eq!(project.heads(committed_objective), heads(1, 1));
        assert!(matches!(
            configuration(&project, committed_objective).objective_state(),
            ObjectiveState::Mapping { objective, .. } if objective == committed_objective
        ));

        let audit = project
            .service
            .audit(AuditRequest {
                binding: project.binding.clone(),
                maintenance: None,
                limit: None,
            })
            .expect("audit the concurrent activation result");
        assert_eq!(audit.status, AuditStatus::Healthy);
        assert_eq!(audit.project_seq, 1);
        assert_eq!(audit.checked_objectives, 1);
        assert!(audit.issues.items.is_empty());
    }

    #[test]
    fn idempotent_retry_precedes_heads_and_request_conflicts_while_single_active_is_enforced() {
        let project = TestProject::new();
        let first = ObjectiveId::new("objective-first");
        let second = ObjectiveId::new("objective-second");
        let initial_heads = heads(0, 0);
        let activation = activate_command(
            &project.binding.project_id,
            &first,
            initial_heads.clone(),
            "deliver the first outcome",
        );
        let activated = project
            .apply(initial_heads.clone(), "activate-first", activation.clone())
            .expect("activate first Objective");

        assert_error_code(
            project.apply(
                heads(1, 0),
                "activate-second-while-active",
                activate_command(
                    &project.binding.project_id,
                    &second,
                    heads(1, 0),
                    "deliver the second outcome",
                ),
            ),
            "active_objective_exists",
        );
        assert_eq!(project.event_count(), 1);

        project
            .apply(
                heads(1, 1),
                "abandon-first",
                abandon_command(
                    &project.binding.project_id,
                    &first,
                    heads(1, 1),
                    "finish the retry-order scenario",
                ),
            )
            .expect("advance both heads after activation");

        let retried = project
            .apply(initial_heads.clone(), "activate-first", activation.clone())
            .expect("an exact retry must ignore advanced heads");
        assert_eq!(retried, activated);
        assert_eq!(project.event_count(), 2, "retry must not append an event");

        assert_error_code(
            project.apply(
                initial_heads.clone(),
                "activate-first",
                activate_command(
                    &project.binding.project_id,
                    &first,
                    initial_heads.clone(),
                    "a different payload under the same request id",
                ),
            ),
            "request_conflict",
        );
        assert_error_code(
            project.apply(initial_heads, "stale-new-request", activation),
            "stale_heads",
        );
        assert_eq!(project.event_count(), 2);
    }

    #[test]
    fn projection_damage_blocks_mutation_and_rebuild_restores_the_trail_projection() {
        let project = TestProject::new();
        let objective = ObjectiveId::new("objective-projection-rebuild");
        project
            .apply(
                heads(0, 0),
                "activate",
                activate_command(
                    &project.binding.project_id,
                    &objective,
                    heads(0, 0),
                    "prove projection recovery",
                ),
            )
            .expect("activate Objective");

        let connection = Connection::open(project.database_path()).expect("open test database");
        connection
            .execute(
                "DELETE FROM object_projection WHERE objective_id = ?1 AND object_kind = 'objective'",
                [objective.as_str()],
            )
            .expect("damage only the derived object projection");
        drop(connection);

        let abandonment = abandon_command(
            &project.binding.project_id,
            &objective,
            heads(1, 1),
            "projection was repaired",
        );
        assert_error_code(
            project.apply(heads(1, 1), "abandon-after-rebuild", abandonment.clone()),
            "projection_mismatch",
        );
        assert_eq!(project.event_count(), 1, "failed mutation must roll back");

        let degraded = project
            .service
            .audit(AuditRequest {
                binding: project.binding.clone(),
                maintenance: None,
                limit: None,
            })
            .expect("audit damaged projection");
        assert_eq!(degraded.status, AuditStatus::Degraded);
        assert!(degraded.issues.items.iter().any(|issue| {
            issue.code == "projection_mismatch" && issue.objective_id.as_ref() == Some(&objective)
        }));

        let rebuilt = project
            .service
            .audit(AuditRequest {
                binding: project.binding.clone(),
                maintenance: Some(MaintenanceRequest {
                    action: MaintenanceAction::RebuildProjection,
                    expected_project_seq: 1,
                }),
                limit: None,
            })
            .expect("rebuild projection from Trail");
        assert_eq!(rebuilt.status, AuditStatus::Healthy);
        assert_eq!(
            rebuilt.maintenance_applied,
            Some(MaintenanceAction::RebuildProjection)
        );

        project
            .apply(heads(1, 1), "abandon-after-rebuild", abandonment)
            .expect("the same previously rolled-back request can commit after repair");
        assert_eq!(project.event_count(), 2);
    }

    #[test]
    fn valid_carry_verifies_transitive_snapshot_material_before_it_can_reach_achieved() {
        let project = TestProject::new();
        let objective = ObjectiveId::new("objective-carry-artifact-integrity");
        project
            .apply(
                heads(0, 0),
                "activate-carry",
                activate_command(
                    &project.binding.project_id,
                    &objective,
                    heads(0, 0),
                    "carry a proof whose accepted dependency owns artifact-backed material",
                ),
            )
            .expect("activate carry Objective");

        let dependency_stage = stage(&objective);
        let dependency_criterion = criterion(&objective);
        let carried_stage = Stage {
            id: StageId::new("stage-carry-dependent"),
            name: "Carried dependent".to_owned(),
            outcome: "the dependency-backed Stage is accepted".to_owned(),
            output: "dependency-backed output".to_owned(),
            kind: StageKind::Ordinary,
        };
        let carried_criterion = Criterion {
            id: CriterionId::new("criterion-carry-dependent"),
            statement: "the dependent result is observable".to_owned(),
            verification_rule: "inspect the dependent result".to_owned(),
            scope: CriterionScope::Local,
        };
        let followup_stage = Stage {
            id: StageId::new("stage-carry-followup"),
            name: "Carry follow-up".to_owned(),
            outcome: "a second Stage remains incomplete".to_owned(),
            output: "follow-up output".to_owned(),
            kind: StageKind::Ordinary,
        };
        let followup_criterion = Criterion {
            id: CriterionId::new("criterion-carry-followup"),
            statement: "the follow-up is observable".to_owned(),
            verification_rule: "inspect the follow-up".to_owned(),
            scope: CriterionScope::Local,
        };
        let mut first_map = map_revision(&objective);
        first_map
            .stages
            .insert(carried_stage.identity(), carried_stage.clone());
        first_map
            .stages
            .insert(followup_stage.identity(), followup_stage.clone());
        first_map
            .criteria
            .insert(carried_criterion.identity(), carried_criterion.clone());
        first_map
            .criteria
            .insert(followup_criterion.identity(), followup_criterion.clone());
        first_map.priorities.insert(carried_stage.identity(), 2);
        first_map.priorities.insert(followup_stage.identity(), 3);
        first_map.dependencies.insert(StageDependency {
            dependency: dependency_stage.identity(),
            dependent: carried_stage.identity(),
        });
        first_map
            .owners
            .insert(carried_criterion.identity(), carried_stage.identity());
        first_map
            .owners
            .insert(followup_criterion.identity(), followup_stage.identity());
        first_map.contracts.insert(
            carried_stage.identity(),
            StageContract {
                outcome: carried_stage.outcome.clone(),
                criteria: BTreeSet::from([carried_criterion.identity()]),
                objective_boundaries: BTreeSet::from([
                    "remain inside the admitted project".to_owned()
                ]),
                output: carried_stage.output.clone(),
            },
        );
        first_map.contracts.insert(
            followup_stage.identity(),
            StageContract {
                outcome: followup_stage.outcome.clone(),
                criteria: BTreeSet::from([followup_criterion.identity()]),
                objective_boundaries: BTreeSet::from([
                    "remain inside the admitted project".to_owned()
                ]),
                output: followup_stage.output.clone(),
            },
        );
        project
            .apply(
                heads(1, 1),
                "install-carry-map",
                install_specific_map_command(first_map.clone()),
            )
            .expect("install Map with an incomplete follow-up Stage");

        let structural = structural_context(&first_map, &dependency_stage.id)
            .expect("derive dependency context");
        let (route, add_route) = route_command(&objective, &dependency_stage.id, structural);
        project
            .apply(heads(2, 2), "add-dependency-route", add_route)
            .expect("add dependency Route");
        project
            .apply(
                heads(3, 3),
                "select-dependency-route",
                MutationCommand::SelectRoute(SelectRouteInput {
                    route: route.clone(),
                }),
            )
            .expect("select dependency Route");

        let context = current_context(&project, &objective);
        assert!(context.context.dependency_proofs.is_empty());
        let dependency_attempt = crate::domain::AttemptId::new("attempt-carry-dependency");
        project
            .apply(
                heads(4, 4),
                "start-dependency-attempt",
                MutationCommand::StartAttempt(StartAttemptInput {
                    attempt: Attempt {
                        id: dependency_attempt.clone(),
                        route,
                        ordinal: 1,
                        bound: AttemptBound::TerminationCondition(
                            "the snapshot-backed dependency proof is accepted".to_owned(),
                        ),
                        context: context.context.clone(),
                    },
                }),
            )
            .expect("start dependency Attempt");

        let artifact_bytes = b"snapshot backing the carried proof dependency".to_vec();
        let snapshot = project
            .service
            .capture_artifact(CaptureArtifactRequest {
                binding: project.binding.clone(),
                bytes: artifact_bytes.clone(),
            })
            .expect("capture dependency proof artifact");
        let dependency_evidence = EvidenceId::new("evidence-carry-dependency-artifact");
        project
            .apply(
                heads(5, 5),
                "record-dependency-evidence",
                MutationCommand::RecordEvidence(RecordEvidenceInput {
                    evidence: Evidence {
                        id: dependency_evidence.clone(),
                        subject: EvidenceSubject::Attempt(dependency_attempt.clone()),
                        context: context.context,
                        purpose: EvidencePurpose::StageReview,
                        claims: BTreeMap::from([(
                            dependency_criterion.identity(),
                            EvidenceClaim::Supports,
                        )]),
                        observation: FrozenObservation::CoreSnapshot(snapshot.clone()),
                        provenance: CanonicalValue::String(
                            "transitive carry artifact-integrity transaction regression".to_owned(),
                        ),
                    },
                }),
            )
            .expect("record snapshot-backed dependency Evidence");
        project
            .apply(
                heads(6, 6),
                "seal-dependency-attempt",
                MutationCommand::SealAttempt(SealAttemptCommand {
                    attempt: dependency_attempt,
                    seal_reason: SealReason::Submitted,
                }),
            )
            .expect("seal dependency Attempt");
        let dependency_packet = review_packet(&project, &objective);
        assert_eq!(
            dependency_packet.evidence_set,
            BTreeSet::from([dependency_evidence])
        );
        let dependency_decision = ReviewDecisionId::new("decision-carry-dependency");
        project
            .apply(
                heads(7, 7),
                "accept-dependency-proof",
                MutationCommand::Decision(DecisionInput {
                    decision: ReviewDecision {
                        id: dependency_decision.clone(),
                        packet: dependency_packet.id,
                        judgments: BTreeMap::from([(
                            dependency_criterion.identity(),
                            CriterionJudgment::Satisfied,
                        )]),
                        findings: BTreeSet::new(),
                        action: ReviewAction::Accept,
                    },
                }),
            )
            .expect("accept snapshot-backed dependency proof");

        let carried_structural = structural_context(&first_map, &carried_stage.id)
            .expect("derive carried Stage structural context");
        let carried_route = Route {
            id: RouteId::new("route-carry-dependent"),
            stage: carried_stage.identity(),
            structural_context: carried_structural,
            hypothesis: "the dependent attempt freezes the accepted dependency proof".to_owned(),
            assumptions: BTreeSet::from(["the dependency proof remains current".to_owned()]),
            rationale: "exercise recursive carry material collection".to_owned(),
        };
        project
            .apply(
                heads(8, 8),
                "add-carried-route",
                MutationCommand::AddRoute(AddRouteInput {
                    route: carried_route.clone(),
                }),
            )
            .expect("add carried Stage Route");
        project
            .apply(
                heads(9, 9),
                "select-carried-route",
                MutationCommand::SelectRoute(SelectRouteInput {
                    route: carried_route.id.clone(),
                }),
            )
            .expect("select carried Stage Route");

        let carried_context = current_context(&project, &objective);
        assert_eq!(
            carried_context.context.dependency_proofs,
            BTreeMap::from([(dependency_stage.identity(), dependency_decision.clone())])
        );
        let carried_attempt = crate::domain::AttemptId::new("attempt-carry-dependent");
        project
            .apply(
                heads(10, 10),
                "start-carried-attempt",
                MutationCommand::StartAttempt(StartAttemptInput {
                    attempt: Attempt {
                        id: carried_attempt.clone(),
                        route: carried_route.id,
                        ordinal: 1,
                        bound: AttemptBound::TerminationCondition(
                            "the dependency-backed proof is accepted".to_owned(),
                        ),
                        context: carried_context.context.clone(),
                    },
                }),
            )
            .expect("start carried Stage Attempt");
        let carried_evidence = EvidenceId::new("evidence-carry-dependent-inline");
        project
            .apply(
                heads(11, 11),
                "record-carried-evidence",
                MutationCommand::RecordEvidence(RecordEvidenceInput {
                    evidence: Evidence {
                        id: carried_evidence.clone(),
                        subject: EvidenceSubject::Attempt(carried_attempt.clone()),
                        context: carried_context.context,
                        purpose: EvidencePurpose::StageReview,
                        claims: BTreeMap::from([(
                            carried_criterion.identity(),
                            EvidenceClaim::Supports,
                        )]),
                        observation: FrozenObservation::Inline(CanonicalValue::String(
                            "the carried Stage itself has no artifact material".to_owned(),
                        )),
                        provenance: CanonicalValue::String(
                            "dependent proof with a frozen dependency".to_owned(),
                        ),
                    },
                }),
            )
            .expect("record inline carried Stage Evidence");
        project
            .apply(
                heads(12, 12),
                "seal-carried-attempt",
                MutationCommand::SealAttempt(SealAttemptCommand {
                    attempt: carried_attempt,
                    seal_reason: SealReason::Submitted,
                }),
            )
            .expect("seal carried Stage Attempt");
        let carried_packet = review_packet(&project, &objective);
        assert_eq!(
            carried_packet.evidence_set,
            BTreeSet::from([carried_evidence])
        );
        assert_eq!(
            carried_packet.context.dependency_proofs,
            BTreeMap::from([(dependency_stage.identity(), dependency_decision.clone())])
        );
        let dependency_material =
            dependency_view(&configuration(&project, &objective), &carried_packet)
                .expect("derive carried dependency material");
        assert_eq!(
            dependency_material,
            BTreeSet::from([dependency_decision.clone()])
        );
        let carried_decision = ReviewDecisionId::new("decision-carry-dependent");
        project
            .apply(
                heads(13, 13),
                "accept-carried-proof",
                MutationCommand::Decision(DecisionInput {
                    decision: ReviewDecision {
                        id: carried_decision.clone(),
                        packet: carried_packet.id,
                        judgments: BTreeMap::from([(
                            carried_criterion.identity(),
                            CriterionJudgment::Satisfied,
                        )]),
                        findings: BTreeSet::new(),
                        action: ReviewAction::Accept,
                    },
                }),
            )
            .expect("accept carried proof with its frozen dependency proof");

        fs::remove_file(project.blob_path(&snapshot)).expect("remove carried proof artifact");
        project
            .apply(
                heads(14, 14),
                "request-carry-remap",
                MutationCommand::RequestRemap(RequestRemapInput {
                    reason: "replace the Map while carrying the dependency proof chain".to_owned(),
                }),
            )
            .expect("enter Mapping with both accepted proofs available for carry");

        let mut replacement = first_map.clone();
        replacement.revision = 2;
        replacement.stages.remove(&followup_stage.id);
        replacement.criteria.remove(&followup_criterion.id);
        replacement.priorities.remove(&followup_stage.id);
        replacement.owners.remove(&followup_criterion.id);
        replacement.contracts.remove(&followup_stage.id);
        let replacement_id = replacement.identity();
        let carry_install = MutationCommand::InstallMap(InstallMapInput {
            cover: crate::domain::CoverJudgment {
                map: replacement_id,
                objective_spec: replacement.objective_spec.clone(),
                verdict: crate::domain::CoverVerdict::Covered,
                rationale: "the carried Stage covers the confirmed Objective".to_owned(),
            },
            map: replacement,
            initial_routes: BTreeMap::new(),
            carry: BTreeMap::from([
                (dependency_stage.identity(), CarryVerdict::Valid),
                (carried_stage.identity(), CarryVerdict::Valid),
            ]),
        });
        assert_error_code(
            project.apply(heads(15, 15), "install-valid-carry", carry_install.clone()),
            "artifact_integrity_failed",
        );
        let assert_rejected_install_state = || {
            assert_eq!(
                project.event_count(),
                15,
                "failed carry must not append Trail"
            );
            assert_eq!(
                project.heads(&objective),
                heads(15, 15),
                "failed carry must preserve both heads"
            );
            assert!(matches!(
                configuration(&project, &objective).objective_state(),
                ObjectiveState::Mapping { .. }
            ));
        };
        assert_rejected_install_state();

        let restored = project
            .service
            .capture_artifact(CaptureArtifactRequest {
                binding: project.binding.clone(),
                bytes: artifact_bytes.clone(),
            })
            .expect("restore the missing dependency artifact");
        assert_eq!(restored, snapshot);
        fs::write(
            project.blob_path(&snapshot),
            b"corrupt bytes under the dependency snapshot digest",
        )
        .expect("corrupt the dependency artifact in place");
        assert_error_code(
            project.apply(heads(15, 15), "install-valid-carry", carry_install.clone()),
            "artifact_integrity_failed",
        );
        assert_rejected_install_state();

        fs::remove_file(project.blob_path(&snapshot)).expect("remove corrupt dependency artifact");
        let restored = project
            .service
            .capture_artifact(CaptureArtifactRequest {
                binding: project.binding.clone(),
                bytes: artifact_bytes,
            })
            .expect("restore the exact content-addressed dependency artifact");
        assert_eq!(restored, snapshot);
        project
            .apply(heads(15, 15), "install-valid-carry", carry_install)
            .expect("the same valid carry may commit after exact artifact repair");
        assert_eq!(
            project.event_count(),
            16,
            "repair permits exactly one install event"
        );
        assert!(matches!(
            configuration(&project, &objective).objective_state(),
            ObjectiveState::Achieved { manifest, .. } if manifest.len() == 2
        ));
    }

    #[test]
    fn report_claim_counts_fail_closed_on_a_missing_packet_evidence_reference() {
        let criterion = CriterionId::new("criterion-missing-evidence");
        let packet = ReviewPacket {
            id: ReviewPacketId::new("packet-missing-evidence"),
            attempt: crate::domain::AttemptId::new("attempt-missing-evidence"),
            stage: StageId::new("stage-missing-evidence"),
            context: AcceptanceContext {
                structural: StructuralContext {
                    contract: StageContract {
                        outcome: "test missing Evidence".to_owned(),
                        criteria: BTreeSet::from([criterion.clone()]),
                        objective_boundaries: BTreeSet::new(),
                        output: "no report row".to_owned(),
                    },
                    dependencies: BTreeMap::new(),
                },
                dependency_proofs: BTreeMap::new(),
            },
            termination: SealReason::Submitted,
            evidence_set: BTreeSet::from([EvidenceId::new("evidence-is-missing")]),
        };
        let error = packet_claim_counts(&initial_configuration(), &packet, &criterion)
            .expect_err("missing Packet Evidence must fail the report snapshot");
        assert_eq!(error.code, "invalid_projection");
        assert!(error.message.contains("evidence-is-missing"));
    }

    #[test]
    fn seal_materializes_exact_evidence_and_missing_blob_rolls_back_then_degrades_rebuild() {
        let project = TestProject::new();
        let objective = ObjectiveId::new("objective-seal-materialization");
        project
            .apply(
                heads(0, 0),
                "activate",
                activate_command(
                    &project.binding.project_id,
                    &objective,
                    heads(0, 0),
                    "complete the assembled flow",
                ),
            )
            .expect("activate Objective");
        project
            .apply(heads(1, 1), "install-map", install_map_command(&objective))
            .expect("install map");

        let map = map_revision(&objective);
        let stage = stage(&objective);
        let structural = structural_context(&map, &stage.id).expect("derive structural context");
        let (route, add_route) = route_command(&objective, &stage.id, structural);
        project
            .apply(heads(2, 2), "add-route", add_route)
            .expect("add route");
        project
            .apply(
                heads(3, 3),
                "select-route",
                MutationCommand::SelectRoute(SelectRouteInput {
                    route: route.clone(),
                }),
            )
            .expect("select route");

        let context = current_context(&project, &objective);
        let attempt = crate::domain::AttemptId::new("attempt-assembled");
        project
            .apply(
                heads(4, 4),
                "start-attempt",
                MutationCommand::StartAttempt(StartAttemptInput {
                    attempt: Attempt {
                        id: attempt.clone(),
                        route,
                        ordinal: 1,
                        bound: AttemptBound::TerminationCondition(
                            "the frozen service observation exists".to_owned(),
                        ),
                        context: context.context.clone(),
                    },
                }),
            )
            .expect("start attempt");

        let artifact_bytes = b"assembled CoreService evidence".to_vec();
        let snapshot = project
            .service
            .capture_artifact(CaptureArtifactRequest {
                binding: project.binding.clone(),
                bytes: artifact_bytes.clone(),
            })
            .expect("capture evidence artifact");
        let review_criterion = criterion(&objective).identity();
        let mut evidence_ids = BTreeSet::new();
        for (offset, (suffix, claim)) in [
            ("supports", Some(EvidenceClaim::Supports)),
            ("contradicts", Some(EvidenceClaim::Contradicts)),
            ("unknown", Some(EvidenceClaim::Unknown)),
            ("unassessed", None),
        ]
        .into_iter()
        .enumerate()
        {
            let evidence_id = EvidenceId::new(format!("evidence-{suffix}"));
            evidence_ids.insert(evidence_id.clone());
            let claims = claim.map_or_else(BTreeMap::new, |claim| {
                BTreeMap::from([(review_criterion.clone(), claim)])
            });
            let observation = if offset == 0 {
                FrozenObservation::CoreSnapshot(snapshot.clone())
            } else {
                FrozenObservation::Inline(CanonicalValue::String(format!("{suffix} observation")))
            };
            project
                .apply(
                    heads(5 + offset as u64, 5 + offset as u64),
                    &format!("record-{suffix}-evidence"),
                    MutationCommand::RecordEvidence(RecordEvidenceInput {
                        evidence: Evidence {
                            id: evidence_id,
                            subject: EvidenceSubject::Attempt(attempt.clone()),
                            context: context.context.clone(),
                            purpose: EvidencePurpose::StageReview,
                            claims,
                            observation,
                            provenance: CanonicalValue::String(
                                "assembled CoreService transaction test".to_owned(),
                            ),
                        },
                    }),
                )
                .expect("record review-count Evidence");
        }

        fs::remove_file(project.blob_path(&snapshot)).expect("remove referenced artifact");
        let seal = MutationCommand::SealAttempt(SealAttemptCommand {
            attempt: attempt.clone(),
            seal_reason: SealReason::Submitted,
        });
        assert_error_code(
            project.apply(heads(9, 9), "seal", seal.clone()),
            "artifact_integrity_failed",
        );
        assert_eq!(
            project.event_count(),
            9,
            "artifact failure must roll back the Seal event and heads"
        );

        let restored = project
            .service
            .capture_artifact(CaptureArtifactRequest {
                binding: project.binding.clone(),
                bytes: artifact_bytes,
            })
            .expect("recapture the same content-addressed artifact");
        assert_eq!(restored, snapshot);
        project
            .apply(heads(9, 9), "seal", seal)
            .expect("the rolled-back Seal request can succeed after repair");

        let trail = project.trail(&objective);
        assert_eq!(trail.len(), 10);
        let TransitionInput::SealAttempt(materialized) = &trail.last().unwrap().input else {
            panic!("last Trail fact must be the materialized SealAttempt");
        };
        assert_eq!(materialized.packet.attempt, attempt);
        assert_eq!(materialized.packet.stage, stage.id);
        assert_eq!(materialized.packet.evidence_set, evidence_ids);
        assert_eq!(materialized.packet.termination, SealReason::Submitted);
        assert_eq!(materialized.seal_reason, SealReason::Submitted);

        let report = project
            .service
            .report_snapshot(&project.binding, &objective)
            .expect("build Packet claim-count report");
        assert_eq!(
            report.reviews.columns,
            [
                "packet_id",
                "attempt_id",
                "stage_id",
                "criterion_id",
                "decision_id",
                "judgment",
                "action",
                "finding",
                "evidence_count",
                "supports_count",
                "contradicts_count",
                "unknown_count",
                "unassessed_count",
            ]
        );
        assert_eq!(report.reviews.rows.len(), 1);
        assert_eq!(
            &report.reviews.rows[0][8..13],
            &[
                ReportCell::Integer(4),
                ReportCell::Integer(1),
                ReportCell::Integer(1),
                ReportCell::Integer(1),
                ReportCell::Integer(1),
            ]
        );

        fs::remove_file(project.blob_path(&snapshot)).expect("remove reachable artifact again");
        let rebuilt = project
            .service
            .audit(AuditRequest {
                binding: project.binding.clone(),
                maintenance: Some(MaintenanceRequest {
                    action: MaintenanceAction::RebuildProjection,
                    expected_project_seq: 10,
                }),
                limit: None,
            })
            .expect("projection rebuild must remain independent of artifact availability");
        assert_eq!(rebuilt.status, AuditStatus::Degraded);
        assert_eq!(
            rebuilt.maintenance_applied,
            Some(MaintenanceAction::RebuildProjection)
        );
        assert!(
            rebuilt
                .issues
                .items
                .iter()
                .any(|issue| issue.code == "artifact_integrity_failed")
        );
        assert_eq!(project.event_count(), 10);
    }
}
