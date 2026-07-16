//! Strict, canonical persistence codec for immutable Trail facts.
//!
//! This is the only byte representation accepted for v1 events. Decoding is deliberately a
//! two-step operation: a bounded generic shape is inspected before typed decoding, then the typed
//! value is re-encoded and compared byte-for-byte with the source. No upcaster or permissive
//! compatibility path exists.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::*;

pub const TRAIL_EVENT_SCHEMA: &str = "mobius.trail-event.v1";
pub const MAX_EVENT_BYTES: usize = 1_048_576;
pub const MAX_JSON_DEPTH: usize = 64;
pub const MAX_JSON_NODES: usize = 65_536;
pub const MAX_STRING_BYTES: usize = 262_144;
pub const MAX_COLLECTION_ITEMS: usize = 16_384;

#[derive(Debug)]
pub enum EventCodecError {
    ByteLimit {
        actual: usize,
        maximum: usize,
    },
    ShapeLimit {
        limit: &'static str,
        actual: usize,
        maximum: usize,
    },
    Json(serde_json::Error),
    UnsupportedSchema(String),
    InvalidDigest(String),
    IdentityKeyMismatch {
        collection: &'static str,
        key: String,
        value_identity: String,
    },
    IdentityConflict(ObjectIdentity),
    NonCanonical,
}

impl Display for EventCodecError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ByteLimit { actual, maximum } => write!(
                formatter,
                "Trail event is {actual} bytes, exceeding the {maximum}-byte parser limit"
            ),
            Self::ShapeLimit {
                limit,
                actual,
                maximum,
            } => write!(
                formatter,
                "Trail event {limit} is {actual}, exceeding the parser limit of {maximum}"
            ),
            Self::Json(error) => write!(formatter, "invalid Trail event JSON: {error}"),
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "unsupported Trail event schema {schema:?}")
            }
            Self::InvalidDigest(digest) => write!(
                formatter,
                "CoreSnapshot digest {digest:?} is not canonical sha256:<lowercase-hex>"
            ),
            Self::IdentityKeyMismatch {
                collection,
                key,
                value_identity,
            } => write!(
                formatter,
                "{collection} key {key:?} does not match value identity {value_identity:?}"
            ),
            Self::IdentityConflict(identity) => write!(
                formatter,
                "Trail event binds object identity {identity:?} to different content"
            ),
            Self::NonCanonical => write!(
                formatter,
                "Trail event bytes are valid JSON but are not the canonical v1 encoding"
            ),
        }
    }
}

impl Error for EventCodecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for EventCodecError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct EventEnvelopeRef<'a> {
    schema: &'static str,
    objective: &'a ObjectiveId,
    input: &'a TransitionInput,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct EventEnvelope {
    schema: String,
    objective: ObjectiveId,
    input: TransitionInput,
}

/// Encode one immutable fact to the sole canonical v1 byte representation.
pub fn encode_trail_fact(fact: &TrailFact) -> Result<Vec<u8>, EventCodecError> {
    validate_event_identities(fact)?;
    let bytes = encode_canonical(&EventEnvelopeRef {
        schema: TRAIL_EVENT_SCHEMA,
        objective: &fact.objective,
        input: &fact.input,
    })?;
    admit_event_bytes(&bytes)?;
    Ok(bytes)
}

/// Encode an internal typed value for hashing or derived projection storage.
///
/// Callers must supply only closed serde types whose collections have deterministic order (the
/// domain and application DTOs use `BTreeMap`/`BTreeSet`). This helper adds no alternate event
/// schema and performs no permissive conversion.
pub fn encode_canonical<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, EventCodecError> {
    serde_json::to_vec(value).map_err(EventCodecError::from)
}

/// Decode one immutable fact, accepting only byte-for-byte canonical v1 JSON.
pub fn decode_trail_fact(bytes: &[u8]) -> Result<TrailFact, EventCodecError> {
    let shape = admit_event_bytes(bytes)?;

    let envelope: EventEnvelope = serde_json::from_value(shape)?;
    if envelope.schema != TRAIL_EVENT_SCHEMA {
        return Err(EventCodecError::UnsupportedSchema(envelope.schema));
    }

    let fact = TrailFact {
        objective: envelope.objective,
        input: envelope.input,
    };
    let canonical = encode_trail_fact(&fact)?;
    if canonical != bytes {
        return Err(EventCodecError::NonCanonical);
    }
    Ok(fact)
}

