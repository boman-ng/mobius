//! Pure guards and derived queries for the Mobius domain model.
//!
//! Every function in this module is deterministic and reads only typed domain values.  It never
//! reads a clock, filesystem, environment, runtime identity, transport, or projection store.
//!
//! `audit_invariants` deliberately audits only properties observable in a
//! [`DomainConfiguration`].  The configuration does not contain the immutable Trail or its event
//! order, so a configuration-only audit cannot prove the historical portions of I6 (the exact
//! Evidence admission pre-state), I15 (replay equality), I17 (the historical human confirmation
//! attached to Abandon), or the `CheckWait(new_route)` branch of I19.  Live guards below enforce
//! their admission-time portions.  A Trail/replay audit owner must prove the remaining historical
//! claims; this module never reports configuration validity as proof of those claims.

use super::types::*;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display};

/// A stable, structured rejection from a Programmatic guard.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GuardViolation {
    pub code: &'static str,
    pub message: String,
}

impl GuardViolation {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl Display for GuardViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for GuardViolation {}

/// A configuration-observable invariant failure.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct InvariantViolation {
    pub invariant: &'static str,
    pub message: String,
}

impl InvariantViolation {
    fn new(invariant: &'static str, message: impl Into<String>) -> Self {
        Self {
            invariant,
            message: message.into(),
        }
    }
}

fn guard<T>(
    condition: bool,
    code: &'static str,
    message: impl FnOnce() -> String,
) -> Result<T, GuardViolation>
where
    T: Default,
{
    if condition {
        Ok(T::default())
    } else {
        Err(GuardViolation::new(code, message()))
    }
}

fn ensure(
    condition: bool,
    code: &'static str,
    message: impl FnOnce() -> String,
) -> Result<(), GuardViolation> {
    guard::<()>(condition, code, message)
}

fn non_empty(value: &str, code: &'static str, field: &str) -> Result<(), GuardViolation> {
    ensure(!value.trim().is_empty(), code, || {
        format!("{field} must not be empty")
    })
}

fn validate_objective_id(id: &ObjectiveId) -> Result<(), GuardViolation> {
    non_empty(id.as_str(), "empty_identity", "Objective identity")
}

fn validate_stage_id(id: &StageId) -> Result<(), GuardViolation> {
    non_empty(id.as_str(), "empty_identity", "Stage identity")
}

fn validate_criterion_id(id: &CriterionId) -> Result<(), GuardViolation> {
    non_empty(id.as_str(), "empty_identity", "Criterion identity")
}

fn validate_route_id(id: &RouteId) -> Result<(), GuardViolation> {
    non_empty(id.as_str(), "empty_identity", "Route identity")
}

fn validate_attempt_id(id: &AttemptId) -> Result<(), GuardViolation> {
    non_empty(id.as_str(), "empty_identity", "Attempt identity")
}

fn validate_evidence_id(id: &EvidenceId) -> Result<(), GuardViolation> {
    non_empty(id.as_str(), "empty_identity", "Evidence identity")
}

fn validate_packet_id(id: &ReviewPacketId) -> Result<(), GuardViolation> {
    non_empty(id.as_str(), "empty_identity", "ReviewPacket identity")
}

fn validate_decision_id(id: &ReviewDecisionId) -> Result<(), GuardViolation> {
    non_empty(id.as_str(), "empty_identity", "ReviewDecision identity")
}

fn validate_wait_id(id: &WaitConditionId) -> Result<(), GuardViolation> {
    non_empty(id.as_str(), "empty_identity", "WaitCondition identity")
}

fn object<'a>(
    configuration: &'a DomainConfiguration,
    identity: &ObjectIdentity,
) -> Result<&'a FirstClassObject, GuardViolation> {
    configuration.objects.get(identity).ok_or_else(|| {
        GuardViolation::new(
            "missing_object",
            format!("missing first-class object {identity:?}"),
        )
    })
}

fn objective<'a>(
    configuration: &'a DomainConfiguration,
    id: &ObjectiveId,
) -> Result<&'a Objective, GuardViolation> {
    match object(configuration, &ObjectIdentity::Objective(id.clone()))? {
        FirstClassObject::Objective(value) => Ok(value),
        other => Err(GuardViolation::new(
            "object_kind_mismatch",
            format!("expected Objective {id:?}, found {:?}", other.kind()),
        )),
    }
}

fn objective_spec<'a>(
    configuration: &'a DomainConfiguration,
    id: &ObjectiveSpecId,
) -> Result<&'a ObjectiveSpec, GuardViolation> {
    match object(configuration, &ObjectIdentity::ObjectiveSpec(id.clone()))? {
        FirstClassObject::ObjectiveSpec(value) => Ok(value),
        other => Err(GuardViolation::new(
            "object_kind_mismatch",
            format!("expected ObjectiveSpec {id:?}, found {:?}", other.kind()),
        )),
    }
}

fn map_revision<'a>(
    configuration: &'a DomainConfiguration,
    id: &MapRevisionId,
) -> Result<&'a MapRevision, GuardViolation> {
    match object(configuration, &ObjectIdentity::MapRevision(id.clone()))? {
        FirstClassObject::MapRevision(value) => Ok(value),
        other => Err(GuardViolation::new(
            "object_kind_mismatch",
            format!("expected MapRevision {id:?}, found {:?}", other.kind()),
        )),
    }
}

fn route<'a>(
    configuration: &'a DomainConfiguration,
    id: &RouteId,
) -> Result<&'a Route, GuardViolation> {
    match object(configuration, &ObjectIdentity::Route(id.clone()))? {
        FirstClassObject::Route(value) => Ok(value),
        other => Err(GuardViolation::new(
            "object_kind_mismatch",
            format!("expected Route {id:?}, found {:?}", other.kind()),
        )),
    }
}

fn attempt<'a>(
    configuration: &'a DomainConfiguration,
    id: &AttemptId,
) -> Result<&'a Attempt, GuardViolation> {
    match object(configuration, &ObjectIdentity::Attempt(id.clone()))? {
        FirstClassObject::Attempt(value) => Ok(value),
        other => Err(GuardViolation::new(
            "object_kind_mismatch",
            format!("expected Attempt {id:?}, found {:?}", other.kind()),
        )),
    }
}

fn evidence<'a>(
    configuration: &'a DomainConfiguration,
    id: &EvidenceId,
) -> Result<&'a Evidence, GuardViolation> {
    match object(configuration, &ObjectIdentity::Evidence(id.clone()))? {
        FirstClassObject::Evidence(value) => Ok(value),
        other => Err(GuardViolation::new(
            "object_kind_mismatch",
            format!("expected Evidence {id:?}, found {:?}", other.kind()),
        )),
    }
}

fn packet<'a>(
    configuration: &'a DomainConfiguration,
    id: &ReviewPacketId,
) -> Result<&'a ReviewPacket, GuardViolation> {
    match object(configuration, &ObjectIdentity::ReviewPacket(id.clone()))? {
        FirstClassObject::ReviewPacket(value) => Ok(value),
        other => Err(GuardViolation::new(
            "object_kind_mismatch",
            format!("expected ReviewPacket {id:?}, found {:?}", other.kind()),
        )),
    }
}

fn decision<'a>(
    configuration: &'a DomainConfiguration,
    id: &ReviewDecisionId,
) -> Result<&'a ReviewDecision, GuardViolation> {
    match object(configuration, &ObjectIdentity::ReviewDecision(id.clone()))? {
        FirstClassObject::ReviewDecision(value) => Ok(value),
        other => Err(GuardViolation::new(
            "object_kind_mismatch",
            format!("expected ReviewDecision {id:?}, found {:?}", other.kind()),
        )),
    }
}

fn wait_condition<'a>(
    configuration: &'a DomainConfiguration,
    id: &WaitConditionId,
) -> Result<&'a WaitCondition, GuardViolation> {
    match object(configuration, &ObjectIdentity::WaitCondition(id.clone()))? {
        FirstClassObject::WaitCondition(value) => Ok(value),
        other => Err(GuardViolation::new(
            "object_kind_mismatch",
            format!("expected WaitCondition {id:?}, found {:?}", other.kind()),
        )),
    }
}

fn ensure_fresh(
    configuration: &DomainConfiguration,
    proposed: FirstClassObject,
) -> Result<(), GuardViolation> {
    let identity = proposed.identity();
    match configuration.objects.get(&identity) {
        None => Ok(()),
        Some(existing) if existing == &proposed => Err(GuardViolation::new(
            "not_fresh",
            format!("object {identity:?} is already admitted"),
        )),
        Some(_) => Err(GuardViolation::new(
            "identity_conflict",
            format!("object {identity:?} conflicts with the admitted value"),
        )),
    }
}

fn ensure_compatible_or_new(
    configuration: &DomainConfiguration,
    proposed: FirstClassObject,
) -> Result<(), GuardViolation> {
    let identity = proposed.identity();
    match configuration.objects.get(&identity) {
        None => Ok(()),
        Some(existing) if existing == &proposed => Ok(()),
        Some(_) => Err(GuardViolation::new(
            "identity_conflict",
            format!("object {identity:?} conflicts with the admitted value"),
        )),
    }
}

fn selected_map_id(configuration: &DomainConfiguration) -> Option<&MapRevisionId> {
    match &configuration.objective_state {
        ObjectiveState::Mapping { previous_map, .. } => previous_map.as_ref(),
        ObjectiveState::Navigating { map, .. } | ObjectiveState::Achieved { map, .. } => Some(map),
        ObjectiveState::Idle | ObjectiveState::Abandoned { .. } => None,
    }
}

fn selected_map(
    configuration: &DomainConfiguration,
) -> Result<Option<&MapRevision>, GuardViolation> {
    selected_map_id(configuration)
        .map(|id| map_revision(configuration, id))
        .transpose()
}

fn active_objective(configuration: &DomainConfiguration) -> Result<&ObjectiveId, GuardViolation> {
    match &configuration.objective_state {
        ObjectiveState::Mapping { objective, .. }
        | ObjectiveState::Navigating { objective, .. } => Ok(objective),
        ObjectiveState::Idle => Err(GuardViolation::new(
            "objective_not_active",
            "no Objective is active",
        )),
        ObjectiveState::Achieved { .. } | ObjectiveState::Abandoned { .. } => {
            Err(GuardViolation::new(
                "terminal_state",
                "terminal Objective rejects business transitions",
            ))
        }
    }
}

fn validate_objective_spec_shape(specification: &ObjectiveSpec) -> Result<(), GuardViolation> {
    validate_objective_id(&specification.objective)?;
    ensure(!specification.criteria.is_empty(), "criteria_empty", || {
        "ObjectiveSpec must contain at least one Criterion".into()
    })?;
    for (id, criterion) in &specification.criteria {
        validate_criterion_id(id)?;
        ensure(id == &criterion.id, "identity_key_mismatch", || {
            format!(
                "Criterion map key {id:?} does not match value identity {:?}",
                criterion.id
            )
        })?;
    }
    Ok(())
}

