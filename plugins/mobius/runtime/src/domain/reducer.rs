//! Deterministic state reduction and Trail replay for the Mobius domain model.
//!
//! The reducer has no authority to admit an input. Every transition is validated by the domain
//! guards before a cloned configuration is changed. It reads no clock, filesystem, environment,
//! transport, or runtime state, so an identical configuration and transition always produce an
//! identical result.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::guards::{
    GuardViolation, InvariantViolation, audit_invariants, complete, current_proofs, next_stage,
    validate_transition,
};
use super::types::*;

#[derive(Debug)]
pub enum ReduceError {
    Guard(GuardViolation),
    IdentityConflict(ObjectIdentity),
    MissingObject(ObjectIdentity),
    MissingAttemptState(AttemptId),
    InvalidPostGuardState {
        transition: TransitionKind,
        detail: &'static str,
    },
}

impl From<GuardViolation> for ReduceError {
    fn from(value: GuardViolation) -> Self {
        Self::Guard(value)
    }
}

impl Display for ReduceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Guard(violation) => {
                write!(formatter, "transition guard rejected input: {violation:?}")
            }
            Self::IdentityConflict(identity) => {
                write!(
                    formatter,
                    "object identity is bound to different content: {identity:?}"
                )
            }
            Self::MissingObject(identity) => {
                write!(
                    formatter,
                    "validated transition references a missing object: {identity:?}"
                )
            }
            Self::MissingAttemptState(attempt) => write!(
                formatter,
                "validated transition references missing Attempt state: {attempt:?}"
            ),
            Self::InvalidPostGuardState { transition, detail } => write!(
                formatter,
                "validated {transition:?} transition produced no legal deterministic successor: {detail}"
            ),
        }
    }
}

impl Error for ReduceError {}

/// A fail-closed Trail replay or projection-audit failure.
#[derive(Debug)]
pub enum ReplayError {
    MixedObjective {
        fact_index: usize,
        expected: ObjectiveId,
        found: ObjectiveId,
    },
    ObjectiveBindingMismatch {
        fact_index: usize,
        transition: TransitionKind,
        fact_objective: ObjectiveId,
        input_objective: Option<ObjectiveId>,
        current_objective: Option<ObjectiveId>,
    },
    TransitionRejected {
        fact_index: usize,
        transition: TransitionKind,
        source: ReduceError,
    },
    HistoricalInvariant {
        fact_index: usize,
        transition: TransitionKind,
        invariant: &'static str,
        detail: String,
    },
    ConfigurationInvariant {
        fact_index: usize,
        transition: TransitionKind,
        violations: Vec<InvariantViolation>,
    },
    ProjectionMismatch {
        objective: Option<ObjectiveId>,
    },
}

impl Display for ReplayError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MixedObjective {
                fact_index,
                expected,
                found,
            } => write!(
                formatter,
                "Trail fact {fact_index} belongs to {found:?}, but this Trail is scoped to {expected:?}"
            ),
            Self::ObjectiveBindingMismatch {
                fact_index,
                transition,
                fact_objective,
                input_objective,
                current_objective,
            } => write!(
                formatter,
                "Trail fact {fact_index} ({transition:?}) objective {fact_objective:?} is not bound to input objective {input_objective:?} and current objective {current_objective:?}"
            ),
            Self::TransitionRejected {
                fact_index,
                transition,
                source,
            } => write!(
                formatter,
                "Trail fact {fact_index} ({transition:?}) is not enabled in its replay pre-state: {source}"
            ),
            Self::HistoricalInvariant {
                fact_index,
                transition,
                invariant,
                detail,
            } => write!(
                formatter,
                "Trail fact {fact_index} ({transition:?}) violates historical {invariant}: {detail}"
            ),
            Self::ConfigurationInvariant {
                fact_index,
                transition,
                violations,
            } => write!(
                formatter,
                "Trail fact {fact_index} ({transition:?}) produced an invalid configuration: {violations:?}"
            ),
            Self::ProjectionMismatch { objective } => write!(
                formatter,
                "I15 replay result differs from the current projection for Objective {objective:?}"
            ),
        }
    }
}

