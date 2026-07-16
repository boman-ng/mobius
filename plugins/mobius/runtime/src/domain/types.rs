//! Pure domain types for the Mobius model.
//!
//! It contains values that can be compared and ordered deterministically. Serde derives define
//! the strict persistence boundary owned by [`super::codec`]; clocks, paths, I/O, and runtime
//! identities remain outside the domain.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

macro_rules! semantic_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

// Implementation-boundary decision: the mathematical model does not prescribe byte-level
// identities for these objects. These are explicit, auditable semantic identifiers supplied by
// the main-agent/Core boundary. Admission must reject an empty identifier and any attempt to
// associate one identifier with two structurally different values. They must never be derived
// from a timestamp, filesystem path, database row id, or Runtime identity.
semantic_id!(ObjectiveId);
semantic_id!(ProjectId);
semantic_id!(StageId);
semantic_id!(CriterionId);
semantic_id!(RouteId);
semantic_id!(AttemptId);
semantic_id!(EvidenceId);
semantic_id!(ReviewPacketId);
semantic_id!(ReviewDecisionId);
semantic_id!(WaitConditionId);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ObjectiveSpecId {
    pub objective: ObjectiveId,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct MapRevisionId {
    pub objective: ObjectiveId,
    pub revision: u64,
}

pub trait HasIdentity {
    type Id: Clone + Debug + Eq + Ord;

    fn identity(&self) -> Self::Id;
}

/// Mechanical support for the model axiom `id(x) = id(y) iff x = y`.
///
/// `IdentityConflict` is never a valid knowledge-set state. A later admission guard must reject
/// it rather than overwrite either value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IdentityRelation {
    SameObject,
    DifferentIdentity,
    IdentityConflict,
}