fn validate_confirmation(
    confirmation: &ObjectiveConfirmation,
    expected_action: ObjectiveConfirmationAction,
    specification: &ObjectiveSpec,
) -> Result<(), GuardViolation> {
    non_empty(
        confirmation.project.as_str(),
        "confirmation_project_empty",
        "confirmation project identity",
    )?;
    ensure(confirmation.confirmed, "human_confirmation_missing", || {
        "Objective transition requires explicit human confirmation".into()
    })?;
    ensure(
        confirmation.action == expected_action,
        "confirmation_action_mismatch",
        || {
            format!(
                "confirmation action {:?} does not match expected {expected_action:?}",
                confirmation.action
            )
        },
    )?;
    ensure(
        confirmation.objective_spec == specification.identity(),
        "confirmation_identity_mismatch",
        || "confirmation is not bound to the command ObjectiveSpec identity".into(),
    )?;
    ensure(
        confirmation.confirmed_payload.as_ref() == specification,
        "confirmation_payload_mismatch",
        || "confirmation is not bound to the complete typed ObjectiveSpec payload".into(),
    )
}

fn direct_dependencies(map: &MapRevision, stage: &StageId) -> BTreeSet<StageId> {
    map.dependencies
        .iter()
        .filter(|edge| &edge.dependent == stage)
        .map(|edge| edge.dependency.clone())
        .collect()
}

fn graph_endpoints(map: &MapRevision) -> Result<(), GuardViolation> {
    for edge in &map.dependencies {
        ensure(
            map.stages.contains_key(&edge.dependency),
            "dependency_endpoint_missing",
            || {
                format!(
                    "dependency endpoint {:?} is not a Stage in the Map",
                    edge.dependency
                )
            },
        )?;
        ensure(
            map.stages.contains_key(&edge.dependent),
            "dependency_endpoint_missing",
            || {
                format!(
                    "dependent endpoint {:?} is not a Stage in the Map",
                    edge.dependent
                )
            },
        )?;
    }
    Ok(())
}

fn topological_order(map: &MapRevision) -> Result<Vec<StageId>, GuardViolation> {
    graph_endpoints(map)?;
    let mut indegree: BTreeMap<StageId, usize> =
        map.stages.keys().cloned().map(|stage| (stage, 0)).collect();
    let mut dependents: BTreeMap<StageId, BTreeSet<StageId>> = BTreeMap::new();
    for edge in &map.dependencies {
        *indegree
            .get_mut(&edge.dependent)
            .expect("graph_endpoints checked dependent") += 1;
        dependents
            .entry(edge.dependency.clone())
            .or_default()
            .insert(edge.dependent.clone());
    }

    let mut ready: BTreeSet<StageId> = indegree
        .iter()
        .filter_map(|(stage, count)| (*count == 0).then_some(stage.clone()))
        .collect();
    let mut order = Vec::with_capacity(map.stages.len());
    while let Some(stage) = ready.pop_first() {
        order.push(stage.clone());
        if let Some(children) = dependents.get(&stage) {
            for child in children {
                let count = indegree
                    .get_mut(child)
                    .expect("graph_endpoints checked child");
                *count -= 1;
                if *count == 0 {
                    ready.insert(child.clone());
                }
            }
        }
    }

    ensure(order.len() == map.stages.len(), "map_cycle", || {
        "Stage dependency graph must be acyclic".into()
    })?;
    Ok(order)
}

fn transitive_dependencies(
    map: &MapRevision,
    stage: &StageId,
) -> Result<BTreeSet<StageId>, GuardViolation> {
    let _ = topological_order(map)?;
    let mut result = BTreeSet::new();
    let mut pending: Vec<_> = direct_dependencies(map, stage).into_iter().collect();
    while let Some(dependency) = pending.pop() {
        if result.insert(dependency.clone()) {
            pending.extend(direct_dependencies(map, &dependency));
        }
    }
    Ok(result)
}

fn validate_map_shape(
    map: &MapRevision,
    specification: &ObjectiveSpec,
) -> Result<(), GuardViolation> {
    validate_objective_spec_shape(specification)?;
    ensure(
        map.objective_spec == specification.identity(),
        "map_spec_mismatch",
        || "MapRevision is not bound to the current ObjectiveSpec revision".into(),
    )?;
    ensure(!map.stages.is_empty(), "map_stages_empty", || {
        "MapRevision must contain at least one Stage".into()
    })?;

    for (id, stage) in &map.stages {
        validate_stage_id(id)?;
        ensure(id == &stage.id, "identity_key_mismatch", || {
            format!(
                "Stage map key {id:?} does not match value identity {:?}",
                stage.id
            )
        })?;
    }
    for (id, criterion) in &map.criteria {
        validate_criterion_id(id)?;
        ensure(id == &criterion.id, "identity_key_mismatch", || {
            format!(
                "Criterion map key {id:?} does not match value identity {:?}",
                criterion.id
            )
        })?;
    }

    for (id, objective_criterion) in &specification.criteria {
        let map_criterion = map.criteria.get(id).ok_or_else(|| {
            GuardViolation::new(
                "objective_criterion_missing",
                format!("Map omits Objective Criterion {id:?}"),
            )
        })?;
        ensure(
            map_criterion == objective_criterion,
            "identity_conflict",
            || format!("Map Criterion {id:?} conflicts with the ObjectiveSpec value"),
        )?;
    }

    let stage_ids: BTreeSet<_> = map.stages.keys().cloned().collect();
    let priority_ids: BTreeSet<_> = map.priorities.keys().cloned().collect();
    ensure(priority_ids == stage_ids, "priority_not_total", || {
        "Map priority function must be total over exactly the Stage set".into()
    })?;

    let criterion_ids: BTreeSet<_> = map.criteria.keys().cloned().collect();
    let owner_ids: BTreeSet<_> = map.owners.keys().cloned().collect();
    ensure(owner_ids == criterion_ids, "owner_not_total", || {
        "Criterion owner function must be total over exactly the Map Criterion set".into()
    })?;
    for (criterion, stage) in &map.owners {
        ensure(
            map.stages.contains_key(stage),
            "owner_stage_missing",
            || format!("Criterion {criterion:?} is owned by missing Stage {stage:?}"),
        )?;
    }

    let contract_ids: BTreeSet<_> = map.contracts.keys().cloned().collect();
    ensure(contract_ids == stage_ids, "contract_not_total", || {
        "Stage contract function must be total over exactly the Stage set".into()
    })?;

    for (stage_id, stage) in &map.stages {
        let owned: BTreeSet<_> = map
            .owners
            .iter()
            .filter_map(|(criterion, owner)| (owner == stage_id).then_some(criterion.clone()))
            .collect();
        ensure(!owned.is_empty(), "stage_criteria_empty", || {
            format!("Stage {stage_id:?} must own at least one Criterion")
        })?;
        let contract = map.contracts.get(stage_id).ok_or_else(|| {
            GuardViolation::new(
                "contract_missing",
                format!("Stage {stage_id:?} has no contract"),
            )
        })?;
        ensure(
            contract.outcome == stage.outcome,
            "contract_outcome_mismatch",
            || format!("Stage {stage_id:?} contract outcome does not match the Stage outcome"),
        )?;
        ensure(
            contract.output == stage.output,
            "contract_output_mismatch",
            || format!("Stage {stage_id:?} contract output does not match the Stage output"),
        )?;
        ensure(
            contract.criteria == owned,
            "contract_criteria_mismatch",
            || format!("Stage {stage_id:?} contract Criteria do not equal its owned Criteria"),
        )?;
        ensure(
            contract
                .objective_boundaries
                .is_subset(&specification.boundaries),
            "contract_boundary_unknown",
            || format!("Stage {stage_id:?} contract contains a boundary absent from ObjectiveSpec"),
        )?;
    }

    let _ = topological_order(map)?;
    let final_stages: Vec<_> = map
        .stages
        .values()
        .filter(|stage| stage.kind == StageKind::FinalIntegration)
        .map(|stage| stage.id.clone())
        .collect();
    ensure(
        final_stages.len() <= 1,
        "final_integration_not_unique",
        || "Map may contain at most one final-integration Stage".into(),
    )?;

    let cross_stage: Vec<_> = map
        .criteria
        .values()
        .filter(|criterion| criterion.scope == CriterionScope::CrossStage)
        .map(|criterion| criterion.id.clone())
        .collect();
    if !cross_stage.is_empty() {
        ensure(final_stages.len() == 1, "final_integration_missing", || {
            "cross-stage Criteria require a final-integration Stage".into()
        })?;
        let final_stage = &final_stages[0];
        for criterion in cross_stage {
            ensure(
                map.owners.get(&criterion) == Some(final_stage),
                "cross_stage_owner_mismatch",
                || {
                    format!(
                        "cross-stage Criterion {criterion:?} must be owned by final integration"
                    )
                },
            )?;
        }
    }

    if let Some(final_stage) = final_stages.first() {
        let expected: BTreeSet<_> = map
            .stages
            .keys()
            .filter(|stage| *stage != final_stage)
            .cloned()
            .collect();
        ensure(
            transitive_dependencies(map, final_stage)? == expected,
            "final_integration_dependency_coverage",
            || "final-integration Stage must transitively depend on every ordinary Stage".into(),
        )?;
    }

    Ok(())
}

fn structural_context_inner(
    map: &MapRevision,
    stage: &StageId,
    visiting: &mut BTreeSet<StageId>,
) -> Result<StructuralContext, GuardViolation> {
    ensure(map.stages.contains_key(stage), "stage_missing", || {
        format!("Stage {stage:?} is not in the Map")
    })?;
    ensure(visiting.insert(stage.clone()), "map_cycle", || {
        format!("cycle encountered while deriving StructuralContext at {stage:?}")
    })?;
    let contract = map.contracts.get(stage).cloned().ok_or_else(|| {
        GuardViolation::new(
            "contract_missing",
            format!("Stage {stage:?} has no contract"),
        )
    })?;
    let mut dependencies = BTreeMap::new();
    for dependency in direct_dependencies(map, stage) {
        let dependency_stage = map.stages.get(&dependency).ok_or_else(|| {
            GuardViolation::new(
                "dependency_endpoint_missing",
                format!("dependency Stage {dependency:?} is missing"),
            )
        })?;
        let context = structural_context_inner(map, &dependency, visiting)?;
        dependencies.insert(
            dependency.clone(),
            DependencyStructuralContext {
                output: dependency_stage.output.clone(),
                context: Box::new(context),
            },
        );
    }
    visiting.remove(stage);
    Ok(StructuralContext {
        contract,
        dependencies,
    })
}

/// Derive `kappa_mu(stage)` from a Map without reading runtime or persistence state.
pub fn structural_context(
    map: &MapRevision,
    stage: &StageId,
) -> Result<StructuralContext, GuardViolation> {
    graph_endpoints(map)?;
    structural_context_inner(map, stage, &mut BTreeSet::new())
}

fn acceptance_context_with_proofs(
    map: &MapRevision,
    stage: &StageId,
    proofs: &BTreeMap<StageId, ReviewDecisionId>,
) -> Result<AcceptanceContext, GuardViolation> {
    let mut dependency_proofs = BTreeMap::new();
    for dependency in direct_dependencies(map, stage) {
        let proof = proofs.get(&dependency).cloned().ok_or_else(|| {
            GuardViolation::new(
                "dependency_proof_missing",
                format!("Stage {stage:?} dependency {dependency:?} has no current proof"),
            )
        })?;
        dependency_proofs.insert(dependency, proof);
    }
    Ok(AcceptanceContext {
        structural: structural_context(map, stage)?,
        dependency_proofs,
    })
}