impl Error for ReplayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TransitionRejected { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The unique initial configuration used by Trail replay.
pub fn initial_configuration() -> DomainConfiguration {
    DomainConfiguration {
        objective_state: ObjectiveState::Idle,
        objects: ObjectKnowledge::new(),
        lifecycle: LifecycleProjection::default(),
    }
}

/// Apply one already-typed transition after validating it against its exact pre-state.
pub fn reduce(
    configuration: &DomainConfiguration,
    input: &TransitionInput,
) -> Result<DomainConfiguration, ReduceError> {
    // Admission is always evaluated against the caller's immutable pre-state. No partial reducer
    // mutation is observable if either validation or a defensive post-guard check fails.
    validate_transition(configuration, input)?;

    let mut next = configuration.clone();
    match input {
        TransitionInput::ActivateObjective(input) => reduce_activate(&mut next, input)?,
        TransitionInput::InstallMap(input) => reduce_install_map(configuration, &mut next, input)?,
        TransitionInput::AddRoute(input) => reduce_add_route(&mut next, input)?,
        TransitionInput::SelectRoute(input) => reduce_select_route(&mut next, input)?,
        TransitionInput::StartAttempt(input) => reduce_start_attempt(&mut next, input)?,
        TransitionInput::RecordEvidence(input) => reduce_record_evidence(&mut next, input)?,
        TransitionInput::SealAttempt(input) => reduce_seal_attempt(&mut next, input)?,
        TransitionInput::Decision(input) => reduce_decision(&mut next, input)?,
        TransitionInput::CheckWait(input) => reduce_check_wait(&mut next, input)?,
        TransitionInput::RequestRemap(input) => reduce_request_remap(&mut next, input)?,
        TransitionInput::ReviseObjective(input) => reduce_revise_objective(&mut next, input)?,
        TransitionInput::Abandon(input) => reduce_abandon(&mut next, input)?,
    }
    Ok(next)
}

/// Rebuild one Objective configuration from its immutable Trail facts.
///
/// Replay validates each input against its exact historical pre-state. The additional step audit
/// mechanically covers the historical parts of I6, I17, and I19. Configuration-only invariant
/// checks remain scoped to what the projection can observe; they are not presented as a complete
/// `I1..I19` certificate.
pub fn replay(facts: &[TrailFact]) -> Result<DomainConfiguration, ReplayError> {
    let stream_objective = facts.first().map(|fact| fact.objective.clone());
    if let Some(expected) = &stream_objective {
        for (fact_index, fact) in facts.iter().enumerate().skip(1) {
            if &fact.objective != expected {
                return Err(ReplayError::MixedObjective {
                    fact_index,
                    expected: expected.clone(),
                    found: fact.objective.clone(),
                });
            }
        }
    }

    let mut configuration = initial_configuration();
    for (fact_index, fact) in facts.iter().enumerate() {
        validate_fact_objective(fact_index, &configuration, fact)?;
        let transition = fact.transition();
        let next = reduce(&configuration, &fact.input).map_err(|source| {
            ReplayError::TransitionRejected {
                fact_index,
                transition,
                source,
            }
        })?;
        audit_historical_step(fact_index, &configuration, fact, &next)?;
        audit_invariants(&next).map_err(|violations| ReplayError::ConfigurationInvariant {
            fact_index,
            transition,
            violations,
        })?;
        configuration = next;
    }
    Ok(configuration)
}

/// Compare the deterministic replay result with a stored current projection (I15).
///
/// The function deliberately does not claim that a configuration-only comparison proves every
/// model invariant. Historical I6, I17, and I19 checks are performed inside [`replay`].
pub fn audit_trail(
    facts: &[TrailFact],
    current_projection: &DomainConfiguration,
) -> Result<(), ReplayError> {
    let replayed = replay(facts)?;
    if &replayed == current_projection {
        Ok(())
    } else {
        Err(ReplayError::ProjectionMismatch {
            objective: facts.first().map(|fact| fact.objective.clone()),
        })
    }
}

fn validate_fact_objective(
    fact_index: usize,
    configuration: &DomainConfiguration,
    fact: &TrailFact,
) -> Result<(), ReplayError> {
    let input_objective = input_objective(&fact.input).cloned();
    let current_objective = current_objective(configuration).cloned();
    let input_matches = input_objective
        .as_ref()
        .is_none_or(|objective| objective == &fact.objective);
    let current_matches = if fact.transition() == TransitionKind::ActivateObjective {
        true
    } else {
        current_objective.as_ref() == Some(&fact.objective)
    };
    if input_matches && current_matches {
        Ok(())
    } else {
        Err(ReplayError::ObjectiveBindingMismatch {
            fact_index,
            transition: fact.transition(),
            fact_objective: fact.objective.clone(),
            input_objective,
            current_objective,
        })
    }
}

fn input_objective(input: &TransitionInput) -> Option<&ObjectiveId> {
    match input {
        TransitionInput::ActivateObjective(input) => Some(&input.objective_spec.objective),
        TransitionInput::InstallMap(input) => Some(&input.map.objective_spec.objective),
        TransitionInput::ReviseObjective(input) => Some(&input.objective_spec.objective),
        TransitionInput::Abandon(input) => Some(&input.confirmation.objective),
        TransitionInput::AddRoute(_)
        | TransitionInput::SelectRoute(_)
        | TransitionInput::StartAttempt(_)
        | TransitionInput::RecordEvidence(_)
        | TransitionInput::SealAttempt(_)
        | TransitionInput::Decision(_)
        | TransitionInput::CheckWait(_)
        | TransitionInput::RequestRemap(_) => None,
    }
}

fn current_objective(configuration: &DomainConfiguration) -> Option<&ObjectiveId> {
    match &configuration.objective_state {
        ObjectiveState::Idle => None,
        ObjectiveState::Mapping { objective, .. }
        | ObjectiveState::Navigating { objective, .. }
        | ObjectiveState::Achieved { objective, .. }
        | ObjectiveState::Abandoned { objective, .. } => Some(objective),
    }
}

fn evidence_ids(configuration: &DomainConfiguration) -> BTreeSet<EvidenceId> {
    configuration
        .objects
        .values()
        .filter_map(|object| match object {
            FirstClassObject::Evidence(evidence) => Some(evidence.identity()),
            _ => None,
        })
        .collect()
}

fn rejected_routes(configuration: &DomainConfiguration) -> BTreeSet<RouteId> {
    configuration
        .lifecycle
        .route_status
        .iter()
        .filter_map(|(route, status)| (*status == RouteStatus::Rejected).then_some(route.clone()))
        .collect()
}

fn historical_failure(
    fact_index: usize,
    fact: &TrailFact,
    invariant: &'static str,
    detail: impl Into<String>,
) -> ReplayError {
    ReplayError::HistoricalInvariant {
        fact_index,
        transition: fact.transition(),
        invariant,
        detail: detail.into(),
    }
}

fn audit_historical_step(
    fact_index: usize,
    before: &DomainConfiguration,
    fact: &TrailFact,
    after: &DomainConfiguration,
) -> Result<(), ReplayError> {
    // I6: `reduce` has already run EvidenceAdmission against `before`; the set delta proves that
    // exactly the Evidence named by this fact first entered Omega at this position.
    let before_evidence = evidence_ids(before);
    let after_evidence = evidence_ids(after);
    let expected_evidence: BTreeSet<_> = match &fact.input {
        TransitionInput::RecordEvidence(input) => BTreeSet::from([input.evidence.identity()]),
        TransitionInput::CheckWait(input) => input.evidence.keys().cloned().collect(),
        _ => BTreeSet::new(),
    };
    let added_evidence: BTreeSet<_> = after_evidence
        .difference(&before_evidence)
        .cloned()
        .collect();
    if !before_evidence.is_subset(&after_evidence) || added_evidence != expected_evidence {
        return Err(historical_failure(
            fact_index,
            fact,
            "I6",
            format!(
                "Evidence delta {added_evidence:?} differs from admitted input {expected_evidence:?}"
            ),
        ));
    }

    // I17: entering Abandoned is equivalent to applying the exact confirmed Abandon payload.
    let entered_abandoned = !matches!(before.objective_state, ObjectiveState::Abandoned { .. })
        && matches!(after.objective_state, ObjectiveState::Abandoned { .. });
    let confirmed_abandon = match &fact.input {
        TransitionInput::Abandon(input) => {
            input.confirmation.confirmed
                && input.confirmation.objective == fact.objective
                && input.confirmation.reason == input.reason
        }
        _ => false,
    };
    if entered_abandoned != confirmed_abandon {
        return Err(historical_failure(
            fact_index,
            fact,
            "I17",
            "Abandoned projection is not equivalent to an exact confirmed Abandon fact",
        ));
    }

    // I19: prove the biconditional incrementally. Rejected status is monotonic, and its exact
    // delta is the current Route only for Decision(replace) or CheckWait(new_route).
    let before_rejected = rejected_routes(before);
    let after_rejected = rejected_routes(after);
    let expected_rejection = match (&fact.input, &before.objective_state) {
        (
            TransitionInput::Decision(DecisionInput {
                decision:
                    ReviewDecision {
                        action: ReviewAction::Replace,
                        ..
                    },
            }),
            ObjectiveState::Navigating {
                navigation: NavState::Reviewing { route, .. },
                ..
            },
        )
        | (
            TransitionInput::CheckWait(CheckWaitInput {
                judgment:
                    WaitJudgment {
                        direction: WaitDirection::NewRoute,
                        ..
                    },
                ..
            }),
            ObjectiveState::Navigating {
                navigation: NavState::Waiting { route, .. },
                ..
            },
        ) => BTreeSet::from([route.clone()]),
        _ => BTreeSet::new(),
    };
    let added_rejections: BTreeSet<_> = after_rejected
        .difference(&before_rejected)
        .cloned()
        .collect();
    if !before_rejected.is_subset(&after_rejected) || added_rejections != expected_rejection {
        return Err(historical_failure(
            fact_index,
            fact,
            "I19",
            format!(
                "rejected Route delta {added_rejections:?} differs from fact-derived delta {expected_rejection:?}"
            ),
        ));
    }
    Ok(())
}

fn reduce_activate(
    next: &mut DomainConfiguration,
    input: &ActivateObjectiveInput,
) -> Result<(), ReduceError> {
    let specification = input.objective_spec.clone();
    let objective = Objective {
        id: specification.objective.clone(),
    };
    insert_object(next, FirstClassObject::Objective(objective))?;
    for criterion in specification.criteria.values().cloned() {
        insert_object(next, FirstClassObject::Criterion(criterion))?;
    }
    let specification_id = specification.identity();
    insert_object(next, FirstClassObject::ObjectiveSpec(specification.clone()))?;
    next.objective_state = ObjectiveState::Mapping {
        objective: specification.objective,
        objective_spec: specification_id,
        previous_map: None,
        reason: Some(MappingReason::Initial),
    };
    Ok(())
}

fn reduce_install_map(
    before: &DomainConfiguration,
    next: &mut DomainConfiguration,
    input: &InstallMapInput,
) -> Result<(), ReduceError> {
    let (objective, previous_map) = match &before.objective_state {
        ObjectiveState::Mapping {
            objective,
            previous_map,
            ..
        } => (objective.clone(), previous_map.clone()),
        _ => {
            return Err(invalid_state(
                TransitionKind::InstallMap,
                "guard admitted InstallMap outside Mapping",
            ));
        }
    };

    // Proof invalidation is monotonic. `carry` is guard-checked to be total over exactly the
    // structurally eligible stages, so any previous current proof not explicitly carried as Valid
    // loses proof status in the new Map.
    if previous_map.is_some() {
        for (stage, decision) in current_proofs(before)? {
            if input.carry.get(&stage) != Some(&CarryVerdict::Valid) {
                next.lifecycle.invalidated_proofs.insert(decision);
            }
        }
    }

    let map = input.map.clone();
    let map_id = map.identity();
    for stage in map.stages.values().cloned() {
        insert_object(next, FirstClassObject::Stage(stage))?;
    }
    for criterion in map.criteria.values().cloned() {
        insert_object(next, FirstClassObject::Criterion(criterion))?;
    }
    insert_object(next, FirstClassObject::MapRevision(map.clone()))?;
    for route in input.initial_routes.values().cloned() {
        let route_id = route.identity();
        insert_object(next, FirstClassObject::Route(route))?;
        next.lifecycle
            .route_status
            .entry(route_id)
            .or_insert(RouteStatus::Available);
    }

    // Derived scheduling queries need the newly installed Map to be current. This provisional
    // state exists only in this stack frame and is replaced before success is returned.
    let provisional_stage = deterministic_stage(&map).ok_or_else(|| {
        invalid_state(
            TransitionKind::InstallMap,
            "validated Map contains no Stage",
        )
    })?;
    next.objective_state = ObjectiveState::Navigating {
        objective: objective.clone(),
        map: map_id.clone(),
        navigation: NavState::SeekingRoute {
            stage: provisional_stage,
        },
    };
    settle_after_progress(next, objective, map_id, TransitionKind::InstallMap)
}

fn reduce_add_route(
    next: &mut DomainConfiguration,
    input: &AddRouteInput,
) -> Result<(), ReduceError> {
    let route = input.route.clone();
    let route_id = route.identity();
    insert_object(next, FirstClassObject::Route(route))?;
    next.lifecycle
        .route_status
        .entry(route_id)
        .or_insert(RouteStatus::Available);
    Ok(())
}

fn reduce_select_route(
    next: &mut DomainConfiguration,
    input: &SelectRouteInput,
) -> Result<(), ReduceError> {
    let (objective, map, stage) = match &next.objective_state {
        ObjectiveState::Navigating {
            objective,
            map,
            navigation: NavState::SeekingRoute { stage },
        } => (objective.clone(), map.clone(), stage.clone()),
        _ => {
            return Err(invalid_state(
                TransitionKind::SelectRoute,
                "guard admitted SelectRoute outside SeekingRoute",
            ));
        }
    };
    next.objective_state = ObjectiveState::Navigating {
        objective,
        map,
        navigation: NavState::Ready {
            stage,
            route: input.route.clone(),
        },
    };
    Ok(())
}

fn reduce_start_attempt(
    next: &mut DomainConfiguration,
    input: &StartAttemptInput,
) -> Result<(), ReduceError> {
    let (objective, map, stage, route) = match &next.objective_state {
        ObjectiveState::Navigating {
            objective,
            map,
            navigation: NavState::Ready { stage, route },
        } => (objective.clone(), map.clone(), stage.clone(), route.clone()),
        _ => {
            return Err(invalid_state(
                TransitionKind::StartAttempt,
                "guard admitted StartAttempt outside Ready",
            ));
        }
    };
    let attempt = input.attempt.clone();
    let attempt_id = attempt.identity();
    insert_object(next, FirstClassObject::Attempt(attempt))?;
    next.lifecycle
        .attempt_state
        .insert(attempt_id.clone(), AttemptState::Running);
    next.objective_state = ObjectiveState::Navigating {
        objective,
        map,
        navigation: NavState::Attempting {
            stage,
            route,
            attempt: attempt_id,
        },
    };
    Ok(())
}

fn reduce_record_evidence(
    next: &mut DomainConfiguration,
    input: &RecordEvidenceInput,
) -> Result<(), ReduceError> {
    insert_object(next, FirstClassObject::Evidence(input.evidence.clone()))
}

fn reduce_seal_attempt(
    next: &mut DomainConfiguration,
    input: &SealAttemptInput,
) -> Result<(), ReduceError> {
    let (objective, map, stage, route, attempt) = match &next.objective_state {
        ObjectiveState::Navigating {
            objective,
            map,
            navigation:
                NavState::Attempting {
                    stage,
                    route,
                    attempt,
                },
        } => (
            objective.clone(),
            map.clone(),
            stage.clone(),
            route.clone(),
            attempt.clone(),
        ),
        _ => {
            return Err(invalid_state(
                TransitionKind::SealAttempt,
                "guard admitted SealAttempt outside Attempting",
            ));
        }
    };
    let packet = input.packet.clone();
    let packet_id = packet.identity();
    insert_object(next, FirstClassObject::ReviewPacket(packet))?;
    next.lifecycle
        .attempt_state
        .insert(attempt.clone(), AttemptState::Sealed);
    next.objective_state = ObjectiveState::Navigating {
        objective,
        map,
        navigation: NavState::Reviewing {
            stage,
            route,
            attempt,
            packet: packet_id,
        },
    };
    Ok(())
}

fn reduce_decision(
    next: &mut DomainConfiguration,
    input: &DecisionInput,
) -> Result<(), ReduceError> {
    let (objective, map, stage, route, attempt) = match &next.objective_state {
        ObjectiveState::Navigating {
            objective,
            map,
            navigation:
                NavState::Reviewing {
                    stage,
                    route,
                    attempt,
                    ..
                },
        } => (
            objective.clone(),
            map.clone(),
            stage.clone(),
            route.clone(),
            attempt.clone(),
        ),
        _ => {
            return Err(invalid_state(
                TransitionKind::Decision,
                "guard admitted Decision outside Reviewing",
            ));
        }
    };

    let decision = input.decision.clone();
    insert_object(next, FirstClassObject::ReviewDecision(decision.clone()))?;
    close_attempt(next, &attempt, TransitionKind::Decision)?;

    match &decision.action {
        ReviewAction::Accept => {
            settle_after_progress(next, objective, map, TransitionKind::Decision)
        }
        ReviewAction::Retry => {
            next.objective_state = ObjectiveState::Navigating {
                objective,
                map,
                navigation: NavState::Ready { stage, route },
            };
            Ok(())
        }
        ReviewAction::Replace => {
            // I19: this is one of exactly two reducer branches that can reject a Route.
            next.lifecycle
                .route_status
                .insert(route.clone(), RouteStatus::Rejected);
            next.objective_state = ObjectiveState::Navigating {
                objective,
                map,
                navigation: NavState::SeekingRoute { stage },
            };
            Ok(())
        }
        ReviewAction::Wait(condition) => {
            let condition = condition.as_ref().clone();
            let condition_id = condition.identity();
            insert_object(next, FirstClassObject::WaitCondition(condition))?;
            next.objective_state = ObjectiveState::Navigating {
                objective,
                map,
                navigation: NavState::Waiting {
                    stage,
                    route,
                    wait_condition: condition_id,
                },
            };
            Ok(())
        }
        ReviewAction::Remap { reason } => {
            let objective_spec = objective_spec_for_map(next, &map)?;
            next.objective_state = ObjectiveState::Mapping {
                objective,
                objective_spec,
                previous_map: Some(map),
                reason: Some(MappingReason::Remap(reason.clone())),
            };
            Ok(())
        }
    }
}

fn reduce_check_wait(
    next: &mut DomainConfiguration,
    input: &CheckWaitInput,
) -> Result<(), ReduceError> {
    let (objective, map, stage, route, wait_condition) = match &next.objective_state {
        ObjectiveState::Navigating {
            objective,
            map,
            navigation:
                NavState::Waiting {
                    stage,
                    route,
                    wait_condition,
                },
        } => (
            objective.clone(),
            map.clone(),
            stage.clone(),
            route.clone(),
            wait_condition.clone(),
        ),
        _ => {
            return Err(invalid_state(
                TransitionKind::CheckWait,
                "guard admitted CheckWait outside Waiting",
            ));
        }
    };
    for evidence in input.evidence.values().cloned() {
        insert_object(next, FirstClassObject::Evidence(evidence))?;
    }

    match input.judgment.direction {
        WaitDirection::Stay => {
            next.objective_state = ObjectiveState::Navigating {
                objective,
                map,
                navigation: NavState::Waiting {
                    stage,
                    route,
                    wait_condition,
                },
            };
        }
        WaitDirection::SameRoute => {
            next.objective_state = ObjectiveState::Navigating {
                objective,
                map,
                navigation: NavState::Ready { stage, route },
            };
        }
        WaitDirection::NewRoute => {
            // I19: this is the only Route-rejecting branch other than Decision(replace).
            next.lifecycle
                .route_status
                .insert(route.clone(), RouteStatus::Rejected);
            next.objective_state = ObjectiveState::Navigating {
                objective,
                map,
                navigation: NavState::SeekingRoute { stage },
            };
        }
        WaitDirection::Remap => {
            let objective_spec = objective_spec_for_map(next, &map)?;
            next.objective_state = ObjectiveState::Mapping {
                objective,
                objective_spec,
                previous_map: Some(map),
                reason: Some(MappingReason::WaitRevealedDrift),
            };
        }
    }
    Ok(())
}

fn reduce_request_remap(
    next: &mut DomainConfiguration,
    input: &RequestRemapInput,
) -> Result<(), ReduceError> {
    let (objective, map, navigation) = match &next.objective_state {
        ObjectiveState::Navigating {
            objective,
            map,
            navigation,
        } => (objective.clone(), map.clone(), navigation.clone()),
        _ => {
            return Err(invalid_state(
                TransitionKind::RequestRemap,
                "guard admitted RequestRemap outside Navigating",
            ));
        }
    };
    close_navigation_attempt(next, &navigation, TransitionKind::RequestRemap)?;
    let objective_spec = objective_spec_for_map(next, &map)?;
    next.objective_state = ObjectiveState::Mapping {
        objective,
        objective_spec,
        previous_map: Some(map),
        reason: Some(MappingReason::Remap(input.reason.clone())),
    };
    Ok(())
}

fn reduce_revise_objective(
    next: &mut DomainConfiguration,
    input: &ReviseObjectiveInput,
) -> Result<(), ReduceError> {
    let (objective, previous_map, navigation) = match &next.objective_state {
        ObjectiveState::Mapping {
            objective,
            previous_map,
            ..
        } => (objective.clone(), previous_map.clone(), None),
        ObjectiveState::Navigating {
            objective,
            map,
            navigation,
        } => (
            objective.clone(),
            Some(map.clone()),
            Some(navigation.clone()),
        ),
        _ => {
            return Err(invalid_state(
                TransitionKind::ReviseObjective,
                "guard admitted ReviseObjective outside an active Objective",
            ));
        }
    };
    if let Some(navigation) = navigation {
        close_navigation_attempt(next, &navigation, TransitionKind::ReviseObjective)?;
    }

    let specification = input.objective_spec.clone();
    for criterion in specification.criteria.values().cloned() {
        insert_object(next, FirstClassObject::Criterion(criterion))?;
    }
    let specification_id = specification.identity();
    insert_object(next, FirstClassObject::ObjectiveSpec(specification))?;
    next.objective_state = ObjectiveState::Mapping {
        objective,
        objective_spec: specification_id,
        previous_map,
        reason: Some(MappingReason::SpecRevised),
    };
    Ok(())
}

fn reduce_abandon(next: &mut DomainConfiguration, input: &AbandonInput) -> Result<(), ReduceError> {
    let (objective, navigation) = match &next.objective_state {
        ObjectiveState::Mapping { objective, .. } => (objective.clone(), None),
        ObjectiveState::Navigating {
            objective,
            navigation,
            ..
        } => (objective.clone(), Some(navigation.clone())),
        _ => {
            return Err(invalid_state(
                TransitionKind::Abandon,
                "guard admitted Abandon outside an active Objective",
            ));
        }
    };
    if let Some(navigation) = navigation {
        close_navigation_attempt(next, &navigation, TransitionKind::Abandon)?;
    }
    next.objective_state = ObjectiveState::Abandoned {
        objective,
        reason: input.reason.clone(),
    };
    Ok(())
}

fn settle_after_progress(
    next: &mut DomainConfiguration,
    objective: ObjectiveId,
    map: MapRevisionId,
    transition: TransitionKind,
) -> Result<(), ReduceError> {
    if complete(next)? {
        let manifest = current_proofs(next)?;
        next.objective_state = ObjectiveState::Achieved {
            objective,
            map,
            manifest,
        };
        return Ok(());
    }

    let stage = next_stage(next)?.ok_or_else(|| {
        invalid_state(
            transition,
            "Objective is incomplete but no schedulable Stage exists",
        )
    })?;
    next.objective_state = ObjectiveState::Navigating {
        objective,
        map,
        navigation: NavState::SeekingRoute { stage },
    };
    Ok(())
}

fn close_navigation_attempt(
    next: &mut DomainConfiguration,
    navigation: &NavState,
    transition: TransitionKind,
) -> Result<(), ReduceError> {
    match navigation {
        NavState::Attempting { attempt, .. } | NavState::Reviewing { attempt, .. } => {
            close_attempt(next, attempt, transition)
        }
        NavState::SeekingRoute { .. } | NavState::Ready { .. } | NavState::Waiting { .. } => Ok(()),
    }
}

fn close_attempt(
    next: &mut DomainConfiguration,
    attempt: &AttemptId,
    transition: TransitionKind,
) -> Result<(), ReduceError> {
    let state = next
        .lifecycle
        .attempt_state
        .get(attempt)
        .copied()
        .ok_or_else(|| ReduceError::MissingAttemptState(attempt.clone()))?;
    match state {
        AttemptState::Running | AttemptState::Sealed => {}
        AttemptState::Closed => {
            return Err(ReduceError::InvalidPostGuardState {
                transition,
                detail: "guard admitted a second Attempt close",
            });
        }
    }
    next.lifecycle
        .attempt_state
        .insert(attempt.clone(), AttemptState::Closed);
    Ok(())
}

fn insert_object(
    next: &mut DomainConfiguration,
    object: FirstClassObject,
) -> Result<(), ReduceError> {
    let identity = object.identity();
    next.objects
        .insert_checked(object)
        .map_err(|_| ReduceError::IdentityConflict(identity))
}

fn objective_spec_for_map(
    configuration: &DomainConfiguration,
    map: &MapRevisionId,
) -> Result<ObjectiveSpecId, ReduceError> {
    let identity = ObjectIdentity::MapRevision(map.clone());
    match configuration.objects.get(&identity) {
        Some(FirstClassObject::MapRevision(map)) => Ok(map.objective_spec.clone()),
        _ => Err(ReduceError::MissingObject(identity)),
    }
}

fn deterministic_stage(map: &MapRevision) -> Option<StageId> {
    map.stages
        .keys()
        .min_by(|left, right| {
            map.priorities
                .get(*left)
                .cmp(&map.priorities.get(*right))
                .then_with(|| left.cmp(right))
        })
        .cloned()
}

fn invalid_state(transition: TransitionKind, detail: &'static str) -> ReduceError {
    ReduceError::InvalidPostGuardState { transition, detail }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::super::guards::audit_invariants;
    use super::*;

    fn assert_invariants(configuration: &DomainConfiguration) {
        if let Err(violations) = audit_invariants(configuration) {
            panic!("reduced configuration violates invariants: {violations:#?}");
        }
    }

    fn objective_id() -> ObjectiveId {
        ObjectiveId::new("objective-1")
    }

    fn stage_id(number: u8) -> StageId {
        StageId::new(format!("stage-{number}"))
    }

    fn criterion_id(number: u8) -> CriterionId {
        CriterionId::new(format!("criterion-{number}"))
    }

    fn route_id(number: u8) -> RouteId {
        RouteId::new(format!("route-{number}"))
    }

    fn criterion(number: u8) -> Criterion {
        Criterion {
            id: criterion_id(number),
            statement: format!("criterion {number} holds"),
            verification_rule: "inspect frozen evidence".into(),
            scope: CriterionScope::Local,
        }
    }

    fn stage(number: u8) -> Stage {
        Stage {
            id: stage_id(number),
            name: format!("Stage {number}"),
            outcome: format!("outcome {number}"),
            output: format!("output {number}"),
            kind: StageKind::Ordinary,
        }
    }

    fn contract(number: u8) -> StageContract {
        StageContract {
            outcome: format!("outcome {number}"),
            criteria: BTreeSet::from([criterion_id(number)]),
            objective_boundaries: BTreeSet::from(["remain inside the project".into()]),
            output: format!("output {number}"),
        }
    }

    fn structural_context(number: u8) -> StructuralContext {
        let dependencies = if number == 1 {
            BTreeMap::new()
        } else {
            BTreeMap::from([(
                stage_id(1),
                DependencyStructuralContext {
                    output: "output 1".into(),
                    context: Box::new(structural_context(1)),
                },
            )])
        };
        StructuralContext {
            contract: contract(number),
            dependencies,
        }
    }

    fn acceptance_context(
        number: u8,
        dependency_proof: Option<ReviewDecisionId>,
    ) -> AcceptanceContext {
        let dependency_proofs = dependency_proof
            .map(|proof| BTreeMap::from([(stage_id(1), proof)]))
            .unwrap_or_default();
        AcceptanceContext {
            structural: structural_context(number),
            dependency_proofs,
        }
    }

    fn objective_spec(revision: u64, criterion_count: u8) -> ObjectiveSpec {
        ObjectiveSpec {
            objective: objective_id(),
            revision,
            intended_outcome: "deliver the verified Objective".into(),
            criteria: (1..=criterion_count)
                .map(|number| (criterion_id(number), criterion(number)))
                .collect(),
            boundaries: BTreeSet::from(["remain inside the project".into()]),
            excluded_claims: BTreeSet::from(["unverified completion".into()]),
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

    fn map_revision(revision: u64, stage_count: u8) -> MapRevision {
        let stages: BTreeMap<_, _> = (1..=stage_count)
            .map(|number| (stage_id(number), stage(number)))
            .collect();
        let criteria: BTreeMap<_, _> = (1..=stage_count)
            .map(|number| (criterion_id(number), criterion(number)))
            .collect();
        let dependencies = if stage_count > 1 {
            BTreeSet::from([StageDependency {
                dependency: stage_id(1),
                dependent: stage_id(2),
            }])
        } else {
            BTreeSet::new()
        };
        MapRevision {
            objective_spec: ObjectiveSpecId {
                objective: objective_id(),
                revision: 1,
            },
            revision,
            stages,
            criteria,
            dependencies,
            priorities: (1..=stage_count)
                .map(|number| (stage_id(number), u64::from(number)))
                .collect(),
            owners: (1..=stage_count)
                .map(|number| (criterion_id(number), stage_id(number)))
                .collect(),
            contracts: (1..=stage_count)
                .map(|number| (stage_id(number), contract(number)))
                .collect(),
        }
    }

    fn cover(map: &MapRevision) -> CoverJudgment {
        CoverJudgment {
            map: map.identity(),
            objective_spec: map.objective_spec.clone(),
            verdict: CoverVerdict::Covered,
            rationale: "the Stage contracts cover the Objective".into(),
        }
    }

    fn route(number: u8, stage_number: u8) -> Route {
        Route {
            id: route_id(number),
            stage: stage_id(stage_number),
            structural_context: structural_context(stage_number),
            hypothesis: format!("route {number} reaches stage {stage_number}"),
            assumptions: BTreeSet::from(["required tool is available".into()]),
            rationale: "smallest falsifiable route".into(),
        }
    }

    fn attempt(route_number: u8, ordinal: u64, context: AcceptanceContext) -> Attempt {
        Attempt {
            id: AttemptId::new(format!("attempt-{route_number}-{ordinal}")),
            route: route_id(route_number),
            ordinal,
            bound: AttemptBound::TerminationCondition("frozen evidence exists".into()),
            context,
        }
    }

    fn evidence(
        number: u8,
        subject: AttemptId,
        stage_number: u8,
        context: AcceptanceContext,
    ) -> Evidence {
        Evidence {
            id: EvidenceId::new(format!("evidence-{number}")),
            subject: EvidenceSubject::Attempt(subject),
            context,
            purpose: EvidencePurpose::StageReview,
            claims: BTreeMap::from([(criterion_id(stage_number), EvidenceClaim::Supports)]),
            observation: FrozenObservation::Inline(CanonicalValue::String(format!(
                "observation {number}"
            ))),
            provenance: CanonicalValue::String("verified test fixture".into()),
        }
    }

    fn wait_evidence(number: u8, condition: &WaitCondition) -> Evidence {
        Evidence {
            id: EvidenceId::new(format!("wait-evidence-{number}")),
            subject: EvidenceSubject::WaitCondition(condition.identity()),
            context: condition.context.clone(),
            purpose: EvidencePurpose::WaitResolution,
            claims: BTreeMap::new(),
            observation: FrozenObservation::Inline(CanonicalValue::String(format!(
                "wait observation {number}"
            ))),
            provenance: CanonicalValue::String("verified wait fixture".into()),
        }
    }

    fn packet(
        attempt: &Attempt,
        stage_number: u8,
        context: AcceptanceContext,
        evidence_ids: BTreeSet<EvidenceId>,
    ) -> ReviewPacket {
        ReviewPacket {
            id: ReviewPacketId::new(format!("packet-{}", attempt.id.as_str())),
            attempt: attempt.identity(),
            stage: stage_id(stage_number),
            context,
            termination: SealReason::Submitted,
            evidence_set: evidence_ids,
        }
    }

    fn decision(
        number: u8,
        packet: &ReviewPacket,
        stage_number: u8,
        action: ReviewAction,
    ) -> ReviewDecision {
        ReviewDecision {
            id: ReviewDecisionId::new(format!("decision-{number}")),
            packet: packet.identity(),
            judgments: BTreeMap::from([(
                criterion_id(stage_number),
                if matches!(action, ReviewAction::Accept) {
                    CriterionJudgment::Satisfied
                } else {
                    CriterionJudgment::Unknown
                },
            )]),
            findings: BTreeSet::new(),
            action,
        }
    }

    fn activation(specification: &ObjectiveSpec) -> TransitionInput {
        TransitionInput::ActivateObjective(ActivateObjectiveInput {
            objective_spec: specification.clone(),
            confirmation: confirmation(specification, ObjectiveConfirmationAction::Activate),
        })
    }

    fn installation(map: &MapRevision, carry: BTreeMap<StageId, CarryVerdict>) -> TransitionInput {
        TransitionInput::InstallMap(InstallMapInput {
            map: map.clone(),
            initial_routes: BTreeMap::new(),
            cover: cover(map),
            carry,
        })
    }

    fn fact(input: TransitionInput) -> TrailFact {
        TrailFact {
            objective: objective_id(),
            input,
        }
    }

    fn facts(inputs: Vec<TransitionInput>) -> Vec<TrailFact> {
        inputs.into_iter().map(fact).collect()
    }

    fn reviewing_trail() -> (Vec<TrailFact>, Attempt, ReviewPacket) {
        let specification = objective_spec(1, 1);
        let map = map_revision(1, 1);
        let route = route(1, 1);
        let context = acceptance_context(1, None);
        let attempt = attempt(1, 1, context.clone());
        let evidence = evidence(1, attempt.identity(), 1, context.clone());
        let packet = packet(&attempt, 1, context, BTreeSet::from([evidence.identity()]));
        (
            facts(vec![
                activation(&specification),
                installation(&map, BTreeMap::new()),
                TransitionInput::AddRoute(AddRouteInput { route }),
                TransitionInput::SelectRoute(SelectRouteInput { route: route_id(1) }),
                TransitionInput::StartAttempt(StartAttemptInput {
                    attempt: attempt.clone(),
                }),
                TransitionInput::RecordEvidence(RecordEvidenceInput { evidence }),
                TransitionInput::SealAttempt(SealAttemptInput {
                    packet: packet.clone(),
                    seal_reason: SealReason::Submitted,
                }),
            ]),
            attempt,
            packet,
        )
    }

    fn attempting_trail() -> (Vec<TrailFact>, Attempt) {
        let specification = objective_spec(1, 1);
        let map = map_revision(1, 1);
        let route = route(1, 1);
        let attempt = attempt(1, 1, acceptance_context(1, None));
        (
            facts(vec![
                activation(&specification),
                installation(&map, BTreeMap::new()),
                TransitionInput::AddRoute(AddRouteInput { route }),
                TransitionInput::SelectRoute(SelectRouteInput { route: route_id(1) }),
                TransitionInput::StartAttempt(StartAttemptInput {
                    attempt: attempt.clone(),
                }),
            ]),
            attempt,
        )
    }

    #[test]
    fn complete_accept_path_reaches_achieved_with_manifest_and_closed_attempt() {
        let (mut trail, attempt, packet) = reviewing_trail();
        let accepted = decision(1, &packet, 1, ReviewAction::Accept);
        trail.push(fact(TransitionInput::Decision(DecisionInput {
            decision: accepted.clone(),
        })));

        let configuration = replay(&trail).expect("valid complete trail");
        assert_invariants(&configuration);
        assert_eq!(
            configuration.objective_state,
            ObjectiveState::Achieved {
                objective: objective_id(),
                map: MapRevisionId {
                    objective: objective_id(),
                    revision: 1,
                },
                manifest: BTreeMap::from([(stage_id(1), accepted.identity())]),
            }
        );
        assert_eq!(
            configuration
                .lifecycle
                .attempt_state
                .get(&attempt.identity()),
            Some(&AttemptState::Closed)
        );
        assert_eq!(
            configuration.lifecycle.route_status.get(&route_id(1)),
            Some(&RouteStatus::Available)
        );
    }

    #[test]
    fn review_retry_replace_and_remap_have_distinct_successors() {
        let (trail, attempt, packet) = reviewing_trail();
        let reviewing = replay(&trail).expect("reviewing pre-state");

        let retried = reduce(
            &reviewing,
            &TransitionInput::Decision(DecisionInput {
                decision: decision(1, &packet, 1, ReviewAction::Retry),
            }),
        )
        .expect("retry");
        assert_invariants(&retried);
        assert!(matches!(
            retried.objective_state,
            ObjectiveState::Navigating {
                navigation: NavState::Ready { .. },
                ..
            }
        ));
        assert_eq!(
            retried.lifecycle.route_status.get(&route_id(1)),
            Some(&RouteStatus::Available)
        );

        let replaced = reduce(
            &reviewing,
            &TransitionInput::Decision(DecisionInput {
                decision: decision(2, &packet, 1, ReviewAction::Replace),
            }),
        )
        .expect("replace");
        assert_invariants(&replaced);
        assert!(matches!(
            replaced.objective_state,
            ObjectiveState::Navigating {
                navigation: NavState::SeekingRoute { .. },
                ..
            }
        ));
        assert_eq!(
            replaced.lifecycle.route_status.get(&route_id(1)),
            Some(&RouteStatus::Rejected)
        );

        let remapped = reduce(
            &reviewing,
            &TransitionInput::Decision(DecisionInput {
                decision: decision(
                    3,
                    &packet,
                    1,
                    ReviewAction::Remap {
                        reason: "Stage contract is wrong".into(),
                    },
                ),
            }),
        )
        .expect("review remap");
        assert_invariants(&remapped);
        assert!(matches!(
            remapped.objective_state,
            ObjectiveState::Mapping {
                reason: Some(MappingReason::Remap(_)),
                ..
            }
        ));
        assert_eq!(
            remapped.lifecycle.attempt_state.get(&attempt.identity()),
            Some(&AttemptState::Closed)
        );
    }

    #[test]
    fn wait_check_directions_preserve_or_reject_route_exactly_as_modeled() {
        let (trail, _attempt, packet) = reviewing_trail();
        let reviewing = replay(&trail).expect("reviewing pre-state");
        let condition = WaitCondition {
            id: WaitConditionId::new("wait-1"),
            stage: stage_id(1),
            context: acceptance_context(1, None),
            cause: "external fact is unavailable".into(),
            responsible_party: "environment".into(),
            resume_condition: "a new observation exists".into(),
        };
        let waiting = reduce(
            &reviewing,
            &TransitionInput::Decision(DecisionInput {
                decision: decision(
                    1,
                    &packet,
                    1,
                    ReviewAction::Wait(Box::new(condition.clone())),
                ),
            }),
        )
        .expect("enter wait");

        for (number, direction) in [
            (1, WaitDirection::Stay),
            (2, WaitDirection::SameRoute),
            (3, WaitDirection::NewRoute),
            (4, WaitDirection::Remap),
        ] {
            let evidence = wait_evidence(number, &condition);
            let evidence_id = evidence.identity();
            let checked = reduce(
                &waiting,
                &TransitionInput::CheckWait(CheckWaitInput {
                    wait_condition: condition.identity(),
                    evidence: BTreeMap::from([(evidence_id.clone(), evidence)]),
                    judgment: WaitJudgment {
                        wait_condition: condition.identity(),
                        evidence_set: BTreeSet::from([evidence_id]),
                        direction,
                        rationale: "checked the complete wait evidence".into(),
                    },
                }),
            )
            .expect("check wait direction");
            assert_invariants(&checked);

            match direction {
                WaitDirection::Stay => assert!(matches!(
                    checked.objective_state,
                    ObjectiveState::Navigating {
                        navigation: NavState::Waiting { .. },
                        ..
                    }
                )),
                WaitDirection::SameRoute => assert!(matches!(
                    checked.objective_state,
                    ObjectiveState::Navigating {
                        navigation: NavState::Ready { .. },
                        ..
                    }
                )),
                WaitDirection::NewRoute => assert!(matches!(
                    checked.objective_state,
                    ObjectiveState::Navigating {
                        navigation: NavState::SeekingRoute { .. },
                        ..
                    }
                )),
                WaitDirection::Remap => assert!(matches!(
                    checked.objective_state,
                    ObjectiveState::Mapping {
                        reason: Some(MappingReason::WaitRevealedDrift),
                        ..
                    }
                )),
            }
            assert_eq!(
                checked.lifecycle.route_status.get(&route_id(1)),
                Some(if direction == WaitDirection::NewRoute {
                    &RouteStatus::Rejected
                } else {
                    &RouteStatus::Available
                })
            );
        }
    }

    #[test]
    fn explicit_remap_and_revision_close_running_attempt_without_seal() {
        let (trail, attempt) = attempting_trail();
        let attempting = replay(&trail).expect("attempting pre-state");

        let remapped = reduce(
            &attempting,
            &TransitionInput::RequestRemap(RequestRemapInput {
                reason: "dependencies changed".into(),
            }),
        )
        .expect("explicit remap");
        assert_invariants(&remapped);
        assert!(matches!(
            remapped.objective_state,
            ObjectiveState::Mapping {
                reason: Some(MappingReason::Remap(_)),
                ..
            }
        ));
        assert_eq!(
            remapped.lifecycle.attempt_state.get(&attempt.identity()),
            Some(&AttemptState::Closed)
        );

        let revised_specification = objective_spec(2, 1);
        let revised = reduce(
            &attempting,
            &TransitionInput::ReviseObjective(ReviseObjectiveInput {
                objective_spec: revised_specification.clone(),
                confirmation: confirmation(
                    &revised_specification,
                    ObjectiveConfirmationAction::Revise,
                ),
            }),
        )
        .expect("revise Objective");
        assert_invariants(&revised);
        assert!(matches!(
            revised.objective_state,
            ObjectiveState::Mapping {
                objective_spec: ObjectiveSpecId { revision: 2, .. },
                previous_map: Some(_),
                reason: Some(MappingReason::SpecRevised),
                ..
            }
        ));
        assert_eq!(
            revised.lifecycle.attempt_state.get(&attempt.identity()),
            Some(&AttemptState::Closed)
        );
    }

    #[test]
    fn remap_install_invalidates_uncarried_proof_preserves_carry_and_can_complete() {
        // Criterion 2 is Map-local, so a later Map may remove Stage 2 while preserving the
        // Objective's sole Criterion and the unchanged Stage 1 proof.
        let specification = objective_spec(1, 1);
        let first_map = map_revision(1, 2);
        let route = route(1, 1);
        let context = acceptance_context(1, None);
        let attempt = attempt(1, 1, context.clone());
        let evidence = evidence(1, attempt.identity(), 1, context.clone());
        let packet = packet(&attempt, 1, context, BTreeSet::from([evidence.identity()]));
        let accepted = decision(1, &packet, 1, ReviewAction::Accept);
        let trail = facts(vec![
            activation(&specification),
            installation(&first_map, BTreeMap::new()),
            TransitionInput::AddRoute(AddRouteInput { route }),
            TransitionInput::SelectRoute(SelectRouteInput { route: route_id(1) }),
            TransitionInput::StartAttempt(StartAttemptInput { attempt }),
            TransitionInput::RecordEvidence(RecordEvidenceInput { evidence }),
            TransitionInput::SealAttempt(SealAttemptInput {
                packet: packet.clone(),
                seal_reason: SealReason::Submitted,
            }),
            TransitionInput::Decision(DecisionInput {
                decision: accepted.clone(),
            }),
            TransitionInput::RequestRemap(RequestRemapInput {
                reason: "recheck the map".into(),
            }),
        ]);
        let mapping = replay(&trail).expect("mapping with one current proof");
        let replacement_map = map_revision(2, 2);

        let invalidated = reduce(
            &mapping,
            &installation(
                &replacement_map,
                BTreeMap::from([(stage_id(1), CarryVerdict::Invalid)]),
            ),
        )
        .expect("install with explicit invalid carry");
        assert_invariants(&invalidated);
        assert!(
            invalidated
                .lifecycle
                .invalidated_proofs
                .contains(&accepted.identity())
        );
        assert!(matches!(
            invalidated.objective_state,
            ObjectiveState::Navigating {
                navigation: NavState::SeekingRoute { ref stage },
                ..
            } if stage == &stage_id(1)
        ));

        let carried = reduce(
            &mapping,
            &installation(
                &replacement_map,
                BTreeMap::from([(stage_id(1), CarryVerdict::Valid)]),
            ),
        )
        .expect("install with valid carry");
        assert_invariants(&carried);
        assert!(
            !carried
                .lifecycle
                .invalidated_proofs
                .contains(&accepted.identity())
        );
        assert!(matches!(
            carried.objective_state,
            ObjectiveState::Navigating {
                navigation: NavState::SeekingRoute { ref stage },
                ..
            } if stage == &stage_id(2)
        ));

        let completion_map = map_revision(3, 1);
        let completed = reduce(
            &mapping,
            &installation(
                &completion_map,
                BTreeMap::from([(stage_id(1), CarryVerdict::Valid)]),
            ),
        )
        .expect("install a complete carried Map");
        assert_invariants(&completed);
        assert_eq!(
            completed.objective_state,
            ObjectiveState::Achieved {
                objective: objective_id(),
                map: completion_map.identity(),
                manifest: BTreeMap::from([(stage_id(1), accepted.identity())]),
            }
        );
    }

    #[test]
    fn abandon_closes_current_attempt_and_terminal_rejects_later_business_input() {
        let (mut trail, attempt) = attempting_trail();
        let reason = "human stopped the Objective".to_string();
        trail.push(fact(TransitionInput::Abandon(AbandonInput {
            reason: reason.clone(),
            confirmation: AbandonConfirmation {
                project: ProjectId::new("project-1"),
                objective: objective_id(),
                reason: reason.clone(),
                heads: HeadBinding {
                    expected_project_seq: 0,
                    expected_objective_seq: 0,
                },
                confirmed: true,
            },
        })));
        let abandoned = replay(&trail).expect("confirmed Abandon fact");
        audit_trail(&trail, &abandoned).expect("I15 projection equality");
        assert_invariants(&abandoned);
        assert_eq!(
            abandoned.objective_state,
            ObjectiveState::Abandoned {
                objective: objective_id(),
                reason,
            }
        );
        assert_eq!(
            abandoned.lifecycle.attempt_state.get(&attempt.identity()),
            Some(&AttemptState::Closed)
        );

        let after_terminal = reduce(
            &abandoned,
            &TransitionInput::AddRoute(AddRouteInput { route: route(2, 1) }),
        );
        assert!(matches!(after_terminal, Err(ReduceError::Guard(_))));
    }

    #[test]
    fn replay_rejects_mixed_objective_facts_with_the_exact_index() {
        let (mut trail, _) = attempting_trail();
        let foreign = ObjectiveId::new("objective-2");
        trail[1].objective = foreign.clone();

        assert!(matches!(
            replay(&trail),
            Err(ReplayError::MixedObjective {
                fact_index: 1,
                expected,
                found,
            }) if expected == objective_id() && found == foreign
        ));
    }

    #[test]
    fn replay_rejects_fact_objective_not_bound_to_input_and_current_objective() {
        let specification = objective_spec(1, 1);
        let foreign = ObjectiveId::new("objective-2");
        let mismatched_activation = TrailFact {
            objective: foreign.clone(),
            input: activation(&specification),
        };
        assert!(matches!(
            replay(&[mismatched_activation]),
            Err(ReplayError::ObjectiveBindingMismatch {
                fact_index: 0,
                transition: TransitionKind::ActivateObjective,
                fact_objective,
                input_objective: Some(input_objective),
                current_objective: None,
            }) if fact_objective == foreign && input_objective == objective_id()
        ));

        let mut foreign_revision = objective_spec(2, 1);
        foreign_revision.objective = foreign.clone();
        let trail = vec![
            fact(activation(&specification)),
            fact(TransitionInput::ReviseObjective(ReviseObjectiveInput {
                objective_spec: foreign_revision.clone(),
                confirmation: confirmation(&foreign_revision, ObjectiveConfirmationAction::Revise),
            })),
        ];
        assert!(matches!(
            replay(&trail),
            Err(ReplayError::ObjectiveBindingMismatch {
                fact_index: 1,
                transition: TransitionKind::ReviseObjective,
                fact_objective,
                input_objective: Some(input_objective),
                current_objective: Some(current_objective),
            }) if fact_objective == objective_id()
                && input_objective == foreign
                && current_objective == objective_id()
        ));
    }

    #[test]
    fn i6_replay_rechecks_evidence_admission_at_the_historical_prestate() {
        let (mut trail, attempt) = attempting_trail();
        let fact_index = trail.len();
        let invalid = evidence(
            9,
            AttemptId::new("not-the-current-attempt"),
            1,
            acceptance_context(1, None),
        );
        trail.push(fact(TransitionInput::RecordEvidence(RecordEvidenceInput {
            evidence: invalid,
        })));
        assert!(matches!(
            replay(&trail),
            Err(ReplayError::TransitionRejected {
                fact_index: index,
                transition: TransitionKind::RecordEvidence,
                source: ReduceError::Guard(GuardViolation { code: "evidence_subject", .. }),
            }) if index == fact_index
        ));

        // Defensive audit coverage: even a hypothetical reducer bug that adds unrecorded Evidence
        // is detected from the exact Omega delta at this Trail position.
        let (prefix, _) = attempting_trail();
        let before = replay(&prefix).expect("attempting pre-state");
        let remap = fact(TransitionInput::RequestRemap(RequestRemapInput {
            reason: "dependency drift".into(),
        }));
        let mut corrupted_after = reduce(&before, &remap.input).expect("valid remap successor");
        let unexpected = evidence(10, attempt.identity(), 1, acceptance_context(1, None));
        corrupted_after
            .objects
            .insert_checked(FirstClassObject::Evidence(unexpected))
            .expect("fresh test Evidence");
        assert!(matches!(
            audit_historical_step(prefix.len(), &before, &remap, &corrupted_after),
            Err(ReplayError::HistoricalInvariant {
                invariant: "I6",
                ..
            })
        ));
    }

    #[test]
    fn i17_replay_requires_the_exact_confirmed_abandon_fact() {
        let (mut trail, _) = attempting_trail();
        let fact_index = trail.len();
        let reason = "stop this Objective".to_string();
        trail.push(fact(TransitionInput::Abandon(AbandonInput {
            reason: reason.clone(),
            confirmation: AbandonConfirmation {
                project: ProjectId::new("project-1"),
                objective: objective_id(),
                reason,
                heads: HeadBinding {
                    expected_project_seq: 0,
                    expected_objective_seq: 0,
                },
                confirmed: false,
            },
        })));

        assert!(matches!(
            replay(&trail),
            Err(ReplayError::TransitionRejected {
                fact_index: index,
                transition: TransitionKind::Abandon,
                source: ReduceError::Guard(GuardViolation {
                    code: "human_confirmation_missing",
                    ..
                }),
            }) if index == fact_index
        ));
    }

    #[test]
    fn i19_replay_derives_route_rejection_only_from_the_two_modeled_facts() {
        let (mut replacement_trail, _, packet) = reviewing_trail();
        replacement_trail.push(fact(TransitionInput::Decision(DecisionInput {
            decision: decision(7, &packet, 1, ReviewAction::Replace),
        })));
        let replaced = replay(&replacement_trail).expect("replace Trail");
        assert_eq!(
            replaced.lifecycle.route_status.get(&route_id(1)),
            Some(&RouteStatus::Rejected)
        );
        audit_trail(&replacement_trail, &replaced).expect("replace audit");

        let (mut wait_trail, _, packet) = reviewing_trail();
        let condition = WaitCondition {
            id: WaitConditionId::new("wait-for-i19"),
            stage: stage_id(1),
            context: acceptance_context(1, None),
            cause: "external fact unavailable".into(),
            responsible_party: "environment".into(),
            resume_condition: "new observation exists".into(),
        };
        wait_trail.push(fact(TransitionInput::Decision(DecisionInput {
            decision: decision(
                8,
                &packet,
                1,
                ReviewAction::Wait(Box::new(condition.clone())),
            ),
        })));
        let observation = wait_evidence(8, &condition);
        let observation_id = observation.identity();
        wait_trail.push(fact(TransitionInput::CheckWait(CheckWaitInput {
            wait_condition: condition.identity(),
            evidence: BTreeMap::from([(observation_id.clone(), observation)]),
            judgment: WaitJudgment {
                wait_condition: condition.identity(),
                evidence_set: BTreeSet::from([observation_id]),
                direction: WaitDirection::NewRoute,
                rationale: "the old Route is invalid".into(),
            },
        })));
        let wait_rejected = replay(&wait_trail).expect("new-route wait Trail");
        assert_eq!(
            wait_rejected.lifecycle.route_status.get(&route_id(1)),
            Some(&RouteStatus::Rejected)
        );
        audit_trail(&wait_trail, &wait_rejected).expect("wait audit");

        let (prefix, _) = attempting_trail();
        let before = replay(&prefix).expect("attempting pre-state");
        let remap = fact(TransitionInput::RequestRemap(RequestRemapInput {
            reason: "ordinary remap".into(),
        }));
        let mut corrupted_after = reduce(&before, &remap.input).expect("valid remap successor");
        corrupted_after
            .lifecycle
            .route_status
            .insert(route_id(1), RouteStatus::Rejected);
        assert!(matches!(
            audit_historical_step(prefix.len(), &before, &remap, &corrupted_after),
            Err(ReplayError::HistoricalInvariant {
                invariant: "I19",
                ..
            })
        ));
    }

    #[test]
    fn replay_is_deterministic_and_equals_manual_prefix_fold() {
        let (mut trail, _attempt, packet) = reviewing_trail();
        trail.push(fact(TransitionInput::Decision(DecisionInput {
            decision: decision(1, &packet, 1, ReviewAction::Accept),
        })));

        let first = replay(&trail).expect("first replay");
        let second = replay(&trail).expect("second replay");
        assert_invariants(&first);
        assert_eq!(first, second);
        assert_eq!(trail[0].transition(), TransitionKind::ActivateObjective);
        audit_trail(&trail, &first).expect("I15 projection equality");

        let manual = trail
            .iter()
            .try_fold(initial_configuration(), |configuration, fact| {
                reduce(&configuration, &fact.input)
            })
            .expect("manual fold");
        assert_eq!(first, manual);

        let mut mismatched_projection = first.clone();
        mismatched_projection
            .lifecycle
            .route_status
            .insert(route_id(1), RouteStatus::Rejected);
        assert!(matches!(
            audit_trail(&trail, &mismatched_projection),
            Err(ReplayError::ProjectionMismatch {
                objective: Some(objective)
            }) if objective == objective_id()
        ));
    }
}