pub fn compare_identity<T>(left: &T, right: &T) -> IdentityRelation
where
    T: HasIdentity + Eq,
{
    if left.identity() != right.identity() {
        IdentityRelation::DifferentIdentity
    } else if left == right {
        IdentityRelation::SameObject
    } else {
        IdentityRelation::IdentityConflict
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Objective {
    pub id: ObjectiveId,
}

impl HasIdentity for Objective {
    type Id = ObjectiveId;

    fn identity(&self) -> Self::Id {
        self.id.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionScope {
    Local,
    CrossStage,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Criterion {
    pub id: CriterionId,
    pub statement: String,
    pub verification_rule: String,
    pub scope: CriterionScope,
}

impl HasIdentity for Criterion {
    type Id = CriterionId;

    fn identity(&self) -> Self::Id {
        self.id.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ObjectiveSpec {
    pub objective: ObjectiveId,
    pub revision: u64,
    pub intended_outcome: String,
    /// A set-like collection keyed by theoretical Criterion identity.
    pub criteria: BTreeMap<CriterionId, Criterion>,
    pub boundaries: BTreeSet<String>,
    pub excluded_claims: BTreeSet<String>,
}

impl HasIdentity for ObjectiveSpec {
    type Id = ObjectiveSpecId;

    fn identity(&self) -> Self::Id {
        ObjectiveSpecId {
            objective: self.objective.clone(),
            revision: self.revision,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Ordinary,
    FinalIntegration,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Stage {
    pub id: StageId,
    pub name: String,
    pub outcome: String,
    pub output: String,
    pub kind: StageKind,
}

impl HasIdentity for Stage {
    type Id = StageId;

    fn identity(&self) -> Self::Id {
        self.id.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct StageContract {
    pub outcome: String,
    pub criteria: BTreeSet<CriterionId>,
    pub objective_boundaries: BTreeSet<String>,
    pub output: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct StageDependency {
    pub dependency: StageId,
    pub dependent: StageId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct MapRevision {
    pub objective_spec: ObjectiveSpecId,
    pub revision: u64,
    /// `S_mu`, represented canonically as identity-to-value entries.
    pub stages: BTreeMap<StageId, Stage>,
    /// `C-hat_mu`, represented canonically as identity-to-value entries.
    pub criteria: BTreeMap<CriterionId, Criterion>,
    /// Directed `(dependency, dependent)` edges in the Stage DAG.
    pub dependencies: BTreeSet<StageDependency>,
    /// The stable priority function `pi_mu`.
    pub priorities: BTreeMap<StageId, u64>,
    /// The total Criterion ownership function.
    pub owners: BTreeMap<CriterionId, StageId>,
    /// The total Stage contract function.
    pub contracts: BTreeMap<StageId, StageContract>,
}

impl HasIdentity for MapRevision {
    type Id = MapRevisionId;

    fn identity(&self) -> Self::Id {
        MapRevisionId {
            objective: self.objective_spec.objective.clone(),
            revision: self.revision,
        }
    }
}

/// Recursive structural context `kappa_mu(s)`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct StructuralContext {
    pub contract: StageContract,
    pub dependencies: BTreeMap<StageId, DependencyStructuralContext>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DependencyStructuralContext {
    pub output: String,
    pub context: Box<StructuralContext>,
}

/// Runtime acceptance context `chi_mu,q(s)`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AcceptanceContext {
    pub structural: StructuralContext,
    pub dependency_proofs: BTreeMap<StageId, ReviewDecisionId>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Route {
    pub id: RouteId,
    pub stage: StageId,
    pub structural_context: StructuralContext,
    pub hypothesis: String,
    pub assumptions: BTreeSet<String>,
    pub rationale: String,
}

impl HasIdentity for Route {
    type Id = RouteId;

    fn identity(&self) -> Self::Id {
        self.id.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteStatus {
    Available,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AttemptBound {
    ResourceBudget { measure: String, limit: u64 },
    VerificationScope(BTreeSet<String>),
    TerminationCondition(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Attempt {
    pub id: AttemptId,
    pub route: RouteId,
    pub ordinal: u64,
    pub bound: AttemptBound,
    pub context: AcceptanceContext,
}

impl HasIdentity for Attempt {
    type Id = AttemptId;

    fn identity(&self) -> Self::Id {
        self.id.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptState {
    Running,
    Sealed,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealReason {
    Submitted,
    BoundReached,
    Interrupted,
}

/// Deterministic, normalized Inline value. Floating point is intentionally absent so equality
/// and ordering are total without a hidden canonicalization rule.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonicalValue {
    Null,
    Bool(bool),
    Integer(i128),
    String(String),
    List(Vec<CanonicalValue>),
    Object(BTreeMap<String, CanonicalValue>),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentDigest(pub String);

impl ContentDigest {
    pub const SHA256_PREFIX: &'static str = "sha256:";
    pub const SHA256_HEX_LENGTH: usize = 64;

    /// Return the digest payload only when it has the sole canonical Mobius syntax.
    pub fn canonical_sha256_hex(&self) -> Option<&str> {
        let hex = self.0.strip_prefix(Self::SHA256_PREFIX)?;
        (hex.len() == Self::SHA256_HEX_LENGTH
            && hex
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
        .then_some(hex)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CoreSnapshot {
    pub digest: ContentDigest,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FrozenObservation {
    Inline(CanonicalValue),
    CoreSnapshot(CoreSnapshot),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceSubject {
    Attempt(AttemptId),
    WaitCondition(WaitConditionId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidencePurpose {
    StageReview,
    WaitResolution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClaim {
    Supports,
    Contradicts,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Evidence {
    pub id: EvidenceId,
    pub subject: EvidenceSubject,
    pub context: AcceptanceContext,
    pub purpose: EvidencePurpose,
    /// Partial Criterion function; omission means that this Evidence makes no claim.
    pub claims: BTreeMap<CriterionId, EvidenceClaim>,
    pub observation: FrozenObservation,
    /// Frozen provenance may include a locator, but the locator is never the observation or id.
    pub provenance: CanonicalValue,
}

impl HasIdentity for Evidence {
    type Id = EvidenceId;

    fn identity(&self) -> Self::Id {
        self.id.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ReviewPacket {
    pub id: ReviewPacketId,
    pub attempt: AttemptId,
    pub stage: StageId,
    pub context: AcceptanceContext,
    pub termination: SealReason,
    pub evidence_set: BTreeSet<EvidenceId>,
}

impl HasIdentity for ReviewPacket {
    type Id = ReviewPacketId;

    fn identity(&self) -> Self::Id {
        self.id.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionJudgment {
    Satisfied,
    NotSatisfied,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct WaitCondition {
    pub id: WaitConditionId,
    pub stage: StageId,
    pub context: AcceptanceContext,
    pub cause: String,
    pub responsible_party: String,
    pub resume_condition: String,
}

impl HasIdentity for WaitCondition {
    type Id = WaitConditionId;

    fn identity(&self) -> Self::Id {
        self.id.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewAction {
    Accept,
    Retry,
    Replace,
    /// The complete condition is action content, not a separately supplied state target.
    Wait(Box<WaitCondition>),
    Remap {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ReviewDecision {
    pub id: ReviewDecisionId,
    pub packet: ReviewPacketId,
    /// Total over the current Stage contract when admitted.
    pub judgments: BTreeMap<CriterionId, CriterionJudgment>,
    pub findings: BTreeSet<String>,
    pub action: ReviewAction,
}

impl HasIdentity for ReviewDecision {
    type Id = ReviewDecisionId;

    fn identity(&self) -> Self::Id {
        self.id.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitDirection {
    Stay,
    SameRoute,
    NewRoute,
    Remap,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct WaitJudgment {
    pub wait_condition: WaitConditionId,
    pub evidence_set: BTreeSet<EvidenceId>,
    pub direction: WaitDirection,
    pub rationale: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarryVerdict {
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverVerdict {
    Covered,
    NotCovered,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CoverJudgment {
    pub map: MapRevisionId,
    pub objective_spec: ObjectiveSpecId,
    pub verdict: CoverVerdict,
    pub rationale: String,
}

pub type Manifest = BTreeMap<StageId, ReviewDecisionId>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MappingReason {
    Initial,
    SpecRevised,
    WaitRevealedDrift,
    Remap(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectiveState {
    Idle,
    Mapping {
        objective: ObjectiveId,
        objective_spec: ObjectiveSpecId,
        previous_map: Option<MapRevisionId>,
        reason: Option<MappingReason>,
    },
    Navigating {
        objective: ObjectiveId,
        map: MapRevisionId,
        navigation: NavState,
    },
    Achieved {
        objective: ObjectiveId,
        map: MapRevisionId,
        manifest: Manifest,
    },
    Abandoned {
        objective: ObjectiveId,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum NavState {
    SeekingRoute {
        stage: StageId,
    },
    Ready {
        stage: StageId,
        route: RouteId,
    },
    Attempting {
        stage: StageId,
        route: RouteId,
        attempt: AttemptId,
    },
    Reviewing {
        stage: StageId,
        route: RouteId,
        attempt: AttemptId,
        packet: ReviewPacketId,
    },
    Waiting {
        stage: StageId,
        route: RouteId,
        wait_condition: WaitConditionId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StageState {
    Queued,
    Current,
    Achieved,
    Retired,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct LifecycleProjection {
    pub route_status: BTreeMap<RouteId, RouteStatus>,
    /// Reasons remain owned by immutable transition facts; the reducer projects only model state.
    pub attempt_state: BTreeMap<AttemptId, AttemptState>,
    pub invalidated_proofs: BTreeSet<ReviewDecisionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObjectKind {
    Objective,
    ObjectiveSpec,
    MapRevision,
    Stage,
    Criterion,
    Route,
    Attempt,
    Evidence,
    ReviewPacket,
    ReviewDecision,
    WaitCondition,
}

impl ObjectKind {
    pub const ALL: [Self; 11] = [
        Self::Objective,
        Self::ObjectiveSpec,
        Self::MapRevision,
        Self::Stage,
        Self::Criterion,
        Self::Route,
        Self::Attempt,
        Self::Evidence,
        Self::ReviewPacket,
        Self::ReviewDecision,
        Self::WaitCondition,
    ];

    pub const fn schema_name(self) -> &'static str {
        match self {
            Self::Objective => "objective",
            Self::ObjectiveSpec => "objective_spec",
            Self::MapRevision => "map_revision",
            Self::Stage => "stage",
            Self::Criterion => "criterion",
            Self::Route => "route",
            Self::Attempt => "attempt",
            Self::Evidence => "evidence",
            Self::ReviewPacket => "review_packet",
            Self::ReviewDecision => "review_decision",
            Self::WaitCondition => "wait_condition",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectIdentity {
    Objective(ObjectiveId),
    ObjectiveSpec(ObjectiveSpecId),
    MapRevision(MapRevisionId),
    Stage(StageId),
    Criterion(CriterionId),
    Route(RouteId),
    Attempt(AttemptId),
    Evidence(EvidenceId),
    ReviewPacket(ReviewPacketId),
    ReviewDecision(ReviewDecisionId),
    WaitCondition(WaitConditionId),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FirstClassObject {
    Objective(Objective),
    ObjectiveSpec(ObjectiveSpec),
    MapRevision(MapRevision),
    Stage(Stage),
    Criterion(Criterion),
    Route(Route),
    Attempt(Attempt),
    Evidence(Evidence),
    ReviewPacket(ReviewPacket),
    ReviewDecision(ReviewDecision),
    WaitCondition(WaitCondition),
}

impl FirstClassObject {
    pub const fn kind(&self) -> ObjectKind {
        match self {
            Self::Objective(_) => ObjectKind::Objective,
            Self::ObjectiveSpec(_) => ObjectKind::ObjectiveSpec,
            Self::MapRevision(_) => ObjectKind::MapRevision,
            Self::Stage(_) => ObjectKind::Stage,
            Self::Criterion(_) => ObjectKind::Criterion,
            Self::Route(_) => ObjectKind::Route,
            Self::Attempt(_) => ObjectKind::Attempt,
            Self::Evidence(_) => ObjectKind::Evidence,
            Self::ReviewPacket(_) => ObjectKind::ReviewPacket,
            Self::ReviewDecision(_) => ObjectKind::ReviewDecision,
            Self::WaitCondition(_) => ObjectKind::WaitCondition,
        }
    }

    pub fn identity(&self) -> ObjectIdentity {
        match self {
            Self::Objective(value) => ObjectIdentity::Objective(value.identity()),
            Self::ObjectiveSpec(value) => ObjectIdentity::ObjectiveSpec(value.identity()),
            Self::MapRevision(value) => ObjectIdentity::MapRevision(value.identity()),
            Self::Stage(value) => ObjectIdentity::Stage(value.identity()),
            Self::Criterion(value) => ObjectIdentity::Criterion(value.identity()),
            Self::Route(value) => ObjectIdentity::Route(value.identity()),
            Self::Attempt(value) => ObjectIdentity::Attempt(value.identity()),
            Self::Evidence(value) => ObjectIdentity::Evidence(value.identity()),
            Self::ReviewPacket(value) => ObjectIdentity::ReviewPacket(value.identity()),
            Self::ReviewDecision(value) => ObjectIdentity::ReviewDecision(value.identity()),
            Self::WaitCondition(value) => ObjectIdentity::WaitCondition(value.identity()),
        }
    }
}

/// Read-only first-class object knowledge. Only the domain reducer can add values, deriving each
/// map key from the value identity and rejecting attempts to rebind an accepted identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct ObjectKnowledge(BTreeMap<ObjectIdentity, FirstClassObject>);

impl ObjectKnowledge {
    pub const fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub(super) fn insert_checked(&mut self, value: FirstClassObject) -> Result<(), ObjectIdentity> {
        let identity = value.identity();
        match self.0.get(&identity) {
            Some(existing) if existing == &value => Ok(()),
            Some(_) => Err(identity),
            None => {
                self.0.insert(identity, value);
                Ok(())
            }
        }
    }
}

impl std::ops::Deref for ObjectKnowledge {
    type Target = BTreeMap<ObjectIdentity, FirstClassObject>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DomainConfiguration {
    pub(super) objective_state: ObjectiveState,
    pub(super) objects: ObjectKnowledge,
    pub(super) lifecycle: LifecycleProjection,
}

impl DomainConfiguration {
    pub const fn objective_state(&self) -> &ObjectiveState {
        &self.objective_state
    }

    pub const fn objects(&self) -> &ObjectKnowledge {
        &self.objects
    }

    pub const fn lifecycle(&self) -> &LifecycleProjection {
        &self.lifecycle
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct HeadBinding {
    pub expected_project_seq: u64,
    pub expected_objective_seq: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveConfirmationAction {
    Activate,
    Revise,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ObjectiveConfirmation {
    pub project: ProjectId,
    pub action: ObjectiveConfirmationAction,
    pub objective_spec: ObjectiveSpecId,
    /// Exact typed payload shown to and confirmed by the human. Admission compares this value
    /// with the command payload; an identity-only reference cannot prove full-payload binding.
    pub confirmed_payload: Box<ObjectiveSpec>,
    pub heads: HeadBinding,
    pub confirmed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AbandonConfirmation {
    pub project: ProjectId,
    pub objective: ObjectiveId,
    pub reason: String,
    pub heads: HeadBinding,
    pub confirmed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ActivateObjectiveInput {
    pub objective_spec: ObjectiveSpec,
    pub confirmation: ObjectiveConfirmation,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct InstallMapInput {
    pub map: MapRevision,
    pub initial_routes: BTreeMap<RouteId, Route>,
    pub cover: CoverJudgment,
    /// Must be total over exactly the structurally eligible Stages.
    pub carry: BTreeMap<StageId, CarryVerdict>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AddRouteInput {
    pub route: Route,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SelectRouteInput {
    pub route: RouteId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct StartAttemptInput {
    pub attempt: Attempt,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RecordEvidenceInput {
    pub evidence: Evidence,
}

/// The model-level input. The application service, not an external caller, materializes `packet`
/// from the current Attempt and complete Evidence universe.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SealAttemptInput {
    pub packet: ReviewPacket,
    pub seal_reason: SealReason,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DecisionInput {
    pub decision: ReviewDecision,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CheckWaitInput {
    pub wait_condition: WaitConditionId,
    pub evidence: BTreeMap<EvidenceId, Evidence>,
    pub judgment: WaitJudgment,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RequestRemapInput {
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ReviseObjectiveInput {
    pub objective_spec: ObjectiveSpec,
    pub confirmation: ObjectiveConfirmation,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AbandonInput {
    pub reason: String,
    pub confirmation: AbandonConfirmation,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitionInput {
    ActivateObjective(ActivateObjectiveInput),
    InstallMap(InstallMapInput),
    AddRoute(AddRouteInput),
    SelectRoute(SelectRouteInput),
    StartAttempt(StartAttemptInput),
    RecordEvidence(RecordEvidenceInput),
    SealAttempt(SealAttemptInput),
    Decision(DecisionInput),
    CheckWait(CheckWaitInput),
    RequestRemap(RequestRemapInput),
    ReviseObjective(ReviseObjectiveInput),
    Abandon(AbandonInput),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    ActivateObjective,
    InstallMap,
    AddRoute,
    SelectRoute,
    StartAttempt,
    RecordEvidence,
    SealAttempt,
    Decision,
    CheckWait,
    RequestRemap,
    ReviseObjective,
    Abandon,
}

impl TransitionKind {
    pub const ALL: [Self; 12] = [
        Self::ActivateObjective,
        Self::InstallMap,
        Self::AddRoute,
        Self::SelectRoute,
        Self::StartAttempt,
        Self::RecordEvidence,
        Self::SealAttempt,
        Self::Decision,
        Self::CheckWait,
        Self::RequestRemap,
        Self::ReviseObjective,
        Self::Abandon,
    ];

    /// Stable names for the future deterministic event parser. Unknown names must be rejected.
    pub const fn schema_name(self) -> &'static str {
        match self {
            Self::ActivateObjective => "activate_objective",
            Self::InstallMap => "install_map",
            Self::AddRoute => "add_route",
            Self::SelectRoute => "select_route",
            Self::StartAttempt => "start_attempt",
            Self::RecordEvidence => "record_evidence",
            Self::SealAttempt => "seal_attempt",
            Self::Decision => "decision",
            Self::CheckWait => "check_wait",
            Self::RequestRemap => "request_remap",
            Self::ReviseObjective => "revise_objective",
            Self::Abandon => "abandon",
        }
    }
}

impl TransitionInput {
    pub const fn kind(&self) -> TransitionKind {
        match self {
            Self::ActivateObjective(_) => TransitionKind::ActivateObjective,
            Self::InstallMap(_) => TransitionKind::InstallMap,
            Self::AddRoute(_) => TransitionKind::AddRoute,
            Self::SelectRoute(_) => TransitionKind::SelectRoute,
            Self::StartAttempt(_) => TransitionKind::StartAttempt,
            Self::RecordEvidence(_) => TransitionKind::RecordEvidence,
            Self::SealAttempt(_) => TransitionKind::SealAttempt,
            Self::Decision(_) => TransitionKind::Decision,
            Self::CheckWait(_) => TransitionKind::CheckWait,
            Self::RequestRemap(_) => TransitionKind::RequestRemap,
            Self::ReviseObjective(_) => TransitionKind::ReviseObjective,
            Self::Abandon(_) => TransitionKind::Abandon,
        }
    }
}

/// One immutable, Objective-scoped business fact in the Trail.
///
/// `transition` is intentionally not stored: the input variant is its only source of truth.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TrailFact {
    pub objective: ObjectiveId,
    pub input: TransitionInput,
}

impl TrailFact {
    pub const fn transition(&self) -> TransitionKind {
        self.input.kind()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn criterion() -> Criterion {
        Criterion {
            id: CriterionId::new("criterion-1"),
            statement: "observable outcome holds".into(),
            verification_rule: "inspect frozen evidence".into(),
            scope: CriterionScope::Local,
        }
    }

    fn stage() -> Stage {
        Stage {
            id: StageId::new("stage-1"),
            name: "Stage one".into(),
            outcome: "outcome".into(),
            output: "output contract".into(),
            kind: StageKind::Ordinary,
        }
    }

    fn contract() -> StageContract {
        StageContract {
            outcome: "outcome".into(),
            criteria: BTreeSet::from([CriterionId::new("criterion-1")]),
            objective_boundaries: BTreeSet::from(["remain local".into()]),
            output: "output contract".into(),
        }
    }

    fn structural_context() -> StructuralContext {
        StructuralContext {
            contract: contract(),
            dependencies: BTreeMap::new(),
        }
    }

    fn acceptance_context() -> AcceptanceContext {
        AcceptanceContext {
            structural: structural_context(),
            dependency_proofs: BTreeMap::new(),
        }
    }

    fn objective_spec() -> ObjectiveSpec {
        let criterion = criterion();
        ObjectiveSpec {
            objective: ObjectiveId::new("objective-1"),
            revision: 1,
            intended_outcome: "ship the verified result".into(),
            criteria: BTreeMap::from([(criterion.identity(), criterion)]),
            boundaries: BTreeSet::from(["local only".into()]),
            excluded_claims: BTreeSet::from(["unverified completion".into()]),
        }
    }

    fn map_revision() -> MapRevision {
        let stage = stage();
        let criterion = criterion();
        MapRevision {
            objective_spec: objective_spec().identity(),
            revision: 1,
            stages: BTreeMap::from([(stage.identity(), stage)]),
            criteria: BTreeMap::from([(criterion.identity(), criterion)]),
            dependencies: BTreeSet::new(),
            priorities: BTreeMap::from([(StageId::new("stage-1"), 1)]),
            owners: BTreeMap::from([(CriterionId::new("criterion-1"), StageId::new("stage-1"))]),
            contracts: BTreeMap::from([(StageId::new("stage-1"), contract())]),
        }
    }

    fn route() -> Route {
        Route {
            id: RouteId::new("route-1"),
            stage: StageId::new("stage-1"),
            structural_context: structural_context(),
            hypothesis: "this mechanism produces the output".into(),
            assumptions: BTreeSet::from(["tool available".into()]),
            rationale: "smallest falsifiable route".into(),
        }
    }

    fn attempt() -> Attempt {
        Attempt {
            id: AttemptId::new("attempt-1"),
            route: RouteId::new("route-1"),
            ordinal: 1,
            bound: AttemptBound::TerminationCondition("evidence captured".into()),
            context: acceptance_context(),
        }
    }

    fn evidence() -> Evidence {
        Evidence {
            id: EvidenceId::new("evidence-1"),
            subject: EvidenceSubject::Attempt(attempt().identity()),
            context: acceptance_context(),
            purpose: EvidencePurpose::StageReview,
            claims: BTreeMap::from([(CriterionId::new("criterion-1"), EvidenceClaim::Supports)]),
            observation: FrozenObservation::Inline(CanonicalValue::String("observed".into())),
            provenance: CanonicalValue::Object(BTreeMap::from([(
                "source".into(),
                CanonicalValue::String("verified command output".into()),
            )])),
        }
    }

    fn packet() -> ReviewPacket {
        ReviewPacket {
            id: ReviewPacketId::new("packet-1"),
            attempt: attempt().identity(),
            stage: StageId::new("stage-1"),
            context: acceptance_context(),
            termination: SealReason::Submitted,
            evidence_set: BTreeSet::from([EvidenceId::new("evidence-1")]),
        }
    }

    fn wait_condition() -> WaitCondition {
        WaitCondition {
            id: WaitConditionId::new("wait-1"),
            stage: StageId::new("stage-1"),
            context: acceptance_context(),
            cause: "external fact unavailable".into(),
            responsible_party: "environment".into(),
            resume_condition: "new observation exists".into(),
        }
    }

    fn decision() -> ReviewDecision {
        ReviewDecision {
            id: ReviewDecisionId::new("decision-1"),
            packet: packet().identity(),
            judgments: BTreeMap::from([(
                CriterionId::new("criterion-1"),
                CriterionJudgment::Satisfied,
            )]),
            findings: BTreeSet::new(),
            action: ReviewAction::Accept,
        }
    }

    #[test]
    fn identity_and_structural_equality_are_distinguished() {
        let original = criterion();
        assert_eq!(
            compare_identity(&original, &original.clone()),
            IdentityRelation::SameObject
        );

        let mut conflicting = original.clone();
        conflicting.statement = "different statement under the same id".into();
        assert_eq!(
            compare_identity(&original, &conflicting),
            IdentityRelation::IdentityConflict
        );

        let mut distinct = original.clone();
        distinct.id = CriterionId::new("criterion-2");
        assert_eq!(
            compare_identity(&original, &distinct),
            IdentityRelation::DifferentIdentity
        );

        let mut conflicting_revision = objective_spec();
        conflicting_revision.intended_outcome = "different revision content".into();
        assert_eq!(
            compare_identity(&objective_spec(), &conflicting_revision),
            IdentityRelation::IdentityConflict
        );
    }

    #[test]
    fn typed_identity_formulas_are_auditable() {
        assert_eq!(
            objective_spec().identity(),
            ObjectiveSpecId {
                objective: ObjectiveId::new("objective-1"),
                revision: 1,
            }
        );
        assert_eq!(
            map_revision().identity(),
            MapRevisionId {
                objective: ObjectiveId::new("objective-1"),
                revision: 1,
            }
        );
        assert_eq!(attempt().identity(), AttemptId::new("attempt-1"));
        assert_eq!(packet().identity(), ReviewPacketId::new("packet-1"));
        assert_eq!(decision().identity(), ReviewDecisionId::new("decision-1"));
    }

    #[test]
    fn first_class_object_union_covers_exactly_eleven_kinds() {
        let objects = vec![
            FirstClassObject::Objective(Objective {
                id: ObjectiveId::new("objective-1"),
            }),
            FirstClassObject::ObjectiveSpec(objective_spec()),
            FirstClassObject::MapRevision(map_revision()),
            FirstClassObject::Stage(stage()),
            FirstClassObject::Criterion(criterion()),
            FirstClassObject::Route(route()),
            FirstClassObject::Attempt(attempt()),
            FirstClassObject::Evidence(evidence()),
            FirstClassObject::ReviewPacket(packet()),
            FirstClassObject::ReviewDecision(decision()),
            FirstClassObject::WaitCondition(wait_condition()),
        ];

        let actual: BTreeSet<_> = objects.iter().map(FirstClassObject::kind).collect();
        let expected: BTreeSet<_> = ObjectKind::ALL.into_iter().collect();
        assert_eq!(objects.len(), 11);
        assert_eq!(actual, expected);

        let identities: BTreeSet<_> = objects.iter().map(FirstClassObject::identity).collect();
        assert_eq!(
            identities.len(),
            11,
            "object-kind tags keep the union disjoint"
        );
    }

    #[test]
    fn transition_union_covers_every_section_ten_relation() {
        let spec = objective_spec();
        let map = map_revision();
        let route = route();
        let attempt = attempt();
        let evidence = evidence();
        let packet = packet();
        let decision = decision();
        let heads = HeadBinding {
            expected_project_seq: 0,
            expected_objective_seq: 0,
        };
        let confirmation = ObjectiveConfirmation {
            project: ProjectId::new("project-1"),
            action: ObjectiveConfirmationAction::Activate,
            objective_spec: spec.identity(),
            confirmed_payload: Box::new(spec.clone()),
            heads: heads.clone(),
            confirmed: true,
        };

        let transitions = vec![
            TransitionInput::ActivateObjective(ActivateObjectiveInput {
                objective_spec: spec.clone(),
                confirmation: confirmation.clone(),
            }),
            TransitionInput::InstallMap(InstallMapInput {
                map: map.clone(),
                initial_routes: BTreeMap::from([(route.identity(), route.clone())]),
                cover: CoverJudgment {
                    map: map.identity(),
                    objective_spec: spec.identity(),
                    verdict: CoverVerdict::Covered,
                    rationale: "complete coverage".into(),
                },
                carry: BTreeMap::new(),
            }),
            TransitionInput::AddRoute(AddRouteInput {
                route: route.clone(),
            }),
            TransitionInput::SelectRoute(SelectRouteInput {
                route: route.identity(),
            }),
            TransitionInput::StartAttempt(StartAttemptInput {
                attempt: attempt.clone(),
            }),
            TransitionInput::RecordEvidence(RecordEvidenceInput {
                evidence: evidence.clone(),
            }),
            TransitionInput::SealAttempt(SealAttemptInput {
                packet: packet.clone(),
                seal_reason: SealReason::Submitted,
            }),
            TransitionInput::Decision(DecisionInput {
                decision: decision.clone(),
            }),
            TransitionInput::CheckWait(CheckWaitInput {
                wait_condition: wait_condition().identity(),
                evidence: BTreeMap::from([(evidence.identity(), evidence)]),
                judgment: WaitJudgment {
                    wait_condition: wait_condition().identity(),
                    evidence_set: BTreeSet::from([EvidenceId::new("evidence-1")]),
                    direction: WaitDirection::Stay,
                    rationale: "condition remains unresolved".into(),
                },
            }),
            TransitionInput::RequestRemap(RequestRemapInput {
                reason: "map no longer matches observed structure".into(),
            }),
            TransitionInput::ReviseObjective(ReviseObjectiveInput {
                objective_spec: spec.clone(),
                confirmation: ObjectiveConfirmation {
                    action: ObjectiveConfirmationAction::Revise,
                    ..confirmation
                },
            }),
            TransitionInput::Abandon(AbandonInput {
                reason: "user ended the objective".into(),
                confirmation: AbandonConfirmation {
                    project: ProjectId::new("project-1"),
                    objective: spec.objective,
                    reason: "user ended the objective".into(),
                    heads,
                    confirmed: true,
                },
            }),
        ];

        let actual: BTreeSet<_> = transitions.iter().map(TransitionInput::kind).collect();
        let expected: BTreeSet<_> = TransitionKind::ALL.into_iter().collect();
        assert_eq!(transitions.len(), 12);
        assert_eq!(actual, expected);

        let schema_names: BTreeSet<_> = TransitionKind::ALL
            .into_iter()
            .map(TransitionKind::schema_name)
            .collect();
        assert_eq!(schema_names.len(), TransitionKind::ALL.len());
    }

    #[test]
    fn lifecycle_values_expose_only_the_model_states() {
        let states = BTreeSet::from([
            AttemptState::Running,
            AttemptState::Sealed,
            AttemptState::Closed,
        ]);
        assert_eq!(states.len(), 3);

        let statuses = BTreeSet::from([RouteStatus::Available, RouteStatus::Rejected]);
        assert_eq!(statuses.len(), 2);
    }

    #[test]
    fn object_knowledge_derives_keys_and_accepts_only_identical_reinsertion() {
        let mut knowledge = ObjectKnowledge::new();
        let value = FirstClassObject::Stage(Stage {
            id: StageId::new("stage-1"),
            name: "Stage one".into(),
            outcome: "verified outcome".into(),
            output: "artifact".into(),
            kind: StageKind::Ordinary,
        });
        let identity = value.identity();

        assert_eq!(knowledge.insert_checked(value.clone()), Ok(()));
        assert_eq!(knowledge.insert_checked(value.clone()), Ok(()));
        assert_eq!(knowledge.get(&identity), Some(&value));
        assert_eq!(knowledge.len(), 1);

        let conflicting = FirstClassObject::Stage(Stage {
            id: StageId::new("stage-1"),
            name: "Different structure".into(),
            outcome: "verified outcome".into(),
            output: "artifact".into(),
            kind: StageKind::Ordinary,
        });
        assert_eq!(knowledge.insert_checked(conflicting), Err(identity.clone()));
        assert_eq!(knowledge.get(&identity), Some(&value));
    }
}