fn criteria_for_stage<'a>(
    map: &'a MapRevision,
    stage: &StageId,
) -> Result<&'a BTreeSet<CriterionId>, GuardViolation> {
    map.contracts
        .get(stage)
        .map(|contract| &contract.criteria)
        .ok_or_else(|| {
            GuardViolation::new(
                "contract_missing",
                format!("Stage {stage:?} has no contract"),
            )
        })
}

fn proofs_for_map(
    configuration: &DomainConfiguration,
    map: &MapRevision,
) -> Result<BTreeMap<StageId, ReviewDecisionId>, GuardViolation> {
    let specification = objective_spec(configuration, &map.objective_spec)?;
    validate_map_shape(map, specification)?;
    let mut proofs = BTreeMap::new();
    for stage in topological_order(map)? {
        let context = match acceptance_context_with_proofs(map, &stage, &proofs) {
            Ok(context) => context,
            Err(error) if error.code == "dependency_proof_missing" => continue,
            Err(error) => return Err(error),
        };
        let criteria = criteria_for_stage(map, &stage)?;
        let mut candidates = Vec::new();
        for value in configuration.objects.values() {
            let FirstClassObject::ReviewDecision(candidate) = value else {
                continue;
            };
            if !matches!(&candidate.action, ReviewAction::Accept)
                || configuration
                    .lifecycle
                    .invalidated_proofs
                    .contains(&candidate.id)
            {
                continue;
            }
            let candidate_packet = packet(configuration, &candidate.packet)?;
            if candidate_packet.stage != stage || candidate_packet.context != context {
                continue;
            }
            let judgment_domain: BTreeSet<_> = candidate.judgments.keys().cloned().collect();
            ensure(
                judgment_domain == *criteria,
                "decision_criteria_domain",
                || {
                    format!(
                        "accepted Decision {:?} does not judge the complete Stage domain",
                        candidate.id
                    )
                },
            )?;
            ensure(
                candidate
                    .judgments
                    .values()
                    .all(|judgment| *judgment == CriterionJudgment::Satisfied),
                "accept_not_satisfied",
                || {
                    format!(
                        "accepted Decision {:?} contains a non-satisfied judgment",
                        candidate.id
                    )
                },
            )?;
            candidates.push(candidate.id.clone());
        }
        ensure(candidates.len() <= 1, "multiple_current_proofs", || {
            format!("Stage {stage:?} has more than one current accepted Decision")
        })?;
        if let Some(proof) = candidates.pop() {
            proofs.insert(stage, proof);
        }
    }
    Ok(proofs)
}

/// Derive the current proof function for the selected Map.
///
/// While Mapping, `previous_map` is selected so InstallMap can evaluate carry.  Reducers may place
/// a newly installed Map in a private provisional Navigating state before using this query; the
/// query does not rely on the provisional navigation Stage.
pub fn current_proofs(
    configuration: &DomainConfiguration,
) -> Result<BTreeMap<StageId, ReviewDecisionId>, GuardViolation> {
    match selected_map(configuration)? {
        Some(map) => proofs_for_map(configuration, map),
        None => Ok(BTreeMap::new()),
    }
}

/// Derive `chi_mu,q(stage)`, requiring current proofs for every direct dependency.
pub fn acceptance_context(
    configuration: &DomainConfiguration,
    stage: &StageId,
) -> Result<AcceptanceContext, GuardViolation> {
    let map = selected_map(configuration)?.ok_or_else(|| {
        GuardViolation::new("map_missing", "AcceptanceContext requires a selected Map")
    })?;
    let proofs = proofs_for_map(configuration, map)?;
    acceptance_context_with_proofs(map, stage, &proofs)
}

/// Whether every Stage in the selected Map has a current proof.
pub fn complete(configuration: &DomainConfiguration) -> Result<bool, GuardViolation> {
    let Some(map) = selected_map(configuration)? else {
        return Ok(false);
    };
    let proofs = proofs_for_map(configuration, map)?;
    Ok(map.stages.keys().all(|stage| proofs.contains_key(stage)))
}

/// Deterministically select the next schedulable Stage by `(priority, Stage identity)`.
pub fn next_stage(configuration: &DomainConfiguration) -> Result<Option<StageId>, GuardViolation> {
    let Some(map) = selected_map(configuration)? else {
        return Ok(None);
    };
    let proofs = proofs_for_map(configuration, map)?;
    let mut candidates = Vec::new();
    for stage in map.stages.keys() {
        if proofs.contains_key(stage)
            || !direct_dependencies(map, stage)
                .iter()
                .all(|dependency| proofs.contains_key(dependency))
        {
            continue;
        }
        let priority = map.priorities.get(stage).copied().ok_or_else(|| {
            GuardViolation::new(
                "priority_missing",
                format!("Stage {stage:?} has no scheduling priority"),
            )
        })?;
        candidates.push((priority, stage.clone()));
    }
    candidates.sort();
    Ok(candidates.into_iter().next().map(|(_, stage)| stage))
}

/// The complete admitted Stage Evidence universe for a Stage and Acceptance Context.
pub fn evidence_universe(
    configuration: &DomainConfiguration,
    stage: &StageId,
    context: &AcceptanceContext,
) -> Result<BTreeSet<EvidenceId>, GuardViolation> {
    let mut result = BTreeSet::new();
    for value in configuration.objects.values() {
        let FirstClassObject::Evidence(candidate) = value else {
            continue;
        };
        if candidate.purpose != EvidencePurpose::StageReview || &candidate.context != context {
            continue;
        }
        let EvidenceSubject::Attempt(attempt_id) = &candidate.subject else {
            continue;
        };
        let candidate_attempt = attempt(configuration, attempt_id)?;
        let candidate_route = route(configuration, &candidate_attempt.route)?;
        if &candidate_route.stage == stage {
            result.insert(candidate.id.clone());
        }
    }
    Ok(result)
}

fn wait_evidence_universe(
    configuration: &DomainConfiguration,
    wait: &WaitCondition,
) -> Result<BTreeSet<EvidenceId>, GuardViolation> {
    let mut result = BTreeSet::new();
    for value in configuration.objects.values() {
        let FirstClassObject::Evidence(candidate) = value else {
            continue;
        };
        if candidate.purpose == EvidencePurpose::WaitResolution
            && candidate.context == wait.context
            && candidate.subject == EvidenceSubject::WaitCondition(wait.id.clone())
        {
            result.insert(candidate.id.clone());
        }
    }
    Ok(result)
}

fn dependency_decision_view(
    configuration: &DomainConfiguration,
    id: &ReviewDecisionId,
    visiting: &mut BTreeSet<ReviewDecisionId>,
    result: &mut BTreeSet<ReviewDecisionId>,
) -> Result<(), GuardViolation> {
    if result.contains(id) {
        return Ok(());
    }
    ensure(visiting.insert(id.clone()), "dependency_view_cycle", || {
        format!("cycle in dependency proof view at Decision {id:?}")
    })?;
    let proof = decision(configuration, id)?;
    ensure(
        matches!(&proof.action, ReviewAction::Accept),
        "dependency_not_proof",
        || format!("Decision {id:?} in DependencyView is not an accept Decision"),
    )?;
    let proof_packet = packet(configuration, &proof.packet)?;
    for dependency in proof_packet.context.dependency_proofs.values() {
        dependency_decision_view(configuration, dependency, visiting, result)?;
    }
    visiting.remove(id);
    result.insert(id.clone());
    Ok(())
}

/// Recursively derive the complete dependency proof view frozen in a Packet context.
pub fn dependency_view(
    configuration: &DomainConfiguration,
    review_packet: &ReviewPacket,
) -> Result<BTreeSet<ReviewDecisionId>, GuardViolation> {
    let mut result = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    for proof in review_packet.context.dependency_proofs.values() {
        dependency_decision_view(configuration, proof, &mut visiting, &mut result)?;
    }
    Ok(result)
}

fn validate_frozen_observation(observation: &FrozenObservation) -> Result<(), GuardViolation> {
    if let FrozenObservation::CoreSnapshot(snapshot) = observation {
        non_empty(
            &snapshot.digest.0,
            "snapshot_digest_empty",
            "CoreSnapshot digest",
        )?;
    }
    Ok(())
}

fn validate_wait_condition_shape(wait: &WaitCondition) -> Result<(), GuardViolation> {
    validate_wait_id(&wait.id)?;
    validate_stage_id(&wait.stage)?;
    non_empty(&wait.cause, "wait_cause_empty", "WaitCondition cause")?;
    non_empty(
        &wait.responsible_party,
        "wait_responsible_party_empty",
        "WaitCondition responsible party",
    )?;
    non_empty(
        &wait.resume_condition,
        "wait_resume_condition_empty",
        "WaitCondition resume condition",
    )
}

fn validate_stage_evidence_admission(
    configuration: &DomainConfiguration,
    candidate: &Evidence,
    stage: &StageId,
    current_attempt: &AttemptId,
) -> Result<(), GuardViolation> {
    validate_evidence_id(&candidate.id)?;
    ensure_fresh(configuration, FirstClassObject::Evidence(candidate.clone()))?;
    validate_frozen_observation(&candidate.observation)?;
    ensure(
        candidate.purpose == EvidencePurpose::StageReview,
        "evidence_purpose",
        || "Attempt Evidence must have stage_review purpose".into(),
    )?;
    ensure(
        candidate.subject == EvidenceSubject::Attempt(current_attempt.clone()),
        "evidence_subject",
        || "Evidence must be bound to the current Attempt".into(),
    )?;
    let current_attempt_value = attempt(configuration, current_attempt)?;
    ensure(
        candidate.context == current_attempt_value.context,
        "evidence_context",
        || "Evidence context must equal the current Attempt context".into(),
    )?;
    ensure(
        candidate.context == acceptance_context(configuration, stage)?,
        "stale_context",
        || "Evidence context is stale relative to the current AcceptanceContext".into(),
    )?;
    let map = selected_map(configuration)?.ok_or_else(|| {
        GuardViolation::new("map_missing", "Evidence admission requires a current Map")
    })?;
    let criteria = criteria_for_stage(map, stage)?;
    ensure(
        candidate
            .claims
            .keys()
            .all(|criterion| criteria.contains(criterion)),
        "evidence_claim_domain",
        || "Evidence claims contain a Criterion outside the current Stage contract".into(),
    )
}

fn validate_wait_evidence_admission(
    configuration: &DomainConfiguration,
    candidate: &Evidence,
    current_wait: &WaitCondition,
) -> Result<(), GuardViolation> {
    validate_evidence_id(&candidate.id)?;
    ensure_fresh(configuration, FirstClassObject::Evidence(candidate.clone()))?;
    validate_frozen_observation(&candidate.observation)?;
    ensure(
        candidate.purpose == EvidencePurpose::WaitResolution,
        "evidence_purpose",
        || "WaitCondition Evidence must have wait_resolution purpose".into(),
    )?;
    ensure(
        candidate.subject == EvidenceSubject::WaitCondition(current_wait.id.clone()),
        "evidence_subject",
        || "wait-resolution Evidence must bind the current WaitCondition".into(),
    )?;
    ensure(
        candidate.context == current_wait.context,
        "evidence_context",
        || "wait-resolution Evidence context must equal the WaitCondition context".into(),
    )?;
    ensure(candidate.claims.is_empty(), "wait_evidence_claims", || {
        "wait-resolution Evidence must not make Stage Criterion claims".into()
    })
}