fn admit_event_bytes(bytes: &[u8]) -> Result<Value, EventCodecError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(EventCodecError::ByteLimit {
            actual: bytes.len(),
            maximum: MAX_EVENT_BYTES,
        });
    }

    let shape: Value = serde_json::from_slice(bytes)?;
    validate_shape(&shape)?;
    validate_schema(&shape)?;
    Ok(shape)
}

fn validate_schema(shape: &Value) -> Result<(), EventCodecError> {
    let Some(schema) = shape
        .as_object()
        .and_then(|object| object.get("schema"))
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    if schema == TRAIL_EVENT_SCHEMA {
        Ok(())
    } else {
        Err(EventCodecError::UnsupportedSchema(schema.to_owned()))
    }
}

#[derive(Default)]
struct ShapeCount {
    nodes: usize,
}

fn validate_shape(shape: &Value) -> Result<(), EventCodecError> {
    fn add_nodes(count: &mut ShapeCount, amount: usize) -> Result<(), EventCodecError> {
        count.nodes = count.nodes.saturating_add(amount);
        if count.nodes > MAX_JSON_NODES {
            return Err(EventCodecError::ShapeLimit {
                limit: "node count",
                actual: count.nodes,
                maximum: MAX_JSON_NODES,
            });
        }
        Ok(())
    }

    fn check_string(value: &str) -> Result<(), EventCodecError> {
        if value.len() > MAX_STRING_BYTES {
            return Err(EventCodecError::ShapeLimit {
                limit: "string byte length",
                actual: value.len(),
                maximum: MAX_STRING_BYTES,
            });
        }
        Ok(())
    }

    fn visit(value: &Value, depth: usize, count: &mut ShapeCount) -> Result<(), EventCodecError> {
        if depth > MAX_JSON_DEPTH {
            return Err(EventCodecError::ShapeLimit {
                limit: "nesting depth",
                actual: depth,
                maximum: MAX_JSON_DEPTH,
            });
        }
        add_nodes(count, 1)?;
        match value {
            Value::String(value) => check_string(value),
            Value::Array(values) => {
                if values.len() > MAX_COLLECTION_ITEMS {
                    return Err(EventCodecError::ShapeLimit {
                        limit: "array item count",
                        actual: values.len(),
                        maximum: MAX_COLLECTION_ITEMS,
                    });
                }
                for value in values {
                    visit(value, depth + 1, count)?;
                }
                Ok(())
            }
            Value::Object(values) => {
                if values.len() > MAX_COLLECTION_ITEMS {
                    return Err(EventCodecError::ShapeLimit {
                        limit: "object member count",
                        actual: values.len(),
                        maximum: MAX_COLLECTION_ITEMS,
                    });
                }
                add_nodes(count, values.len())?;
                for (key, value) in values {
                    check_string(key)?;
                    visit(value, depth + 1, count)?;
                }
                Ok(())
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
        }
    }

    visit(shape, 1, &mut ShapeCount::default())
}

struct IdentityValidator {
    objects: BTreeMap<ObjectIdentity, FirstClassObject>,
}

impl IdentityValidator {
    fn new() -> Self {
        Self {
            objects: BTreeMap::new(),
        }
    }

    fn register(&mut self, value: FirstClassObject) -> Result<(), EventCodecError> {
        let identity = value.identity();
        match self.objects.get(&identity) {
            Some(existing) if existing == &value => Ok(()),
            Some(_) => Err(EventCodecError::IdentityConflict(identity)),
            None => {
                self.objects.insert(identity, value);
                Ok(())
            }
        }
    }

    fn objective_spec(&mut self, value: &ObjectiveSpec) -> Result<(), EventCodecError> {
        for (key, criterion) in &value.criteria {
            ensure_key(
                "objective_spec.criteria",
                key.as_str(),
                criterion.id.as_str(),
            )?;
            self.register(FirstClassObject::Criterion(criterion.clone()))?;
        }
        self.register(FirstClassObject::ObjectiveSpec(value.clone()))
    }

    fn map_revision(&mut self, value: &MapRevision) -> Result<(), EventCodecError> {
        for (key, stage) in &value.stages {
            ensure_key("map_revision.stages", key.as_str(), stage.id.as_str())?;
            self.register(FirstClassObject::Stage(stage.clone()))?;
        }
        for (key, criterion) in &value.criteria {
            ensure_key("map_revision.criteria", key.as_str(), criterion.id.as_str())?;
            self.register(FirstClassObject::Criterion(criterion.clone()))?;
        }
        self.register(FirstClassObject::MapRevision(value.clone()))
    }

    fn route(&mut self, value: &Route) -> Result<(), EventCodecError> {
        self.register(FirstClassObject::Route(value.clone()))
    }

    fn evidence(&mut self, value: &Evidence) -> Result<(), EventCodecError> {
        validate_observation(&value.observation)?;
        self.register(FirstClassObject::Evidence(value.clone()))
    }

    fn decision(&mut self, value: &ReviewDecision) -> Result<(), EventCodecError> {
        if let ReviewAction::Wait(wait) = &value.action {
            self.register(FirstClassObject::WaitCondition((**wait).clone()))?;
        }
        self.register(FirstClassObject::ReviewDecision(value.clone()))
    }
}

fn ensure_key(
    collection: &'static str,
    key: &str,
    value_identity: &str,
) -> Result<(), EventCodecError> {
    if key == value_identity {
        Ok(())
    } else {
        Err(EventCodecError::IdentityKeyMismatch {
            collection,
            key: key.to_owned(),
            value_identity: value_identity.to_owned(),
        })
    }
}

fn validate_observation(observation: &FrozenObservation) -> Result<(), EventCodecError> {
    let FrozenObservation::CoreSnapshot(snapshot) = observation else {
        return Ok(());
    };
    if snapshot.digest.canonical_sha256_hex().is_some() {
        Ok(())
    } else {
        Err(EventCodecError::InvalidDigest(snapshot.digest.0.clone()))
    }
}

fn validate_event_identities(fact: &TrailFact) -> Result<(), EventCodecError> {
    let mut validator = IdentityValidator::new();
    validator.register(FirstClassObject::Objective(Objective {
        id: fact.objective.clone(),
    }))?;

    match &fact.input {
        TransitionInput::ActivateObjective(input) => {
            validator.objective_spec(&input.objective_spec)?;
            validator.objective_spec(&input.confirmation.confirmed_payload)?;
        }
        TransitionInput::InstallMap(input) => {
            validator.map_revision(&input.map)?;
            for (key, route) in &input.initial_routes {
                ensure_key(
                    "install_map.initial_routes",
                    key.as_str(),
                    route.id.as_str(),
                )?;
                validator.route(route)?;
            }
        }
        TransitionInput::AddRoute(input) => validator.route(&input.route)?,
        TransitionInput::SelectRoute(_) | TransitionInput::RequestRemap(_) => {}
        TransitionInput::StartAttempt(input) => {
            validator.register(FirstClassObject::Attempt(input.attempt.clone()))?;
        }
        TransitionInput::RecordEvidence(input) => validator.evidence(&input.evidence)?,
        TransitionInput::SealAttempt(input) => {
            validator.register(FirstClassObject::ReviewPacket(input.packet.clone()))?;
        }
        TransitionInput::Decision(input) => validator.decision(&input.decision)?,
        TransitionInput::CheckWait(input) => {
            for (key, evidence) in &input.evidence {
                ensure_key("check_wait.evidence", key.as_str(), evidence.id.as_str())?;
                validator.evidence(evidence)?;
            }
        }
        TransitionInput::ReviseObjective(input) => {
            validator.objective_spec(&input.objective_spec)?;
            validator.objective_spec(&input.confirmation.confirmed_payload)?;
        }
        TransitionInput::Abandon(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use sha2::{Digest, Sha256};

    use super::*;
    use crate::domain::{ReplayError, replay};

    fn objective() -> ObjectiveId {
        ObjectiveId::new("objective-1")
    }

    fn criterion() -> Criterion {
        Criterion {
            id: CriterionId::new("criterion-1"),
            statement: "criterion holds".into(),
            verification_rule: "inspect evidence".into(),
            scope: CriterionScope::Local,
        }
    }

    fn specification(revision: u64) -> ObjectiveSpec {
        let criterion = criterion();
        ObjectiveSpec {
            objective: objective(),
            revision,
            intended_outcome: "verified outcome".into(),
            criteria: BTreeMap::from([(criterion.id.clone(), criterion)]),
            boundaries: BTreeSet::from(["local".into()]),
            excluded_claims: BTreeSet::from(["unverified".into()]),
        }
    }

    fn confirmation(
        specification: &ObjectiveSpec,
        action: ObjectiveConfirmationAction,
    ) -> ObjectiveConfirmation {
        ObjectiveConfirmation {
            project: ProjectId::new("project-1"),
            action,
            objective_spec: specification.identity(),
            confirmed_payload: Box::new(specification.clone()),
            heads: HeadBinding {
                expected_project_seq: 0,
                expected_objective_seq: 0,
            },
            confirmed: true,
        }
    }

    fn contract() -> StageContract {
        StageContract {
            outcome: "stage outcome".into(),
            criteria: BTreeSet::from([CriterionId::new("criterion-1")]),
            objective_boundaries: BTreeSet::from(["local".into()]),
            output: "stage output".into(),
        }
    }

    fn context() -> StructuralContext {
        StructuralContext {
            contract: contract(),
            dependencies: BTreeMap::new(),
        }
    }

    fn acceptance_context() -> AcceptanceContext {
        AcceptanceContext {
            structural: context(),
            dependency_proofs: BTreeMap::new(),
        }
    }

    fn stage() -> Stage {
        Stage {
            id: StageId::new("stage-1"),
            name: "Stage one".into(),
            outcome: "stage outcome".into(),
            output: "stage output".into(),
            kind: StageKind::Ordinary,
        }
    }

    fn map_revision() -> MapRevision {
        let stage = stage();
        let criterion = criterion();
        MapRevision {
            objective_spec: specification(1).identity(),
            revision: 1,
            stages: BTreeMap::from([(stage.id.clone(), stage)]),
            criteria: BTreeMap::from([(criterion.id.clone(), criterion)]),
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
            structural_context: context(),
            hypothesis: "small hypothesis".into(),
            assumptions: BTreeSet::from(["tool works".into()]),
            rationale: "smallest route".into(),
        }
    }

    fn attempt() -> Attempt {
        Attempt {
            id: AttemptId::new("attempt-1"),
            route: RouteId::new("route-1"),
            ordinal: 1,
            bound: AttemptBound::TerminationCondition("done".into()),
            context: acceptance_context(),
        }
    }

    fn evidence_with(observation: FrozenObservation) -> Evidence {
        Evidence {
            id: EvidenceId::new("evidence-1"),
            subject: EvidenceSubject::Attempt(AttemptId::new("attempt-1")),
            context: acceptance_context(),
            purpose: EvidencePurpose::StageReview,
            claims: BTreeMap::from([(CriterionId::new("criterion-1"), EvidenceClaim::Supports)]),
            observation,
            provenance: CanonicalValue::Object(BTreeMap::from([(
                "source".into(),
                CanonicalValue::String("test".into()),
            )])),
        }
    }

    fn evidence() -> Evidence {
        evidence_with(FrozenObservation::Inline(CanonicalValue::Integer(7)))
    }

    fn packet() -> ReviewPacket {
        ReviewPacket {
            id: ReviewPacketId::new("packet-1"),
            attempt: AttemptId::new("attempt-1"),
            stage: StageId::new("stage-1"),
            context: acceptance_context(),
            termination: SealReason::Submitted,
            evidence_set: BTreeSet::from([EvidenceId::new("evidence-1")]),
        }
    }

    fn fact(input: TransitionInput) -> TrailFact {
        TrailFact {
            objective: objective(),
            input,
        }
    }

    fn all_transition_facts() -> Vec<TrailFact> {
        let spec = specification(1);
        let revised = specification(2);
        let map = map_revision();
        let route = route();
        let evidence = evidence();
        vec![
            fact(TransitionInput::ActivateObjective(ActivateObjectiveInput {
                objective_spec: spec.clone(),
                confirmation: confirmation(&spec, ObjectiveConfirmationAction::Activate),
            })),
            fact(TransitionInput::InstallMap(InstallMapInput {
                map: map.clone(),
                initial_routes: BTreeMap::from([(route.id.clone(), route.clone())]),
                cover: CoverJudgment {
                    map: map.identity(),
                    objective_spec: spec.identity(),
                    verdict: CoverVerdict::Covered,
                    rationale: "covered".into(),
                },
                carry: BTreeMap::new(),
            })),
            fact(TransitionInput::AddRoute(AddRouteInput {
                route: route.clone(),
            })),
            fact(TransitionInput::SelectRoute(SelectRouteInput {
                route: route.id,
            })),
            fact(TransitionInput::StartAttempt(StartAttemptInput {
                attempt: attempt(),
            })),
            fact(TransitionInput::RecordEvidence(RecordEvidenceInput {
                evidence: evidence.clone(),
            })),
            fact(TransitionInput::SealAttempt(SealAttemptInput {
                packet: packet(),
                seal_reason: SealReason::Submitted,
            })),
            fact(TransitionInput::Decision(DecisionInput {
                decision: ReviewDecision {
                    id: ReviewDecisionId::new("decision-1"),
                    packet: ReviewPacketId::new("packet-1"),
                    judgments: BTreeMap::from([(
                        CriterionId::new("criterion-1"),
                        CriterionJudgment::NotSatisfied,
                    )]),
                    findings: BTreeSet::from(["retry".into()]),
                    action: ReviewAction::Retry,
                },
            })),
            fact(TransitionInput::CheckWait(CheckWaitInput {
                wait_condition: WaitConditionId::new("wait-1"),
                evidence: BTreeMap::from([(evidence.id.clone(), evidence)]),
                judgment: WaitJudgment {
                    wait_condition: WaitConditionId::new("wait-1"),
                    evidence_set: BTreeSet::from([EvidenceId::new("evidence-1")]),
                    direction: WaitDirection::Stay,
                    rationale: "still waiting".into(),
                },
            })),
            fact(TransitionInput::RequestRemap(RequestRemapInput {
                reason: "re-map".into(),
            })),
            fact(TransitionInput::ReviseObjective(ReviseObjectiveInput {
                objective_spec: revised.clone(),
                confirmation: confirmation(&revised, ObjectiveConfirmationAction::Revise),
            })),
            fact(TransitionInput::Abandon(AbandonInput {
                reason: "stop".into(),
                confirmation: AbandonConfirmation {
                    project: ProjectId::new("project-1"),
                    objective: objective(),
                    reason: "stop".into(),
                    heads: HeadBinding {
                        expected_project_seq: 11,
                        expected_objective_seq: 11,
                    },
                    confirmed: true,
                },
            })),
        ]
    }

    fn sha256(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let mut encoded = String::with_capacity(digest.len() * 2);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in digest {
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        encoded
    }

    #[test]
    fn all_twelve_transition_encodings_match_golden_digests_and_round_trip() {
        let facts = all_transition_facts();
        let expected_kinds: Vec<_> = TransitionKind::ALL.into_iter().collect();
        let actual_kinds: Vec<_> = facts.iter().map(TrailFact::transition).collect();
        assert_eq!(actual_kinds, expected_kinds);

        let encoded: Vec<_> = facts
            .iter()
            .map(|fact| encode_trail_fact(fact).expect("golden event must encode"))
            .collect();
        for (source, bytes) in facts.iter().zip(&encoded) {
            assert_eq!(
                decode_trail_fact(bytes).expect("golden must decode"),
                *source
            );
            assert_eq!(
                encode_trail_fact(&decode_trail_fact(bytes).expect("golden must decode"))
                    .expect("round trip must encode"),
                *bytes
            );
        }

        let actual_digests: Vec<_> = encoded.iter().map(|bytes| sha256(bytes)).collect();
        let expected_digests = vec![
            "cb84dfdbe483092350b28bc74e4c0d4ee909ff81db083d3a9d1f862b021bcbdf",
            "63b7c32566eb97af4e354a3532f226f0b19fa15d0228f41ce8b7c563aee84af8",
            "8f3868bbd317b6eb9346223dcc095d9f53fe7a2b17a9dd418aaa73a02c6a5a2a",
            "7323608984b561fd4d781386b2d313fcaedd72b00f5157f6c63ea16c1d8bbcdb",
            "2509769b252e636ca333eabbb49b6bf489fb5e5c8ff71f77291485979344b77c",
            "07772b50dae2aa5f54e1981fa4a35631c4de02806264734d5355dd57f53c9917",
            "747313a773ab1f451d914a70b7e5655ac472c41075ece1c7e254e4752497058f",
            "e5be7fc4c9f28dba5aaa473635792719a5d8ef9b938cd6179d76d2481a280097",
            "39069f96ea844fa12b8cb3a1171be2477699c34464bf0592367c3fa0c6abd3b9",
            "b960afda3d957d29e9ba5732b826b6e627547a5262edd5594be74e59fa81f956",
            "c33e02254ab88038b83c32d1a5e9aa5ba0ec919b3559d8f933adf091c057fdf9",
            "9018447bc00a7f2afe5c3f06187bede32e23aae793631f78dbfa973af462d2ab",
        ];
        assert_eq!(actual_digests, expected_digests);
    }

    #[test]
    fn simple_event_has_an_explicit_stable_wire_shape() {
        let event = fact(TransitionInput::RequestRemap(RequestRemapInput {
            reason: "re-map".into(),
        }));
        assert_eq!(
            String::from_utf8(encode_trail_fact(&event).expect("encode")).expect("UTF-8"),
            r#"{"schema":"mobius.trail-event.v1","objective":"objective-1","input":{"request_remap":{"reason":"re-map"}}}"#
        );
    }

    #[test]
    fn schema_variant_fields_and_canonical_spelling_fail_closed() {
        let event = fact(TransitionInput::RequestRemap(RequestRemapInput {
            reason: "re-map".into(),
        }));
        let canonical = String::from_utf8(encode_trail_fact(&event).expect("encode")).unwrap();

        let unknown_schema = canonical.replace(TRAIL_EVENT_SCHEMA, "mobius.trail-event.v2");
        assert!(matches!(
            decode_trail_fact(unknown_schema.as_bytes()),
            Err(EventCodecError::UnsupportedSchema(_))
        ));

        let unknown_variant = canonical.replace("request_remap", "unknown_transition");
        assert!(matches!(
            decode_trail_fact(unknown_variant.as_bytes()),
            Err(EventCodecError::Json(_))
        ));

        let unknown_field = canonical.replace(
            r#"{"reason":"re-map"}"#,
            r#"{"reason":"re-map","packet":"forbidden"}"#,
        );
        assert!(matches!(
            decode_trail_fact(unknown_field.as_bytes()),
            Err(EventCodecError::Json(_))
        ));

        let reordered = r#"{"objective":"objective-1","schema":"mobius.trail-event.v1","input":{"request_remap":{"reason":"re-map"}}}"#;
        assert!(matches!(
            decode_trail_fact(reordered.as_bytes()),
            Err(EventCodecError::NonCanonical)
        ));

        let duplicated = canonical.replacen(
            r#"{"schema":"mobius.trail-event.v1"#,
            r#"{"schema":"mobius.trail-event.v1","schema":"mobius.trail-event.v1"#,
            1,
        );
        assert!(matches!(
            decode_trail_fact(duplicated.as_bytes()),
            Err(EventCodecError::NonCanonical)
        ));

        let whitespace = format!(" {canonical}");
        assert!(matches!(
            decode_trail_fact(whitespace.as_bytes()),
            Err(EventCodecError::NonCanonical)
        ));

        let trailing = format!("{canonical}null");
        assert!(matches!(
            decode_trail_fact(trailing.as_bytes()),
            Err(EventCodecError::Json(_))
        ));
    }

    #[test]
    fn sets_must_be_unique_and_canonically_ordered() {
        let mut event = all_transition_facts().remove(0);
        let TransitionInput::ActivateObjective(input) = &mut event.input else {
            panic!("first fixture is ActivateObjective")
        };
        input.objective_spec.boundaries = BTreeSet::from(["a".into(), "b".into()]);
        input.confirmation.confirmed_payload.boundaries = input.objective_spec.boundaries.clone();
        let canonical = String::from_utf8(encode_trail_fact(&event).expect("encode")).unwrap();
        assert!(canonical.contains(r#""boundaries":["a","b"]"#));

        let reversed = canonical.replace(r#""boundaries":["a","b"]"#, r#""boundaries":["b","a"]"#);
        assert!(matches!(
            decode_trail_fact(reversed.as_bytes()),
            Err(EventCodecError::NonCanonical)
        ));

        let duplicate =
            canonical.replace(r#""boundaries":["a","b"]"#, r#""boundaries":["a","a","b"]"#);
        assert!(matches!(
            decode_trail_fact(duplicate.as_bytes()),
            Err(EventCodecError::NonCanonical)
        ));
    }

    #[test]
    fn numeric_corpus_accepts_i128_bounds_and_rejects_other_spellings_or_ranges() {
        for integer in [i128::MIN, i128::MAX] {
            let event = fact(TransitionInput::RecordEvidence(RecordEvidenceInput {
                evidence: evidence_with(FrozenObservation::Inline(CanonicalValue::Integer(
                    integer,
                ))),
            }));
            let bytes = encode_trail_fact(&event).expect("i128 boundary must encode");
            assert_eq!(
                decode_trail_fact(&bytes).expect("i128 boundary must decode"),
                event
            );
        }

        let zero = fact(TransitionInput::RecordEvidence(RecordEvidenceInput {
            evidence: evidence_with(FrozenObservation::Inline(CanonicalValue::Integer(0))),
        }));
        let canonical = String::from_utf8(encode_trail_fact(&zero).expect("encode")).unwrap();
        assert!(canonical.contains(r#"{"integer":0}"#));

        let cases = [
            "170141183460469231731687303715884105728",
            "-170141183460469231731687303715884105729",
            "1e0",
            "-0",
            "01",
        ];
        for number in cases {
            let candidate =
                canonical.replace(r#"{"integer":0}"#, &format!(r#"{{"integer":{number}}}"#));
            assert!(
                decode_trail_fact(candidate.as_bytes()).is_err(),
                "numeric spelling/range {number} must fail closed"
            );
        }
    }

    #[test]
    fn identity_indexed_maps_and_in_event_rebindings_fail_closed() {
        let mut event = all_transition_facts().remove(0);
        let TransitionInput::ActivateObjective(input) = &mut event.input else {
            panic!("first fixture is ActivateObjective")
        };
        let criterion = input
            .objective_spec
            .criteria
            .remove(&CriterionId::new("criterion-1"))
            .unwrap();
        input
            .objective_spec
            .criteria
            .insert(CriterionId::new("wrong-key"), criterion);
        *input.confirmation.confirmed_payload = input.objective_spec.clone();
        assert!(matches!(
            encode_trail_fact(&event),
            Err(EventCodecError::IdentityKeyMismatch {
                collection: "objective_spec.criteria",
                ..
            })
        ));

        let raw = serde_json::to_vec(&EventEnvelopeRef {
            schema: TRAIL_EVENT_SCHEMA,
            objective: &event.objective,
            input: &event.input,
        })
        .unwrap();
        assert!(matches!(
            decode_trail_fact(&raw),
            Err(EventCodecError::IdentityKeyMismatch { .. })
        ));

        let mut conflict = all_transition_facts().remove(0);
        let TransitionInput::ActivateObjective(input) = &mut conflict.input else {
            panic!("first fixture is ActivateObjective")
        };
        input.confirmation.confirmed_payload.intended_outcome = "different content".into();
        assert!(matches!(
            encode_trail_fact(&conflict),
            Err(EventCodecError::IdentityConflict(_))
        ));
    }

    #[test]
    fn replay_rejects_identity_rebinding_across_individually_valid_events() {
        let spec = specification(1);
        let map = map_revision();
        let original_route = route();
        let mut rebound_route = original_route.clone();
        rebound_route.hypothesis = "different content under the same identity".into();
        let facts = [
            fact(TransitionInput::ActivateObjective(ActivateObjectiveInput {
                objective_spec: spec.clone(),
                confirmation: confirmation(&spec, ObjectiveConfirmationAction::Activate),
            })),
            fact(TransitionInput::InstallMap(InstallMapInput {
                map: map.clone(),
                initial_routes: BTreeMap::new(),
                cover: CoverJudgment {
                    map: map.identity(),
                    objective_spec: spec.identity(),
                    verdict: CoverVerdict::Covered,
                    rationale: "covered".into(),
                },
                carry: BTreeMap::new(),
            })),
            fact(TransitionInput::AddRoute(AddRouteInput {
                route: original_route,
            })),
            fact(TransitionInput::AddRoute(AddRouteInput {
                route: rebound_route,
            })),
        ];

        let decoded: Vec<_> = facts
            .iter()
            .map(|fact| {
                let bytes = encode_trail_fact(fact).expect("individual event is well formed");
                decode_trail_fact(&bytes).expect("individual event is canonical")
            })
            .collect();
        let error = replay(&decoded).expect_err("historical identity rebinding must fail");
        assert!(matches!(
            error,
            ReplayError::TransitionRejected { fact_index: 3, .. }
        ));
        assert!(error.to_string().contains("identity"));
    }

    #[test]
    fn parser_limits_are_applied_before_typed_decoding() {
        let oversized = vec![b' '; MAX_EVENT_BYTES + 1];
        assert!(matches!(
            decode_trail_fact(&oversized),
            Err(EventCodecError::ByteLimit { .. })
        ));

        let long_string = format!(r#""{}""#, "a".repeat(MAX_STRING_BYTES + 1));
        assert!(matches!(
            decode_trail_fact(long_string.as_bytes()),
            Err(EventCodecError::ShapeLimit {
                limit: "string byte length",
                ..
            })
        ));

        let too_many_items = format!(
            "[{}]",
            std::iter::repeat_n("0", MAX_COLLECTION_ITEMS + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(matches!(
            decode_trail_fact(too_many_items.as_bytes()),
            Err(EventCodecError::ShapeLimit {
                limit: "array item count",
                ..
            })
        ));

        let many_nodes = format!(
            "[{}]",
            std::iter::repeat_n("[0,0,0,0]", MAX_COLLECTION_ITEMS)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(matches!(
            decode_trail_fact(many_nodes.as_bytes()),
            Err(EventCodecError::ShapeLimit {
                limit: "node count",
                ..
            })
        ));

        let mut deep = "0".to_owned();
        for _ in 0..MAX_JSON_DEPTH {
            deep = format!("[{deep}]");
        }
        assert!(matches!(
            decode_trail_fact(deep.as_bytes()),
            Err(EventCodecError::ShapeLimit {
                limit: "nesting depth",
                ..
            })
        ));
    }

    #[test]
    fn event_encoder_enforces_the_same_byte_and_shape_admission_as_decoder() {
        let oversized_string = fact(TransitionInput::RequestRemap(RequestRemapInput {
            reason: "a".repeat(MAX_STRING_BYTES + 1),
        }));
        assert!(matches!(
            encode_trail_fact(&oversized_string),
            Err(EventCodecError::ShapeLimit {
                limit: "string byte length",
                ..
            })
        ));

        let oversized_collection = fact(TransitionInput::RecordEvidence(RecordEvidenceInput {
            evidence: evidence_with(FrozenObservation::Inline(CanonicalValue::List(vec![
                CanonicalValue::Null;
                MAX_COLLECTION_ITEMS + 1
            ]))),
        }));
        assert!(matches!(
            encode_trail_fact(&oversized_collection),
            Err(EventCodecError::ShapeLimit {
                limit: "array item count",
                ..
            })
        ));

        let mut nested = CanonicalValue::Null;
        for _ in 0..(MAX_JSON_DEPTH / 2) {
            nested = CanonicalValue::List(vec![nested]);
        }
        let oversized_depth = fact(TransitionInput::RecordEvidence(RecordEvidenceInput {
            evidence: evidence_with(FrozenObservation::Inline(nested)),
        }));
        assert!(matches!(
            encode_trail_fact(&oversized_depth),
            Err(EventCodecError::ShapeLimit {
                limit: "nesting depth",
                ..
            })
        ));

        let oversized_event = fact(TransitionInput::RecordEvidence(RecordEvidenceInput {
            evidence: evidence_with(FrozenObservation::Inline(CanonicalValue::List(vec![
                CanonicalValue::String("x".repeat(64));
                MAX_COLLECTION_ITEMS
            ]))),
        }));
        assert!(matches!(
            encode_trail_fact(&oversized_event),
            Err(EventCodecError::ByteLimit { .. })
        ));

        let internal_value = "a".repeat(MAX_STRING_BYTES + 1);
        assert_eq!(
            encode_canonical(&internal_value)
                .expect("non-event canonical encoding stays unbounded"),
            serde_json::to_vec(&internal_value).expect("reference JSON encoding")
        );
    }

    #[test]
    fn core_snapshot_digest_has_one_canonical_syntax() {
        let valid = fact(TransitionInput::RecordEvidence(RecordEvidenceInput {
            evidence: evidence_with(FrozenObservation::CoreSnapshot(CoreSnapshot {
                digest: ContentDigest(format!("sha256:{}", "a".repeat(64))),
                size_bytes: 9,
            })),
        }));
        let bytes = encode_trail_fact(&valid).expect("canonical digest must encode");
        assert_eq!(
            decode_trail_fact(&bytes).expect("canonical digest must decode"),
            valid
        );

        for digest in ["", "sha256:abc", &format!("sha256:{}", "A".repeat(64))] {
            let invalid = fact(TransitionInput::RecordEvidence(RecordEvidenceInput {
                evidence: evidence_with(FrozenObservation::CoreSnapshot(CoreSnapshot {
                    digest: ContentDigest(digest.to_owned()),
                    size_bytes: 9,
                })),
            }));
            assert!(matches!(
                encode_trail_fact(&invalid),
                Err(EventCodecError::InvalidDigest(_))
            ));
        }
    }
}