fn route_status(
    configuration: &DomainConfiguration,
    route: &RouteId,
) -> Result<RouteStatus, GuardViolation> {
    configuration
        .lifecycle
        .route_status
        .get(route)
        .copied()
        .ok_or_else(|| {
            GuardViolation::new(
                "route_lifecycle_missing",
                format!("Route {route:?} has no lifecycle projection"),
            )
        })
}

fn attempt_state(
    configuration: &DomainConfiguration,
    attempt: &AttemptId,
) -> Result<AttemptState, GuardViolation> {
    configuration
        .lifecycle
        .attempt_state
        .get(attempt)
        .copied()
        .ok_or_else(|| {
            GuardViolation::new(
                "attempt_state_missing",
                format!("Attempt {attempt:?} has no state projection"),
            )
        })
}

fn validate_activate(
    configuration: &DomainConfiguration,
    input: &ActivateObjectiveInput,
) -> Result<(), GuardViolation> {
    ensure(
        matches!(configuration.objective_state, ObjectiveState::Idle),
        "wrong_state",
        || "ActivateObjective is enabled only from Idle".into(),
    )?;
    validate_objective_spec_shape(&input.objective_spec)?;
    validate_confirmation(
        &input.confirmation,
        ObjectiveConfirmationAction::Activate,
        &input.objective_spec,
    )?;
    ensure_fresh(
        configuration,
        FirstClassObject::Objective(Objective {
            id: input.objective_spec.objective.clone(),
        }),
    )?;
    ensure_fresh(
        configuration,
        FirstClassObject::ObjectiveSpec(input.objective_spec.clone()),
    )?;
    for criterion in input.objective_spec.criteria.values() {
        ensure_fresh(
            configuration,
            FirstClassObject::Criterion(criterion.clone()),
        )?;
    }
    Ok(())
}

fn validate_initial_routes(
    configuration: &DomainConfiguration,
    map: &MapRevision,
    routes: &BTreeMap<RouteId, Route>,
) -> Result<(), GuardViolation> {
    for (id, candidate) in routes {
        validate_route_id(id)?;
        ensure(id == &candidate.id, "identity_key_mismatch", || {
            format!(
                "Route map key {id:?} does not match value identity {:?}",
                candidate.id
            )
        })?;
        ensure(
            map.stages.contains_key(&candidate.stage),
            "route_stage_mismatch",
            || format!("initial Route {id:?} targets a Stage outside the installed Map"),
        )?;
        ensure(
            candidate.structural_context == structural_context(map, &candidate.stage)?,
            "route_structural_context",
            || format!("initial Route {id:?} has a stale StructuralContext"),
        )?;
        ensure_compatible_or_new(configuration, FirstClassObject::Route(candidate.clone()))?;
    }
    Ok(())
}

fn structurally_eligible_stages(
    configuration: &DomainConfiguration,
    previous: &MapRevision,
    next: &MapRevision,
) -> Result<BTreeMap<StageId, ReviewDecisionId>, GuardViolation> {
    let previous_proofs = proofs_for_map(configuration, previous)?;
    let mut eligible = BTreeMap::new();
    for stage in next.stages.keys() {
        if previous.stages.contains_key(stage)
            && structural_context(previous, stage)? == structural_context(next, stage)?
        {
            if let Some(proof) = previous_proofs.get(stage) {
                eligible.insert(stage.clone(), proof.clone());
            }
        }
    }
    Ok(eligible)
}

fn validate_carry(
    configuration: &DomainConfiguration,
    previous: Option<&MapRevision>,
    next: &MapRevision,
    carry: &BTreeMap<StageId, CarryVerdict>,
) -> Result<(), GuardViolation> {
    let Some(previous) = previous else {
        return ensure(carry.is_empty(), "carry_domain", || {
            "initial Map installation requires an empty carry judgment".into()
        });
    };
    let eligible = structurally_eligible_stages(configuration, previous, next)?;
    let eligible_ids: BTreeSet<_> = eligible.keys().cloned().collect();
    let carry_ids: BTreeSet<_> = carry.keys().cloned().collect();
    ensure(carry_ids == eligible_ids, "carry_domain", || {
        "carry judgment domain must equal exactly the structurally eligible Stage set".into()
    })?;

    let previous_proofs = proofs_for_map(configuration, previous)?;
    let mut carried: BTreeMap<StageId, bool> = BTreeMap::new();
    for stage in topological_order(next)? {
        let Some(verdict) = carry.get(&stage) else {
            carried.insert(stage, false);
            continue;
        };
        if *verdict == CarryVerdict::Invalid {
            carried.insert(stage, false);
            continue;
        }
        ensure(
            direct_dependencies(next, &stage)
                .iter()
                .all(|dependency| carried.get(dependency) == Some(&true)),
            "carry_dependency_closure",
            || format!("Stage {stage:?} cannot carry when a dependency does not carry"),
        )?;

        if next
            .stages
            .get(&stage)
            .is_some_and(|value| value.kind == StageKind::FinalIntegration)
        {
            let proof_id = eligible.get(&stage).expect("carry domain equals eligible");
            let proof = decision(configuration, proof_id)?;
            let proof_packet = packet(configuration, &proof.packet)?;
            let actual = dependency_view(configuration, proof_packet)?;
            let expected: BTreeSet<_> = transitive_dependencies(next, &stage)?
                .into_iter()
                .filter(|dependency| carried.get(dependency) == Some(&true))
                .filter_map(|dependency| previous_proofs.get(&dependency).cloned())
                .collect();
            ensure(actual == expected, "tree_compatible", || {
                format!("final-integration Stage {stage:?} proof tree is not TreeCompatible")
            })?;
        }
        carried.insert(stage, true);
    }
    Ok(())
}

fn validate_install_map(
    configuration: &DomainConfiguration,
    input: &InstallMapInput,
) -> Result<(), GuardViolation> {
    let (objective, current_spec_id, previous_map_id) = match &configuration.objective_state {
        ObjectiveState::Mapping {
            objective,
            objective_spec,
            previous_map,
            ..
        } => (objective, objective_spec, previous_map.as_ref()),
        _ => {
            return Err(GuardViolation::new(
                "wrong_state",
                "InstallMap is enabled only from Mapping",
            ));
        }
    };
    let specification = objective_spec(configuration, current_spec_id)?;
    ensure(
        &specification.objective == objective,
        "objective_mismatch",
        || "Mapping Objective and ObjectiveSpec disagree".into(),
    )?;
    validate_map_shape(&input.map, specification)?;
    ensure_fresh(
        configuration,
        FirstClassObject::MapRevision(input.map.clone()),
    )?;
    for stage in input.map.stages.values() {
        ensure_compatible_or_new(configuration, FirstClassObject::Stage(stage.clone()))?;
    }
    for criterion in input.map.criteria.values() {
        ensure_compatible_or_new(
            configuration,
            FirstClassObject::Criterion(criterion.clone()),
        )?;
    }
    validate_initial_routes(configuration, &input.map, &input.initial_routes)?;
    ensure(
        input.cover.map == input.map.identity(),
        "cover_map_mismatch",
        || "Cover judgment is not bound to the installed Map revision".into(),
    )?;
    ensure(
        input.cover.objective_spec == specification.identity(),
        "cover_spec_mismatch",
        || "Cover judgment is not bound to the current ObjectiveSpec".into(),
    )?;
    ensure(
        input.cover.verdict == CoverVerdict::Covered,
        "cover_not_covered",
        || "Map installation requires an explicit covered judgment".into(),
    )?;
    let previous = previous_map_id
        .map(|id| map_revision(configuration, id))
        .transpose()?;
    validate_carry(configuration, previous, &input.map, &input.carry)
}

fn validate_add_route(
    configuration: &DomainConfiguration,
    input: &AddRouteInput,
) -> Result<(), GuardViolation> {
    let stage = match &configuration.objective_state {
        ObjectiveState::Navigating {
            navigation: NavState::SeekingRoute { stage },
            ..
        } => stage,
        _ => {
            return Err(GuardViolation::new(
                "wrong_state",
                "AddRoute is enabled only while SeekingRoute",
            ));
        }
    };
    validate_route_id(&input.route.id)?;
    ensure(&input.route.stage == stage, "route_stage_mismatch", || {
        "Route must target the current Stage".into()
    })?;
    let map = selected_map(configuration)?
        .ok_or_else(|| GuardViolation::new("map_missing", "AddRoute requires a current Map"))?;
    ensure(
        input.route.structural_context == structural_context(map, stage)?,
        "route_structural_context",
        || "Route StructuralContext does not match the current Map".into(),
    )?;
    ensure_fresh(configuration, FirstClassObject::Route(input.route.clone()))
}

fn validate_select_route(
    configuration: &DomainConfiguration,
    input: &SelectRouteInput,
) -> Result<(), GuardViolation> {
    let stage = match &configuration.objective_state {
        ObjectiveState::Navigating {
            navigation: NavState::SeekingRoute { stage },
            ..
        } => stage,
        _ => {
            return Err(GuardViolation::new(
                "wrong_state",
                "SelectRoute is enabled only while SeekingRoute",
            ));
        }
    };
    let selected = route(configuration, &input.route)?;
    ensure(&selected.stage == stage, "route_stage_mismatch", || {
        "selected Route does not target the current Stage".into()
    })?;
    ensure(
        route_status(configuration, &selected.id)? == RouteStatus::Available,
        "route_rejected",
        || "selected Route is rejected".into(),
    )?;
    let map = selected_map(configuration)?
        .ok_or_else(|| GuardViolation::new("map_missing", "SelectRoute requires a current Map"))?;
    ensure(
        selected.structural_context == structural_context(map, stage)?,
        "route_structural_context",
        || "selected Route StructuralContext is stale".into(),
    )
}

fn validate_start_attempt(
    configuration: &DomainConfiguration,
    input: &StartAttemptInput,
) -> Result<(), GuardViolation> {
    let (stage, route_id) = match &configuration.objective_state {
        ObjectiveState::Navigating {
            navigation: NavState::Ready { stage, route },
            ..
        } => (stage, route),
        _ => {
            return Err(GuardViolation::new(
                "wrong_state",
                "StartAttempt is enabled only from Ready",
            ));
        }
    };
    validate_attempt_id(&input.attempt.id)?;
    ensure(
        &input.attempt.route == route_id,
        "attempt_route_mismatch",
        || "Attempt must use the selected Route".into(),
    )?;
    ensure(
        route_status(configuration, route_id)? == RouteStatus::Available,
        "route_rejected",
        || "cannot start an Attempt on a rejected Route".into(),
    )?;
    ensure(
        input.attempt.context == acceptance_context(configuration, stage)?,
        "attempt_context",
        || "Attempt context must equal the current AcceptanceContext".into(),
    )?;
    ensure_fresh(
        configuration,
        FirstClassObject::Attempt(input.attempt.clone()),
    )?;
    let maximum = configuration
        .objects
        .values()
        .filter_map(|value| match value {
            FirstClassObject::Attempt(existing) if &existing.route == route_id => {
                Some(existing.ordinal)
            }
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let expected = maximum.checked_add(1).ok_or_else(|| {
        GuardViolation::new(
            "attempt_ordinal_exhausted",
            "Attempt ordinal space is exhausted for the selected Route",
        )
    })?;
    ensure(input.attempt.ordinal == expected, "attempt_ordinal", || {
        format!(
            "Attempt ordinal must be {}, got {}",
            expected, input.attempt.ordinal
        )
    })
}

fn validate_record_evidence(
    configuration: &DomainConfiguration,
    input: &RecordEvidenceInput,
) -> Result<(), GuardViolation> {
    let (stage, current_attempt) = match &configuration.objective_state {
        ObjectiveState::Navigating {
            navigation: NavState::Attempting { stage, attempt, .. },
            ..
        } => (stage, attempt),
        _ => {
            return Err(GuardViolation::new(
                "wrong_state",
                "RecordEvidence is enabled only while Attempting",
            ));
        }
    };
    ensure(
        attempt_state(configuration, current_attempt)? == AttemptState::Running,
        "attempt_not_running",
        || "Evidence can be recorded only for a running Attempt".into(),
    )?;
    validate_stage_evidence_admission(configuration, &input.evidence, stage, current_attempt)
}

fn validate_seal_attempt(
    configuration: &DomainConfiguration,
    input: &SealAttemptInput,
) -> Result<(), GuardViolation> {
    let (stage, current_attempt) = match &configuration.objective_state {
        ObjectiveState::Navigating {
            navigation: NavState::Attempting { stage, attempt, .. },
            ..
        } => (stage, attempt),
        _ => {
            return Err(GuardViolation::new(
                "wrong_state",
                "SealAttempt is enabled only while Attempting",
            ));
        }
    };
    ensure(
        attempt_state(configuration, current_attempt)? == AttemptState::Running,
        "attempt_not_running",
        || "only a running Attempt can be sealed".into(),
    )?;
    validate_packet_id(&input.packet.id)?;
    ensure_fresh(
        configuration,
        FirstClassObject::ReviewPacket(input.packet.clone()),
    )?;
    ensure(
        &input.packet.attempt == current_attempt,
        "packet_attempt",
        || "Packet must bind the current Attempt".into(),
    )?;
    ensure(
        !configuration.objects.values().any(|value| {
            matches!(value, FirstClassObject::ReviewPacket(existing) if &existing.attempt == current_attempt)
        }),
        "attempt_already_sealed",
        || "an Attempt can have only one admitted ReviewPacket".into(),
    )?;
    ensure(&input.packet.stage == stage, "packet_stage", || {
        "Packet must bind the current Stage".into()
    })?;
    let current_attempt_value = attempt(configuration, current_attempt)?;
    ensure(
        input.packet.context == current_attempt_value.context
            && input.packet.context == acceptance_context(configuration, stage)?,
        "packet_context",
        || "Packet context must equal the current Attempt and AcceptanceContext".into(),
    )?;
    ensure(
        input.packet.termination == input.seal_reason,
        "packet_termination",
        || "Packet termination must equal SealAttempt seal_reason".into(),
    )?;
    ensure(
        !input.packet.evidence_set.is_empty(),
        "packet_evidence_empty",
        || "Packet Evidence set must not be empty".into(),
    )?;
    let universe = evidence_universe(configuration, stage, &input.packet.context)?;
    ensure(!universe.is_empty(), "packet_evidence_empty", || {
        "Packet Evidence universe must not be empty".into()
    })?;
    ensure(
        input.packet.evidence_set == universe,
        "packet_evidence_universe",
        || "Packet must contain the exact complete Evidence universe".into(),
    )?;
    ensure(
        input.packet.evidence_set.iter().any(|id| {
            evidence(configuration, id).is_ok_and(|value| {
                value.subject == EvidenceSubject::Attempt(current_attempt.clone())
            })
        }),
        "packet_current_attempt_evidence",
        || "Packet must contain at least one Evidence from the current Attempt".into(),
    )?;
    let _ = dependency_view(configuration, &input.packet)?;
    Ok(())
}

fn validate_decision_transition(
    configuration: &DomainConfiguration,
    input: &DecisionInput,
) -> Result<(), GuardViolation> {
    let (stage, current_attempt, current_packet) = match &configuration.objective_state {
        ObjectiveState::Navigating {
            navigation:
                NavState::Reviewing {
                    stage,
                    attempt,
                    packet,
                    ..
                },
            ..
        } => (stage, attempt, packet),
        _ => {
            return Err(GuardViolation::new(
                "wrong_state",
                "Decision is enabled only while Reviewing",
            ));
        }
    };
    ensure(
        attempt_state(configuration, current_attempt)? == AttemptState::Sealed,
        "attempt_not_sealed",
        || "ReviewDecision requires a sealed Attempt".into(),
    )?;
    validate_decision_id(&input.decision.id)?;
    ensure_fresh(
        configuration,
        FirstClassObject::ReviewDecision(input.decision.clone()),
    )?;
    ensure(
        &input.decision.packet == current_packet,
        "decision_packet",
        || "ReviewDecision must bind the current Packet".into(),
    )?;
    let current_packet_value = packet(configuration, current_packet)?;
    ensure(&current_packet_value.stage == stage, "packet_stage", || {
        "current Packet does not bind the current Stage".into()
    })?;
    ensure(
        current_packet_value.context == acceptance_context(configuration, stage)?,
        "stale_context",
        || "current Packet context is stale".into(),
    )?;
    let map = selected_map(configuration)?.ok_or_else(|| {
        GuardViolation::new("map_missing", "ReviewDecision requires a current Map")
    })?;
    let criteria = criteria_for_stage(map, stage)?;
    let domain: BTreeSet<_> = input.decision.judgments.keys().cloned().collect();
    ensure(domain == *criteria, "decision_criteria_domain", || {
        "ReviewDecision must judge exactly every current Stage Criterion".into()
    })?;
    if matches!(&input.decision.action, ReviewAction::Accept) {
        ensure(
            input
                .decision
                .judgments
                .values()
                .all(|judgment| *judgment == CriterionJudgment::Satisfied),
            "accept_not_satisfied",
            || "accept requires every Criterion judgment to be satisfied".into(),
        )?;
    }
    match &input.decision.action {
        ReviewAction::Wait(wait) => {
            validate_wait_condition_shape(wait)?;
            ensure(&wait.stage == stage, "wait_stage", || {
                "WaitCondition must bind the current Stage".into()
            })?;
            ensure(
                wait.context == current_packet_value.context,
                "wait_context",
                || "WaitCondition must bind the current AcceptanceContext".into(),
            )?;
            ensure_fresh(
                configuration,
                FirstClassObject::WaitCondition((**wait).clone()),
            )?;
        }
        ReviewAction::Remap { reason } => {
            non_empty(reason, "remap_reason_empty", "remap reason")?;
        }
        ReviewAction::Accept | ReviewAction::Retry | ReviewAction::Replace => {}
    }
    Ok(())
}

fn validate_check_wait(
    configuration: &DomainConfiguration,
    input: &CheckWaitInput,
) -> Result<(), GuardViolation> {
    let (stage, route_id, current_wait_id) = match &configuration.objective_state {
        ObjectiveState::Navigating {
            navigation:
                NavState::Waiting {
                    stage,
                    route,
                    wait_condition,
                },
            ..
        } => (stage, route, wait_condition),
        _ => {
            return Err(GuardViolation::new(
                "wrong_state",
                "CheckWait is enabled only while Waiting",
            ));
        }
    };
    ensure(
        &input.wait_condition == current_wait_id,
        "wait_identity",
        || "CheckWait must bind the current WaitCondition".into(),
    )?;
    ensure(
        route_status(configuration, route_id)? == RouteStatus::Available,
        "route_rejected",
        || "current Waiting Route is already rejected".into(),
    )?;
    let current_wait = wait_condition(configuration, current_wait_id)?;
    ensure(&current_wait.stage == stage, "wait_stage", || {
        "current WaitCondition does not bind the current Stage".into()
    })?;
    ensure(!input.evidence.is_empty(), "wait_evidence_empty", || {
        "CheckWait requires a non-empty batch of new Evidence".into()
    })?;
    for (id, candidate) in &input.evidence {
        ensure(id == &candidate.id, "identity_key_mismatch", || {
            format!(
                "Evidence map key {id:?} does not match value identity {:?}",
                candidate.id
            )
        })?;
        validate_wait_evidence_admission(configuration, candidate, current_wait)?;
    }
    ensure(
        input.judgment.wait_condition == *current_wait_id,
        "wait_judgment_identity",
        || "WaitJudgment must bind the current WaitCondition".into(),
    )?;
    let mut expected = wait_evidence_universe(configuration, current_wait)?;
    expected.extend(input.evidence.keys().cloned());
    ensure(
        input.judgment.evidence_set == expected,
        "wait_judgment_evidence",
        || "WaitJudgment must contain exactly cumulative prior plus new wait Evidence".into(),
    )
}

fn validate_request_remap(
    configuration: &DomainConfiguration,
    input: &RequestRemapInput,
) -> Result<(), GuardViolation> {
    ensure(
        matches!(
            configuration.objective_state,
            ObjectiveState::Navigating { .. }
        ),
        "wrong_state",
        || "RequestRemap is enabled only from a Navigating state".into(),
    )?;
    non_empty(&input.reason, "remap_reason_empty", "remap reason")
}

fn validate_revise_objective(
    configuration: &DomainConfiguration,
    input: &ReviseObjectiveInput,
) -> Result<(), GuardViolation> {
    let objective_id = active_objective(configuration)?;
    validate_objective_spec_shape(&input.objective_spec)?;
    ensure(
        &input.objective_spec.objective == objective_id,
        "objective_mismatch",
        || "ObjectiveSpec revision must preserve the stable Objective identity".into(),
    )?;
    validate_confirmation(
        &input.confirmation,
        ObjectiveConfirmationAction::Revise,
        &input.objective_spec,
    )?;
    ensure_fresh(
        configuration,
        FirstClassObject::ObjectiveSpec(input.objective_spec.clone()),
    )?;
    for criterion in input.objective_spec.criteria.values() {
        ensure_compatible_or_new(
            configuration,
            FirstClassObject::Criterion(criterion.clone()),
        )?;
    }
    Ok(())
}

fn validate_abandon(
    configuration: &DomainConfiguration,
    input: &AbandonInput,
) -> Result<(), GuardViolation> {
    let objective_id = active_objective(configuration)?;
    non_empty(&input.reason, "abandon_reason_empty", "abandon reason")?;
    non_empty(
        input.confirmation.project.as_str(),
        "confirmation_project_empty",
        "confirmation project identity",
    )?;
    ensure(
        input.confirmation.confirmed,
        "human_confirmation_missing",
        || "Abandon requires explicit human confirmation".into(),
    )?;
    ensure(
        &input.confirmation.objective == objective_id,
        "confirmation_identity_mismatch",
        || "Abandon confirmation must bind the active Objective".into(),
    )?;
    ensure(
        input.confirmation.reason == input.reason,
        "confirmation_payload_mismatch",
        || "Abandon confirmation must bind the exact reason payload".into(),
    )
}

/// Validate one of the twelve model transition inputs against its exact admission pre-state.
pub fn validate_transition(
    configuration: &DomainConfiguration,
    input: &TransitionInput,
) -> Result<(), GuardViolation> {
    if matches!(
        configuration.objective_state,
        ObjectiveState::Achieved { .. } | ObjectiveState::Abandoned { .. }
    ) {
        return Err(GuardViolation::new(
            "terminal_state",
            "terminal Objective rejects every subsequent business transition",
        ));
    }
    match input {
        TransitionInput::ActivateObjective(input) => validate_activate(configuration, input),
        TransitionInput::InstallMap(input) => validate_install_map(configuration, input),
        TransitionInput::AddRoute(input) => validate_add_route(configuration, input),
        TransitionInput::SelectRoute(input) => validate_select_route(configuration, input),
        TransitionInput::StartAttempt(input) => validate_start_attempt(configuration, input),
        TransitionInput::RecordEvidence(input) => validate_record_evidence(configuration, input),
        TransitionInput::SealAttempt(input) => validate_seal_attempt(configuration, input),
        TransitionInput::Decision(input) => validate_decision_transition(configuration, input),
        TransitionInput::CheckWait(input) => validate_check_wait(configuration, input),
        TransitionInput::RequestRemap(input) => validate_request_remap(configuration, input),
        TransitionInput::ReviseObjective(input) => validate_revise_objective(configuration, input),
        TransitionInput::Abandon(input) => validate_abandon(configuration, input),
    }
}

fn audit_object_key_integrity(
    configuration: &DomainConfiguration,
    violations: &mut Vec<InvariantViolation>,
) {
    for (key, value) in configuration.objects.iter() {
        if key != &value.identity() {
            violations.push(InvariantViolation::new(
                "identity",
                format!(
                    "knowledge key {key:?} does not equal value identity {:?}",
                    value.identity()
                ),
            ));
        }
    }
}

fn audit_embedded_objects(
    configuration: &DomainConfiguration,
    violations: &mut Vec<InvariantViolation>,
) {
    for value in configuration.objects.values() {
        let result = match value {
            FirstClassObject::Objective(objective) => validate_objective_id(&objective.id),
            FirstClassObject::ObjectiveSpec(specification) => {
                validate_objective_spec_shape(specification).and_then(|()| {
                    for criterion in specification.criteria.values() {
                        let stored = object(
                            configuration,
                            &ObjectIdentity::Criterion(criterion.id.clone()),
                        )?;
                        ensure(
                            stored == &FirstClassObject::Criterion(criterion.clone()),
                            "identity_conflict",
                            || format!("ObjectiveSpec embeds conflicting Criterion {:?}", criterion.id),
                        )?;
                    }
                    Ok(())
                })
            }
            FirstClassObject::MapRevision(map) => objective_spec(configuration, &map.objective_spec)
                .and_then(|specification| validate_map_shape(map, specification))
                .and_then(|()| {
                    for stage in map.stages.values() {
                        ensure(
                            object(configuration, &ObjectIdentity::Stage(stage.id.clone()))?
                                == &FirstClassObject::Stage(stage.clone()),
                            "identity_conflict",
                            || format!("Map embeds conflicting Stage {:?}", stage.id),
                        )?;
                    }
                    for criterion in map.criteria.values() {
                        ensure(
                            object(
                                configuration,
                                &ObjectIdentity::Criterion(criterion.id.clone()),
                            )? == &FirstClassObject::Criterion(criterion.clone()),
                            "identity_conflict",
                            || format!("Map embeds conflicting Criterion {:?}", criterion.id),
                        )?;
                    }
                    Ok(())
                }),
            FirstClassObject::Stage(stage) => validate_stage_id(&stage.id),
            FirstClassObject::Criterion(criterion) => validate_criterion_id(&criterion.id),
            FirstClassObject::Route(candidate) => validate_route_id(&candidate.id),
            FirstClassObject::Attempt(candidate) => validate_attempt_id(&candidate.id)
                .and_then(|()| route(configuration, &candidate.route).map(|_| ()))
                .and_then(|()| {
                    let duplicates = configuration
                        .objects
                        .values()
                        .filter(|value| {
                            matches!(value, FirstClassObject::Attempt(other)
                                if other.route == candidate.route && other.ordinal == candidate.ordinal)
                        })
                        .count();
                    ensure(duplicates == 1, "attempt_ordinal_conflict", || {
                        format!(
                            "Route {:?} has more than one Attempt with ordinal {}",
                            candidate.route, candidate.ordinal
                        )
                    })
                }),
            FirstClassObject::Evidence(candidate) => {
                validate_evidence_id(&candidate.id)
                    .and_then(|()| validate_frozen_observation(&candidate.observation))
                    .and_then(|()| match &candidate.subject {
                        EvidenceSubject::Attempt(id) => attempt(configuration, id).map(|_| ()),
                        EvidenceSubject::WaitCondition(id) => {
                            wait_condition(configuration, id).map(|_| ())
                        }
                    })
            }
            FirstClassObject::ReviewPacket(candidate) => validate_packet_id(&candidate.id)
                .and_then(|()| {
                    let duplicates = configuration
                        .objects
                        .values()
                        .filter(|value| {
                            matches!(value, FirstClassObject::ReviewPacket(other)
                                if other.attempt == candidate.attempt)
                        })
                        .count();
                    ensure(duplicates == 1, "attempt_multiple_packets", || {
                        format!("Attempt {:?} has more than one ReviewPacket", candidate.attempt)
                    })
                })
                .and_then(|()| attempt(configuration, &candidate.attempt))
                .and_then(|attempt_value| {
                    let route_value = route(configuration, &attempt_value.route)?;
                    ensure(route_value.stage == candidate.stage, "packet_stage", || {
                        "Packet Stage does not match its Attempt Route".into()
                    })?;
                    ensure(attempt_value.context == candidate.context, "packet_context", || {
                        "Packet context does not match its Attempt".into()
                    })?;
                    ensure(!candidate.evidence_set.is_empty(), "packet_evidence_empty", || {
                        "Packet Evidence set is empty".into()
                    })?;
                    let mut has_current = false;
                    for id in &candidate.evidence_set {
                        let packet_evidence = evidence(configuration, id)?;
                        ensure(
                            packet_evidence.purpose == EvidencePurpose::StageReview
                                && packet_evidence.context == candidate.context,
                            "packet_evidence_context",
                            || "Packet contains Evidence outside its Stage review Context".into(),
                        )?;
                        if packet_evidence.subject
                            == EvidenceSubject::Attempt(candidate.attempt.clone())
                        {
                            has_current = true;
                        }
                    }
                    ensure(has_current, "packet_current_attempt_evidence", || {
                        "Packet contains no Evidence from its own Attempt".into()
                    })
                }),
            FirstClassObject::ReviewDecision(candidate) => packet(configuration, &candidate.packet)
                .and_then(|review_packet| {
                    let expected = &review_packet.context.structural.contract.criteria;
                    let actual: BTreeSet<_> = candidate.judgments.keys().cloned().collect();
                    ensure(actual == *expected, "decision_criteria_domain", || {
                        "Decision judgment domain differs from Packet Stage contract".into()
                    })?;
                    if matches!(&candidate.action, ReviewAction::Accept) {
                        ensure(
                            candidate
                                .judgments
                                .values()
                                .all(|value| *value == CriterionJudgment::Satisfied),
                            "accept_not_satisfied",
                            || "accepted Decision has a non-satisfied Criterion".into(),
                        )?;
                    }
                    if let ReviewAction::Wait(wait) = &candidate.action {
                        validate_wait_condition_shape(wait)?;
                        ensure(
                            wait.stage == review_packet.stage && wait.context == review_packet.context,
                            "wait_context",
                            || "Decision WaitCondition does not match its Packet".into(),
                        )?;
                    }
                    Ok(())
                }),
            FirstClassObject::WaitCondition(wait) => validate_wait_condition_shape(wait),
        };
        if let Err(error) = result {
            violations.push(InvariantViolation::new(
                "object_integrity",
                error.to_string(),
            ));
        }
    }
}

fn audit_lifecycle(configuration: &DomainConfiguration, violations: &mut Vec<InvariantViolation>) {
    let routes: BTreeSet<_> = configuration
        .objects
        .values()
        .filter_map(|value| match value {
            FirstClassObject::Route(route) => Some(route.id.clone()),
            _ => None,
        })
        .collect();
    let status_routes: BTreeSet<_> = configuration
        .lifecycle
        .route_status
        .keys()
        .cloned()
        .collect();
    if routes != status_routes {
        violations.push(InvariantViolation::new(
            "I19",
            "Route lifecycle projection domain does not equal admitted Route identities",
        ));
    }

    let attempts: BTreeSet<_> = configuration
        .objects
        .values()
        .filter_map(|value| match value {
            FirstClassObject::Attempt(attempt) => Some(attempt.identity()),
            _ => None,
        })
        .collect();
    let lifecycle_attempts: BTreeSet<_> = configuration
        .lifecycle
        .attempt_state
        .keys()
        .cloned()
        .collect();
    if attempts != lifecycle_attempts {
        violations.push(InvariantViolation::new(
            "I2",
            "Attempt lifecycle projection domain does not equal admitted Attempt identities",
        ));
    }

    for value in configuration.objects.values() {
        let FirstClassObject::ReviewDecision(review) = value else {
            continue;
        };
        if !matches!(&review.action, ReviewAction::Replace) {
            continue;
        }
        let result = packet(configuration, &review.packet)
            .and_then(|packet| attempt(configuration, &packet.attempt))
            .map(|attempt| attempt.route.clone())
            .and_then(|route| {
                ensure(
                    configuration.lifecycle.route_status.get(&route)
                        == Some(&RouteStatus::Rejected),
                    "route_rejection_projection",
                    || format!("Decision(replace) did not reject Route {route:?}"),
                )
            });
        if let Err(error) = result {
            violations.push(InvariantViolation::new("I19", error.to_string()));
        }
    }

    for proof in &configuration.lifecycle.invalidated_proofs {
        match decision(configuration, proof) {
            Ok(value) if matches!(&value.action, ReviewAction::Accept) => {}
            Ok(_) => violations.push(InvariantViolation::new(
                "I10",
                format!("invalidated proof {proof:?} is not an accept Decision"),
            )),
            Err(error) => violations.push(InvariantViolation::new("I10", error.to_string())),
        }
    }
}

fn audit_navigation(configuration: &DomainConfiguration, violations: &mut Vec<InvariantViolation>) {
    let result = match &configuration.objective_state {
        ObjectiveState::Idle => ensure(
            configuration.objects.is_empty()
                && configuration.lifecycle == LifecycleProjection::default(),
            "idle_not_initial",
            || "Idle configuration must equal the empty initial configuration".into(),
        ),
        ObjectiveState::Mapping {
            objective: objective_id,
            objective_spec: spec,
            previous_map,
            ..
        } => objective(configuration, objective_id)
            .and_then(|_| objective_spec(configuration, spec))
            .and_then(|value| {
                ensure(
                    &value.objective == objective_id,
                    "objective_mismatch",
                    || "Mapping state Objective and ObjectiveSpec disagree".into(),
                )
            })
            .and_then(|()| {
                previous_map
                    .as_ref()
                    .map(|id| map_revision(configuration, id).map(|_| ()))
                    .unwrap_or(Ok(()))
            }),
        ObjectiveState::Navigating {
            objective: objective_id,
            map: map_id,
            navigation,
        } => objective(configuration, objective_id)
            .and_then(|_| map_revision(configuration, map_id))
            .and_then(|map| {
                ensure(
                    &map.objective_spec.objective == objective_id,
                    "objective_mismatch",
                    || "Navigating Map belongs to a different Objective".into(),
                )?;
                let (stage, route_id, attempt_id, packet_id, wait_id) = match navigation {
                    NavState::SeekingRoute { stage } => (stage, None, None, None, None),
                    NavState::Ready { stage, route } => (stage, Some(route), None, None, None),
                    NavState::Attempting {
                        stage,
                        route,
                        attempt,
                    } => (stage, Some(route), Some(attempt), None, None),
                    NavState::Reviewing {
                        stage,
                        route,
                        attempt,
                        packet,
                    } => (stage, Some(route), Some(attempt), Some(packet), None),
                    NavState::Waiting {
                        stage,
                        route,
                        wait_condition,
                    } => (stage, Some(route), None, None, Some(wait_condition)),
                };
                ensure(
                    map.stages.contains_key(stage),
                    "current_stage_missing",
                    || "current Stage is absent from the current Map".into(),
                )?;
                let proofs = proofs_for_map(configuration, map)?;
                for dependency in direct_dependencies(map, stage) {
                    ensure(
                        proofs.contains_key(&dependency),
                        "dependency_proof_missing",
                        || format!("current Stage dependency {dependency:?} lacks a current proof"),
                    )?;
                }
                if let Some(route_id) = route_id {
                    let current_route = route(configuration, route_id)?;
                    ensure(
                        &current_route.stage == stage,
                        "route_stage_mismatch",
                        || "current Route targets a different Stage".into(),
                    )?;
                    ensure(
                        current_route.structural_context == structural_context(map, stage)?,
                        "route_structural_context",
                        || "current Route StructuralContext is stale".into(),
                    )?;
                    ensure(
                        route_status(configuration, route_id)? == RouteStatus::Available,
                        "route_rejected",
                        || "current Route is rejected".into(),
                    )?;
                }
                if let Some(attempt_id) = attempt_id {
                    let current_attempt = attempt(configuration, attempt_id)?;
                    ensure(
                        Some(&current_attempt.route) == route_id,
                        "attempt_route_mismatch",
                        || "current Attempt does not use the current Route".into(),
                    )?;
                    ensure(
                        current_attempt.context
                            == acceptance_context_with_proofs(map, stage, &proofs)?,
                        "attempt_context",
                        || "current Attempt context is stale".into(),
                    )?;
                    let expected_state = if packet_id.is_some() {
                        AttemptState::Sealed
                    } else {
                        AttemptState::Running
                    };
                    ensure(
                        attempt_state(configuration, attempt_id)? == expected_state,
                        "attempt_state",
                        || "current Attempt state does not match NavState".into(),
                    )?;
                }
                if let Some(packet_id) = packet_id {
                    let current_packet = packet(configuration, packet_id)?;
                    ensure(
                        Some(&current_packet.attempt) == attempt_id,
                        "packet_attempt",
                        || "current Packet does not bind the current Attempt".into(),
                    )?;
                    ensure(&current_packet.stage == stage, "packet_stage", || {
                        "current Packet does not bind the current Stage".into()
                    })?;
                }
                if let Some(wait_id) = wait_id {
                    let current_wait = wait_condition(configuration, wait_id)?;
                    ensure(
                        &current_wait.stage == stage
                            && current_wait.context
                                == acceptance_context_with_proofs(map, stage, &proofs)?,
                        "wait_context",
                        || "current WaitCondition does not bind current Stage Context".into(),
                    )?;
                }
                ensure(!complete(configuration)?, "navigating_complete", || {
                    "Navigating configuration already satisfies Complete and should be Achieved"
                        .into()
                })
            }),
        ObjectiveState::Achieved {
            objective: objective_id,
            map: map_id,
            manifest,
        } => objective(configuration, objective_id)
            .and_then(|_| map_revision(configuration, map_id))
            .and_then(|map| {
                ensure(
                    &map.objective_spec.objective == objective_id,
                    "objective_mismatch",
                    || "Achieved Map belongs to a different Objective".into(),
                )?;
                let proofs = proofs_for_map(configuration, map)?;
                ensure(map.stages.len() == proofs.len(), "complete_false", || {
                    "Achieved Objective does not have current proofs for every Stage".into()
                })?;
                ensure(manifest == &proofs, "manifest_mismatch", || {
                    "Achieved Manifest does not equal the exact current proof set".into()
                })?;

                if let Some(final_stage) = map
                    .stages
                    .values()
                    .find(|stage| stage.kind == StageKind::FinalIntegration)
                {
                    let final_proof = proofs.get(&final_stage.id).ok_or_else(|| {
                        GuardViolation::new("final_proof_missing", "final integration has no proof")
                    })?;
                    let final_decision = decision(configuration, final_proof)?;
                    let final_packet = packet(configuration, &final_decision.packet)?;
                    let actual = dependency_view(configuration, final_packet)?;
                    let expected: BTreeSet<_> = transitive_dependencies(map, &final_stage.id)?
                        .into_iter()
                        .filter_map(|stage| proofs.get(&stage).cloned())
                        .collect();
                    ensure(actual == expected, "final_dependency_view", || {
                        "final integration DependencyView does not equal the current proof tree"
                            .into()
                    })?;
                }
                Ok(())
            }),
        ObjectiveState::Abandoned { objective: id, .. } => objective(configuration, id).map(|_| ()),
    };
    if let Err(error) = result {
        let invariant = match error.code {
            "map_cycle" => "I3",
            "owner_not_total" | "owner_stage_missing" => "I4",
            "dependency_proof_missing" => "I5",
            "final_dependency_view" => "I11",
            "complete_false" | "navigating_complete" | "manifest_mismatch" => "I12",
            "wait_context" => "I13",
            _ => "I1-I13",
        };
        violations.push(InvariantViolation::new(invariant, error.to_string()));
    }
}

/// Audit every invariant property that is observable from the current typed configuration.
///
/// An `Ok(())` result is not a Trail/replay certificate; see the module-level historical scope
/// statement.  Reducer/application replay audit must additionally prove I6, I15, I17, and the
/// wait-origin half of I19 against immutable transition facts.
pub fn audit_invariants(
    configuration: &DomainConfiguration,
) -> Result<(), Vec<InvariantViolation>> {
    let mut violations = Vec::new();
    audit_object_key_integrity(configuration, &mut violations);
    audit_embedded_objects(configuration, &mut violations);
    audit_lifecycle(configuration, &mut violations);
    audit_navigation(configuration, &mut violations);

    if violations.is_empty() {
        Ok(())
    } else {
        violations.sort();
        violations.dedup();
        Err(violations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn criterion(scope: CriterionScope) -> Criterion {
        Criterion {
            id: CriterionId::new("criterion-1"),
            statement: "observable result holds".into(),
            verification_rule: "inspect frozen observation".into(),
            scope,
        }
    }

    fn specification(scope: CriterionScope) -> ObjectiveSpec {
        let criterion = criterion(scope);
        ObjectiveSpec {
            objective: ObjectiveId::new("objective-1"),
            revision: 1,
            intended_outcome: "verified result".into(),
            criteria: BTreeMap::from([(criterion.id.clone(), criterion)]),
            boundaries: BTreeSet::from(["local-only".into()]),
            excluded_claims: BTreeSet::from(["unverified completion".into()]),
        }
    }

    fn stage() -> Stage {
        Stage {
            id: StageId::new("stage-1"),
            name: "stage one".into(),
            outcome: "observable result holds".into(),
            output: "verified output".into(),
            kind: StageKind::Ordinary,
        }
    }

    fn stage_contract() -> StageContract {
        StageContract {
            outcome: "observable result holds".into(),
            criteria: BTreeSet::from([CriterionId::new("criterion-1")]),
            objective_boundaries: BTreeSet::from(["local-only".into()]),
            output: "verified output".into(),
        }
    }

    fn map_for(specification: &ObjectiveSpec) -> MapRevision {
        let stage = stage();
        let criterion = specification
            .criteria
            .get(&CriterionId::new("criterion-1"))
            .expect("fixture criterion")
            .clone();
        MapRevision {
            objective_spec: specification.identity(),
            revision: 1,
            stages: BTreeMap::from([(stage.id.clone(), stage)]),
            criteria: BTreeMap::from([(criterion.id.clone(), criterion)]),
            dependencies: BTreeSet::new(),
            priorities: BTreeMap::from([(StageId::new("stage-1"), 1)]),
            owners: BTreeMap::from([(CriterionId::new("criterion-1"), StageId::new("stage-1"))]),
            contracts: BTreeMap::from([(StageId::new("stage-1"), stage_contract())]),
        }
    }

    fn confirmation(
        action: ObjectiveConfirmationAction,
        specification: &ObjectiveSpec,
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

    fn insert(configuration: &mut DomainConfiguration, value: FirstClassObject) {
        configuration
            .objects
            .insert_checked(value)
            .expect("fixture insertion must preserve identity uniqueness");
    }

    fn mapping_configuration(specification: &ObjectiveSpec) -> DomainConfiguration {
        let mut configuration = DomainConfiguration {
            objective_state: ObjectiveState::Mapping {
                objective: specification.objective.clone(),
                objective_spec: specification.identity(),
                previous_map: None,
                reason: Some(MappingReason::Initial),
            },
            objects: ObjectKnowledge::new(),
            lifecycle: LifecycleProjection::default(),
        };
        insert(
            &mut configuration,
            FirstClassObject::Objective(Objective {
                id: specification.objective.clone(),
            }),
        );
        insert(
            &mut configuration,
            FirstClassObject::ObjectiveSpec(specification.clone()),
        );
        for criterion in specification.criteria.values() {
            insert(
                &mut configuration,
                FirstClassObject::Criterion(criterion.clone()),
            );
        }
        configuration
    }

    fn route_for(map: &MapRevision) -> Route {
        Route {
            id: RouteId::new("route-1"),
            stage: StageId::new("stage-1"),
            structural_context: structural_context(map, &StageId::new("stage-1"))
                .expect("fixture context"),
            hypothesis: "smallest falsifiable route".into(),
            assumptions: BTreeSet::new(),
            rationale: "direct observation".into(),
        }
    }

    fn navigating_configuration(navigation: NavState) -> DomainConfiguration {
        let specification = specification(CriterionScope::Local);
        let map = map_for(&specification);
        let route = route_for(&map);
        let mut configuration = mapping_configuration(&specification);
        insert(
            &mut configuration,
            FirstClassObject::MapRevision(map.clone()),
        );
        for stage in map.stages.values() {
            insert(&mut configuration, FirstClassObject::Stage(stage.clone()));
        }
        insert(&mut configuration, FirstClassObject::Route(route.clone()));
        configuration
            .lifecycle
            .route_status
            .insert(route.id.clone(), RouteStatus::Available);
        configuration.objective_state = ObjectiveState::Navigating {
            objective: specification.objective,
            map: map.identity(),
            navigation,
        };
        configuration
    }

    fn attempt_value(configuration: &DomainConfiguration) -> Attempt {
        Attempt {
            id: AttemptId::new("attempt-1"),
            route: RouteId::new("route-1"),
            ordinal: 1,
            bound: AttemptBound::TerminationCondition("observation recorded".into()),
            context: acceptance_context(configuration, &StageId::new("stage-1"))
                .expect("fixture acceptance context"),
        }
    }

    fn attempting_configuration() -> (DomainConfiguration, Attempt) {
        let mut configuration = navigating_configuration(NavState::Ready {
            stage: StageId::new("stage-1"),
            route: RouteId::new("route-1"),
        });
        let attempt = attempt_value(&configuration);
        insert(
            &mut configuration,
            FirstClassObject::Attempt(attempt.clone()),
        );
        configuration
            .lifecycle
            .attempt_state
            .insert(attempt.id.clone(), AttemptState::Running);
        configuration.objective_state = ObjectiveState::Navigating {
            objective: ObjectiveId::new("objective-1"),
            map: MapRevisionId {
                objective: ObjectiveId::new("objective-1"),
                revision: 1,
            },
            navigation: NavState::Attempting {
                stage: StageId::new("stage-1"),
                route: RouteId::new("route-1"),
                attempt: attempt.id.clone(),
            },
        };
        (configuration, attempt)
    }

    fn evidence_value(attempt: &Attempt) -> Evidence {
        Evidence {
            id: EvidenceId::new("evidence-1"),
            subject: EvidenceSubject::Attempt(attempt.id.clone()),
            context: attempt.context.clone(),
            purpose: EvidencePurpose::StageReview,
            claims: BTreeMap::from([(CriterionId::new("criterion-1"), EvidenceClaim::Supports)]),
            observation: FrozenObservation::Inline(CanonicalValue::String("observed".into())),
            provenance: CanonicalValue::String("test fixture".into()),
        }
    }

    fn packet_value(attempt: &Attempt, evidence: &Evidence) -> ReviewPacket {
        ReviewPacket {
            id: ReviewPacketId::new("packet-1"),
            attempt: attempt.id.clone(),
            stage: StageId::new("stage-1"),
            context: attempt.context.clone(),
            termination: SealReason::Submitted,
            evidence_set: BTreeSet::from([evidence.id.clone()]),
        }
    }

    #[test]
    fn activation_confirmation_is_exact_and_project_bound() {
        let specification = specification(CriterionScope::Local);
        let valid = ActivateObjectiveInput {
            objective_spec: specification.clone(),
            confirmation: confirmation(ObjectiveConfirmationAction::Activate, &specification),
        };
        let idle = DomainConfiguration {
            objective_state: ObjectiveState::Idle,
            objects: ObjectKnowledge::new(),
            lifecycle: LifecycleProjection::default(),
        };
        assert!(validate_activate(&idle, &valid).is_ok());

        let mut cases = Vec::new();
        let mut missing_confirmation = valid.clone();
        missing_confirmation.confirmation.confirmed = false;
        cases.push(("human_confirmation_missing", missing_confirmation));
        let mut wrong_action = valid.clone();
        wrong_action.confirmation.action = ObjectiveConfirmationAction::Revise;
        cases.push(("confirmation_action_mismatch", wrong_action));
        let mut empty_project = valid.clone();
        empty_project.confirmation.project = ProjectId::new("");
        cases.push(("confirmation_project_empty", empty_project));
        let mut changed_payload = valid.clone();
        changed_payload
            .confirmation
            .confirmed_payload
            .intended_outcome = "different payload".into();
        cases.push(("confirmation_payload_mismatch", changed_payload));

        for (expected, input) in cases {
            assert_eq!(
                validate_activate(&idle, &input)
                    .expect_err("negative confirmation case")
                    .code,
                expected
            );
        }
    }

    #[test]
    fn map_constraints_fail_closed_table() {
        let local_spec = specification(CriterionScope::Local);
        let valid = map_for(&local_spec);
        assert!(validate_map_shape(&valid, &local_spec).is_ok());

        let mut cases = Vec::new();
        let mut empty = valid.clone();
        empty.stages.clear();
        cases.push(("map_stages_empty", empty));
        let mut priority = valid.clone();
        priority.priorities.clear();
        cases.push(("priority_not_total", priority));
        let mut owner = valid.clone();
        owner.owners.clear();
        cases.push(("owner_not_total", owner));
        let mut contract = valid.clone();
        contract
            .contracts
            .get_mut(&StageId::new("stage-1"))
            .expect("fixture contract")
            .outcome = "different".into();
        cases.push(("contract_outcome_mismatch", contract));
        let mut cycle = valid.clone();
        cycle.dependencies.insert(StageDependency {
            dependency: StageId::new("stage-1"),
            dependent: StageId::new("stage-1"),
        });
        cases.push(("map_cycle", cycle));
        let mut boundary = valid.clone();
        boundary
            .contracts
            .get_mut(&StageId::new("stage-1"))
            .expect("fixture contract")
            .objective_boundaries
            .insert("not-in-objective".into());
        cases.push(("contract_boundary_unknown", boundary));

        for (expected, map) in cases {
            assert_eq!(
                validate_map_shape(&map, &local_spec)
                    .expect_err("negative Map case")
                    .code,
                expected
            );
        }

        let cross_spec = specification(CriterionScope::CrossStage);
        let cross_map = map_for(&cross_spec);
        assert_eq!(
            validate_map_shape(&cross_map, &cross_spec)
                .expect_err("cross-stage Criterion needs final integration")
                .code,
            "final_integration_missing"
        );
    }

    #[test]
    fn evidence_packet_and_accept_guards_are_exact() {
        let (mut configuration, attempt) = attempting_configuration();
        let valid_evidence = evidence_value(&attempt);
        assert!(
            validate_record_evidence(
                &configuration,
                &RecordEvidenceInput {
                    evidence: valid_evidence.clone(),
                },
            )
            .is_ok()
        );

        let mut wrong_purpose = valid_evidence.clone();
        wrong_purpose.purpose = EvidencePurpose::WaitResolution;
        assert_eq!(
            validate_record_evidence(
                &configuration,
                &RecordEvidenceInput {
                    evidence: wrong_purpose,
                },
            )
            .expect_err("wrong Evidence purpose")
            .code,
            "evidence_purpose"
        );

        insert(
            &mut configuration,
            FirstClassObject::Evidence(valid_evidence.clone()),
        );
        let valid_packet = packet_value(&attempt, &valid_evidence);
        assert!(
            validate_seal_attempt(
                &configuration,
                &SealAttemptInput {
                    packet: valid_packet.clone(),
                    seal_reason: SealReason::Submitted,
                },
            )
            .is_ok()
        );

        let mut incomplete_packet = valid_packet.clone();
        incomplete_packet.evidence_set.clear();
        assert_eq!(
            validate_seal_attempt(
                &configuration,
                &SealAttemptInput {
                    packet: incomplete_packet,
                    seal_reason: SealReason::Submitted,
                },
            )
            .expect_err("Packet cannot omit Evidence")
            .code,
            "packet_evidence_empty"
        );

        insert(
            &mut configuration,
            FirstClassObject::ReviewPacket(valid_packet.clone()),
        );
        configuration
            .lifecycle
            .attempt_state
            .insert(attempt.id.clone(), AttemptState::Sealed);
        configuration.objective_state = ObjectiveState::Navigating {
            objective: ObjectiveId::new("objective-1"),
            map: MapRevisionId {
                objective: ObjectiveId::new("objective-1"),
                revision: 1,
            },
            navigation: NavState::Reviewing {
                stage: StageId::new("stage-1"),
                route: RouteId::new("route-1"),
                attempt: attempt.id.clone(),
                packet: valid_packet.id.clone(),
            },
        };
        let mut bad_accept = ReviewDecision {
            id: ReviewDecisionId::new("decision-1"),
            packet: valid_packet.id,
            judgments: BTreeMap::from([(
                CriterionId::new("criterion-1"),
                CriterionJudgment::NotSatisfied,
            )]),
            findings: BTreeSet::new(),
            action: ReviewAction::Accept,
        };
        assert_eq!(
            validate_decision_transition(
                &configuration,
                &DecisionInput {
                    decision: bad_accept.clone(),
                },
            )
            .expect_err("accept requires all satisfied")
            .code,
            "accept_not_satisfied"
        );
        bad_accept.action = ReviewAction::Retry;
        assert!(
            validate_decision_transition(
                &configuration,
                &DecisionInput {
                    decision: bad_accept,
                },
            )
            .is_ok()
        );
    }

    #[test]
    fn attempt_ordinal_exhaustion_fails_closed_without_overflow() {
        let mut configuration = navigating_configuration(NavState::Ready {
            stage: StageId::new("stage-1"),
            route: RouteId::new("route-1"),
        });
        let exhausted = Attempt {
            id: AttemptId::new("attempt-exhausted"),
            route: RouteId::new("route-1"),
            ordinal: u64::MAX,
            bound: AttemptBound::TerminationCondition("historical fixture".into()),
            context: acceptance_context(&configuration, &StageId::new("stage-1"))
                .expect("fixture context"),
        };
        insert(
            &mut configuration,
            FirstClassObject::Attempt(exhausted.clone()),
        );
        configuration
            .lifecycle
            .attempt_state
            .insert(exhausted.id, AttemptState::Closed);

        let candidate = Attempt {
            id: AttemptId::new("attempt-after-exhaustion"),
            route: RouteId::new("route-1"),
            ordinal: 0,
            bound: AttemptBound::TerminationCondition("must not wrap".into()),
            context: exhausted.context,
        };
        assert_eq!(
            validate_start_attempt(&configuration, &StartAttemptInput { attempt: candidate })
                .expect_err("ordinal exhaustion must fail closed")
                .code,
            "attempt_ordinal_exhausted"
        );
    }

    #[test]
    fn derived_queries_and_observable_audit_agree() {
        let (mut configuration, attempt) = attempting_configuration();
        let evidence = evidence_value(&attempt);
        let packet = packet_value(&attempt, &evidence);
        let decision = ReviewDecision {
            id: ReviewDecisionId::new("decision-1"),
            packet: packet.id.clone(),
            judgments: BTreeMap::from([(
                CriterionId::new("criterion-1"),
                CriterionJudgment::Satisfied,
            )]),
            findings: BTreeSet::new(),
            action: ReviewAction::Accept,
        };
        insert(&mut configuration, FirstClassObject::Evidence(evidence));
        insert(
            &mut configuration,
            FirstClassObject::ReviewPacket(packet.clone()),
        );
        insert(
            &mut configuration,
            FirstClassObject::ReviewDecision(decision.clone()),
        );
        configuration
            .lifecycle
            .attempt_state
            .insert(attempt.id, AttemptState::Closed);
        configuration.objective_state = ObjectiveState::Achieved {
            objective: ObjectiveId::new("objective-1"),
            map: MapRevisionId {
                objective: ObjectiveId::new("objective-1"),
                revision: 1,
            },
            manifest: BTreeMap::from([(StageId::new("stage-1"), decision.id.clone())]),
        };

        assert_eq!(current_proofs(&configuration).unwrap().len(), 1);
        assert!(complete(&configuration).unwrap());
        assert_eq!(next_stage(&configuration).unwrap(), None);
        assert_eq!(audit_invariants(&configuration), Ok(()));

        configuration.lifecycle.attempt_state.clear();
        let violations = audit_invariants(&configuration).expect_err("missing lifecycle must fail");
        assert!(
            violations
                .iter()
                .any(|violation| violation.invariant == "I2")
        );
    }

    #[test]
    fn terminal_states_reject_business_transitions() {
        let configuration = DomainConfiguration {
            objective_state: ObjectiveState::Abandoned {
                objective: ObjectiveId::new("objective-1"),
                reason: "human confirmed".into(),
            },
            objects: ObjectKnowledge::new(),
            lifecycle: LifecycleProjection::default(),
        };
        let transition = TransitionInput::RequestRemap(RequestRemapInput {
            reason: "should not run".into(),
        });
        assert_eq!(
            validate_transition(&configuration, &transition)
                .expect_err("terminal transition")
                .code,
            "terminal_state"
        );
    }
}
