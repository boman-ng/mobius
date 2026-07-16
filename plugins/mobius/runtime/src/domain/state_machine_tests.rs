//! Deterministic, generated state-machine coverage for the Phase 1 domain.
//!
//! These tests deliberately construct every input from the current typed configuration. Seeds
//! select a scenario and deterministic fixture variations; they never bypass a guard or patch a
//! projection. Every accepted prefix is replayed and audited before generation can continue.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ReviewBranch {
    Accept,
    Retry,
    Replace,
    Wait,
    Remap,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Coverage {
    transitions: BTreeSet<TransitionKind>,
    review_branches: BTreeSet<ReviewBranch>,
    wait_directions: BTreeSet<WaitDirection>,
    historical_invariants: BTreeSet<&'static str>,
    terminal_rejections: usize,
}

impl Coverage {
    fn observe(&mut self, input: &TransitionInput) {
        self.transitions.insert(input.kind());
        match input {
            TransitionInput::RecordEvidence(_) => {
                self.historical_invariants.insert("I6");
            }
            TransitionInput::Decision(DecisionInput { decision }) => {
                let branch = match &decision.action {
                    ReviewAction::Accept => ReviewBranch::Accept,
                    ReviewAction::Retry => ReviewBranch::Retry,
                    ReviewAction::Replace => {
                        self.historical_invariants.insert("I19");
                        ReviewBranch::Replace
                    }
                    ReviewAction::Wait(_) => ReviewBranch::Wait,
                    ReviewAction::Remap { .. } => ReviewBranch::Remap,
                };
                self.review_branches.insert(branch);
            }
            TransitionInput::CheckWait(CheckWaitInput { judgment, .. }) => {
                self.historical_invariants.insert("I6");
                if judgment.direction == WaitDirection::NewRoute {
                    self.historical_invariants.insert("I19");
                }
                self.wait_directions.insert(judgment.direction);
            }
            TransitionInput::Abandon(_) => {
                self.historical_invariants.insert("I17");
            }
            TransitionInput::ActivateObjective(_)
            | TransitionInput::InstallMap(_)
            | TransitionInput::AddRoute(_)
            | TransitionInput::SelectRoute(_)
            | TransitionInput::StartAttempt(_)
            | TransitionInput::SealAttempt(_)
            | TransitionInput::RequestRemap(_)
            | TransitionInput::ReviseObjective(_) => {}
        }
    }

    fn merge(&mut self, other: &Self) {
        self.transitions.extend(other.transitions.iter().copied());
        self.review_branches
            .extend(other.review_branches.iter().copied());
        self.wait_directions
            .extend(other.wait_directions.iter().copied());
        self.historical_invariants
            .extend(other.historical_invariants.iter().copied());
        self.terminal_rejections += other.terminal_rejections;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Accept,
    Retry,
    Replace,
    ReviewRemap,
    Wait(WaitDirection),
    RequestRemap,
    Revise,
    Abandon,
}

impl Scenario {
    const ALL: [Self; 11] = [
        Self::Accept,
        Self::Retry,
        Self::Replace,
        Self::ReviewRemap,
        Self::Wait(WaitDirection::Stay),
        Self::Wait(WaitDirection::SameRoute),
        Self::Wait(WaitDirection::NewRoute),
        Self::Wait(WaitDirection::Remap),
        Self::RequestRemap,
        Self::Revise,
        Self::Abandon,
    ];
}

#[derive(Clone, Debug)]
struct Machine {
    seed: u64,
    random_state: u64,
    serial: u64,
    objective: ObjectiveId,
    configuration: DomainConfiguration,
    trail: Vec<TrailFact>,
    coverage: Coverage,
}

impl Machine {
    fn new(seed: u64) -> Self {
        let configuration = initial_configuration();
        assert_eq!(replay(&[]).expect("empty Trail replay"), configuration);
        audit_trail(&[], &configuration).expect("empty Trail projection audit");
        audit_invariants(&configuration).expect("empty configuration invariant audit");
        Self {
            seed,
            random_state: seed ^ 0x9e37_79b9_7f4a_7c15,
            serial: 0,
            objective: ObjectiveId::new(format!("objective-{seed}")),
            configuration,
            trail: Vec::new(),
            coverage: Coverage::default(),
        }
    }

    fn choose(&mut self, modulus: u64) -> u64 {
        // A fixed LCG gives reproducible variation without a dependency, clock, or ambient input.
        self.random_state = self
            .random_state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.random_state % modulus
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.serial += 1;
        format!("{prefix}-{}-{}", self.seed, self.serial)
    }

    fn push(&mut self, input: TransitionInput) {
        let kind = input.kind();
        let expected = reduce(&self.configuration, &input).unwrap_or_else(|error| {
            panic!(
                "seed {} generated invalid {kind:?} input from state {:?}: {error}",
                self.seed,
                self.configuration.objective_state()
            )
        });
        self.coverage.observe(&input);
        self.trail.push(TrailFact {
            objective: self.objective.clone(),
            input,
        });

        // I15 plus historical I6, I17, and I19 are checked by replay/audit_trail for every prefix.
        let replayed = replay(&self.trail).unwrap_or_else(|error| {
            panic!(
                "seed {} failed replay at accepted prefix {}: {error}",
                self.seed,
                self.trail.len()
            )
        });
        assert_eq!(
            replayed,
            expected,
            "seed {} replay diverged at prefix {}",
            self.seed,
            self.trail.len()
        );
        audit_invariants(&expected).unwrap_or_else(|violations| {
            panic!(
                "seed {} violated configuration invariants at prefix {}: {violations:#?}",
                self.seed,
                self.trail.len()
            )
        });
        audit_trail(&self.trail, &expected).unwrap_or_else(|error| {
            panic!(
                "seed {} failed Trail audit at prefix {}: {error}",
                self.seed,
                self.trail.len()
            )
        });
        self.configuration = expected;
    }

    fn activate(&mut self) {
        assert!(matches!(
            self.configuration.objective_state(),
            ObjectiveState::Idle
        ));
        let specification = objective_spec(self.seed, self.objective.clone(), 1);
        self.push(TransitionInput::ActivateObjective(ActivateObjectiveInput {
            confirmation: confirmation(
                self.seed,
                &specification,
                ObjectiveConfirmationAction::Activate,
            ),
            objective_spec: specification,
        }));
    }

    fn install_map(&mut self) {
        let objective_spec = match self.configuration.objective_state() {
            ObjectiveState::Mapping { objective_spec, .. } => objective_spec.clone(),
            state => panic!("install_map requires Mapping, got {state:?}"),
        };
        let revision = self
            .configuration
            .objects()
            .values()
            .filter_map(|object| match object {
                FirstClassObject::MapRevision(map)
                    if map.objective_spec.objective == self.objective =>
                {
                    Some(map.revision)
                }
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1;
        let map = map_revision(self.seed, objective_spec, revision);
        let carry = current_proofs(&self.configuration)
            .expect("Mapping proof query")
            .into_keys()
            .map(|stage| (stage, CarryVerdict::Valid))
            .collect();
        let cover = CoverJudgment {
            map: map.identity(),
            objective_spec: map.objective_spec.clone(),
            verdict: CoverVerdict::Covered,
            rationale: "generated map covers the confirmed Objective".into(),
        };
        self.push(TransitionInput::InstallMap(InstallMapInput {
            map,
            initial_routes: BTreeMap::new(),
            cover,
            carry,
        }));
    }

    fn add_route(&mut self) {
        let (map_id, stage) = match self.configuration.objective_state() {
            ObjectiveState::Navigating {
                map,
                navigation: NavState::SeekingRoute { stage },
                ..
            } => (map.clone(), stage.clone()),
            state => panic!("add_route requires SeekingRoute, got {state:?}"),
        };
        let map = map_object(&self.configuration, &map_id);
        let route = Route {
            id: RouteId::new(self.next_id("route")),
            stage: stage.clone(),
            structural_context: structural_context(&map, &stage).expect("route context"),
            hypothesis: "generated Route reaches the current Stage outcome".into(),
            assumptions: BTreeSet::from(["the bounded action remains available".into()]),
            rationale: "deterministic state-machine Route".into(),
        };
        self.push(TransitionInput::AddRoute(AddRouteInput { route }));
    }

    fn select_route(&mut self) {
        let stage = match self.configuration.objective_state() {
            ObjectiveState::Navigating {
                navigation: NavState::SeekingRoute { stage },
                ..
            } => stage.clone(),
            state => panic!("select_route requires SeekingRoute, got {state:?}"),
        };
        let route = self
            .configuration
            .objects()
            .values()
            .filter_map(|object| match object {
                FirstClassObject::Route(route)
                    if route.stage == stage
                        && self.configuration.lifecycle().route_status.get(&route.id)
                            == Some(&RouteStatus::Available) =>
                {
                    Some(route.id.clone())
                }
                _ => None,
            })
            .max()
            .expect("generated available Route for current Stage");
        self.push(TransitionInput::SelectRoute(SelectRouteInput { route }));
    }

    fn start_attempt(&mut self) {
        let (stage, route) = match self.configuration.objective_state() {
            ObjectiveState::Navigating {
                navigation: NavState::Ready { stage, route },
                ..
            } => (stage.clone(), route.clone()),
            state => panic!("start_attempt requires Ready, got {state:?}"),
        };
        let ordinal = self
            .configuration
            .objects()
            .values()
            .filter_map(|object| match object {
                FirstClassObject::Attempt(attempt) if attempt.route == route => {
                    Some(attempt.ordinal)
                }
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1;
        let bound = match self.choose(3) {
            0 => AttemptBound::ResourceBudget {
                measure: "generated steps".into(),
                limit: 3,
            },
            1 => AttemptBound::VerificationScope(BTreeSet::from(["current Stage".into()])),
            _ => AttemptBound::TerminationCondition("frozen evidence exists".into()),
        };
        let attempt = Attempt {
            id: AttemptId::new(self.next_id("attempt")),
            route,
            ordinal,
            bound,
            context: acceptance_context(&self.configuration, &stage).expect("Attempt context"),
        };
        self.push(TransitionInput::StartAttempt(StartAttemptInput { attempt }));
    }

    fn record_evidence(&mut self) {
        let (stage, attempt) = match self.configuration.objective_state() {
            ObjectiveState::Navigating {
                navigation: NavState::Attempting { stage, attempt, .. },
                ..
            } => (stage.clone(), attempt.clone()),
            state => panic!("record_evidence requires Attempting, got {state:?}"),
        };
        let context = acceptance_context(&self.configuration, &stage).expect("Evidence context");
        let claims = context
            .structural
            .contract
            .criteria
            .iter()
            .cloned()
            .map(|criterion| (criterion, EvidenceClaim::Supports))
            .collect();
        let observation = if self.choose(2) == 0 {
            FrozenObservation::Inline(CanonicalValue::String(
                "deterministic generated observation".into(),
            ))
        } else {
            FrozenObservation::CoreSnapshot(CoreSnapshot {
                digest: ContentDigest(format!("sha256:generated-{}", self.seed)),
                size_bytes: 1,
            })
        };
        let evidence = Evidence {
            id: EvidenceId::new(self.next_id("evidence")),
            subject: EvidenceSubject::Attempt(attempt),
            context,
            purpose: EvidencePurpose::StageReview,
            claims,
            observation,
            provenance: CanonicalValue::String("state-machine fixture".into()),
        };
        self.push(TransitionInput::RecordEvidence(RecordEvidenceInput {
            evidence,
        }));
    }

    fn seal_attempt(&mut self) {
        let (stage, attempt) = match self.configuration.objective_state() {
            ObjectiveState::Navigating {
                navigation: NavState::Attempting { stage, attempt, .. },
                ..
            } => (stage.clone(), attempt.clone()),
            state => panic!("seal_attempt requires Attempting, got {state:?}"),
        };
        let attempt_value = attempt_object(&self.configuration, &attempt);
        let evidence_set = evidence_universe(&self.configuration, &stage, &attempt_value.context)
            .expect("complete packet Evidence universe");
        let seal_reason = match self.choose(3) {
            0 => SealReason::Submitted,
            1 => SealReason::BoundReached,
            _ => SealReason::Interrupted,
        };
        let packet = ReviewPacket {
            id: ReviewPacketId::new(self.next_id("packet")),
            attempt,
            stage,
            context: attempt_value.context,
            termination: seal_reason,
            evidence_set,
        };
        self.push(TransitionInput::SealAttempt(SealAttemptInput {
            packet,
            seal_reason,
        }));
    }

    fn decide(&mut self, branch: ReviewBranch) {
        let (stage, packet_id) = match self.configuration.objective_state() {
            ObjectiveState::Navigating {
                navigation: NavState::Reviewing { stage, packet, .. },
                ..
            } => (stage.clone(), packet.clone()),
            state => panic!("decide requires Reviewing, got {state:?}"),
        };
        let packet = packet_object(&self.configuration, &packet_id);
        let judgment = if branch == ReviewBranch::Accept {
            CriterionJudgment::Satisfied
        } else {
            CriterionJudgment::Unknown
        };
        let judgments = packet
            .context
            .structural
            .contract
            .criteria
            .iter()
            .cloned()
            .map(|criterion| (criterion, judgment))
            .collect();
        let action = match branch {
            ReviewBranch::Accept => ReviewAction::Accept,
            ReviewBranch::Retry => ReviewAction::Retry,
            ReviewBranch::Replace => ReviewAction::Replace,
            ReviewBranch::Wait => ReviewAction::Wait(Box::new(WaitCondition {
                id: WaitConditionId::new(self.next_id("wait")),
                stage,
                context: packet.context.clone(),
                cause: "a generated external fact is unavailable".into(),
                responsible_party: "deterministic environment".into(),
                resume_condition: "new frozen observation exists".into(),
            })),
            ReviewBranch::Remap => ReviewAction::Remap {
                reason: "generated review found map drift".into(),
            },
        };
        let decision = ReviewDecision {
            id: ReviewDecisionId::new(self.next_id("decision")),
            packet: packet_id,
            judgments,
            findings: BTreeSet::new(),
            action,
        };
        self.push(TransitionInput::Decision(DecisionInput { decision }));
    }

    fn check_wait(&mut self, direction: WaitDirection) {
        let wait_id = match self.configuration.objective_state() {
            ObjectiveState::Navigating {
                navigation: NavState::Waiting { wait_condition, .. },
                ..
            } => wait_condition.clone(),
            state => panic!("check_wait requires Waiting, got {state:?}"),
        };
        let wait = wait_object(&self.configuration, &wait_id);
        let evidence = Evidence {
            id: EvidenceId::new(self.next_id("wait-evidence")),
            subject: EvidenceSubject::WaitCondition(wait_id.clone()),
            context: wait.context.clone(),
            purpose: EvidencePurpose::WaitResolution,
            claims: BTreeMap::new(),
            observation: FrozenObservation::Inline(CanonicalValue::String(
                "deterministic wait observation".into(),
            )),
            provenance: CanonicalValue::String("state-machine wait fixture".into()),
        };
        let evidence_id = evidence.identity();
        let mut evidence_set: BTreeSet<_> = self
            .configuration
            .objects()
            .values()
            .filter_map(|object| match object {
                FirstClassObject::Evidence(existing)
                    if existing.subject == EvidenceSubject::WaitCondition(wait_id.clone())
                        && existing.purpose == EvidencePurpose::WaitResolution
                        && existing.context == wait.context =>
                {
                    Some(existing.id.clone())
                }
                _ => None,
            })
            .collect();
        evidence_set.insert(evidence_id.clone());
        self.push(TransitionInput::CheckWait(CheckWaitInput {
            wait_condition: wait_id.clone(),
            evidence: BTreeMap::from([(evidence_id, evidence)]),
            judgment: WaitJudgment {
                wait_condition: wait_id,
                evidence_set,
                direction,
                rationale: "generated wait evidence was checked completely".into(),
            },
        }));
    }

    fn request_remap(&mut self) {
        assert!(matches!(
            self.configuration.objective_state(),
            ObjectiveState::Navigating { .. }
        ));
        self.push(TransitionInput::RequestRemap(RequestRemapInput {
            reason: "generated dependency drift".into(),
        }));
    }

    fn revise(&mut self) {
        let revision = self
            .configuration
            .objects()
            .values()
            .filter_map(|object| match object {
                FirstClassObject::ObjectiveSpec(specification)
                    if specification.objective == self.objective =>
                {
                    Some(specification.revision)
                }
                _ => None,
            })
            .max()
            .expect("active ObjectiveSpec")
            + 1;
        let specification = objective_spec(self.seed, self.objective.clone(), revision);
        self.push(TransitionInput::ReviseObjective(ReviseObjectiveInput {
            confirmation: confirmation(
                self.seed,
                &specification,
                ObjectiveConfirmationAction::Revise,
            ),
            objective_spec: specification,
        }));
    }

    fn abandon(&mut self) {
        let reason = "generated human stop".to_string();
        self.push(TransitionInput::Abandon(AbandonInput {
            reason: reason.clone(),
            confirmation: AbandonConfirmation {
                project: ProjectId::new(format!("project-{}", self.seed)),
                objective: self.objective.clone(),
                reason,
                heads: HeadBinding {
                    expected_project_seq: 0,
                    expected_objective_seq: self.trail.len() as u64,
                },
                confirmed: true,
            },
        }));
    }

    fn assert_terminal_rejection(&mut self) {
        assert!(matches!(
            self.configuration.objective_state(),
            ObjectiveState::Achieved { .. } | ObjectiveState::Abandoned { .. }
        ));
        let rejected = TransitionInput::RequestRemap(RequestRemapInput {
            reason: "must be rejected after terminal state".into(),
        });
        assert!(matches!(
            reduce(&self.configuration, &rejected),
            Err(ReduceError::Guard(GuardViolation {
                code: "terminal_state",
                ..
            }))
        ));

        let mut invalid_trail = self.trail.clone();
        invalid_trail.push(TrailFact {
            objective: self.objective.clone(),
            input: rejected,
        });
        assert!(matches!(
            replay(&invalid_trail),
            Err(ReplayError::TransitionRejected {
                fact_index,
                source: ReduceError::Guard(GuardViolation {
                    code: "terminal_state",
                    ..
                }),
                ..
            }) if fact_index == self.trail.len()
        ));
        self.coverage.terminal_rejections += 1;
    }
}

fn criterion(seed: u64) -> Criterion {
    Criterion {
        id: CriterionId::new(format!("criterion-{seed}")),
        statement: "generated outcome is observable".into(),
        verification_rule: "inspect the frozen generated observation".into(),
        scope: CriterionScope::Local,
    }
}

fn stage(seed: u64) -> Stage {
    Stage {
        id: StageId::new(format!("stage-{seed}")),
        name: "Generated Stage".into(),
        outcome: "generated verified outcome".into(),
        output: "generated output".into(),
        kind: StageKind::Ordinary,
    }
}

fn contract(seed: u64) -> StageContract {
    StageContract {
        outcome: "generated verified outcome".into(),
        criteria: BTreeSet::from([criterion(seed).identity()]),
        objective_boundaries: BTreeSet::from(["remain inside the generated project".into()]),
        output: "generated output".into(),
    }
}

fn objective_spec(seed: u64, objective: ObjectiveId, revision: u64) -> ObjectiveSpec {
    let criterion = criterion(seed);
    ObjectiveSpec {
        objective,
        revision,
        intended_outcome: format!("deliver generated revision {revision}"),
        criteria: BTreeMap::from([(criterion.identity(), criterion)]),
        boundaries: BTreeSet::from(["remain inside the generated project".into()]),
        excluded_claims: BTreeSet::from(["unverified completion".into()]),
    }
}

fn confirmation(
    seed: u64,
    specification: &ObjectiveSpec,
    action: ObjectiveConfirmationAction,
) -> ObjectiveConfirmation {
    ObjectiveConfirmation {
        project: ProjectId::new(format!("project-{seed}")),
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

fn map_revision(seed: u64, objective_spec: ObjectiveSpecId, revision: u64) -> MapRevision {
    let stage = stage(seed);
    let criterion = criterion(seed);
    MapRevision {
        objective_spec,
        revision,
        stages: BTreeMap::from([(stage.identity(), stage.clone())]),
        criteria: BTreeMap::from([(criterion.identity(), criterion.clone())]),
        dependencies: BTreeSet::new(),
        priorities: BTreeMap::from([(stage.identity(), 1)]),
        owners: BTreeMap::from([(criterion.identity(), stage.identity())]),
        contracts: BTreeMap::from([(stage.identity(), contract(seed))]),
    }
}

fn map_object(configuration: &DomainConfiguration, id: &MapRevisionId) -> MapRevision {
    match configuration
        .objects()
        .get(&ObjectIdentity::MapRevision(id.clone()))
    {
        Some(FirstClassObject::MapRevision(map)) => map.clone(),
        value => panic!("missing current Map {id:?}: {value:?}"),
    }
}

fn attempt_object(configuration: &DomainConfiguration, id: &AttemptId) -> Attempt {
    match configuration
        .objects()
        .get(&ObjectIdentity::Attempt(id.clone()))
    {
        Some(FirstClassObject::Attempt(attempt)) => attempt.clone(),
        value => panic!("missing current Attempt {id:?}: {value:?}"),
    }
}

fn packet_object(configuration: &DomainConfiguration, id: &ReviewPacketId) -> ReviewPacket {
    match configuration
        .objects()
        .get(&ObjectIdentity::ReviewPacket(id.clone()))
    {
        Some(FirstClassObject::ReviewPacket(packet)) => packet.clone(),
        value => panic!("missing current Packet {id:?}: {value:?}"),
    }
}

fn wait_object(configuration: &DomainConfiguration, id: &WaitConditionId) -> WaitCondition {
    match configuration
        .objects()
        .get(&ObjectIdentity::WaitCondition(id.clone()))
    {
        Some(FirstClassObject::WaitCondition(wait)) => wait.clone(),
        value => panic!("missing current WaitCondition {id:?}: {value:?}"),
    }
}

fn drive_to_attempt(machine: &mut Machine) {
    machine.activate();
    machine.install_map();
    machine.add_route();
    machine.select_route();
    machine.start_attempt();
}

fn drive_to_review(machine: &mut Machine) {
    drive_to_attempt(machine);
    machine.record_evidence();
    machine.seal_attempt();
}

fn run_seed(seed: u64) -> Machine {
    let scenario = Scenario::ALL[(seed as usize) % Scenario::ALL.len()];
    let mut machine = Machine::new(seed);
    match scenario {
        Scenario::Accept => {
            drive_to_review(&mut machine);
            machine.decide(ReviewBranch::Accept);
        }
        Scenario::Retry => {
            drive_to_review(&mut machine);
            machine.decide(ReviewBranch::Retry);
            machine.start_attempt();
            machine.record_evidence();
            machine.seal_attempt();
            machine.decide(ReviewBranch::Accept);
        }
        Scenario::Replace => {
            drive_to_review(&mut machine);
            machine.decide(ReviewBranch::Replace);
            machine.add_route();
            machine.abandon();
        }
        Scenario::ReviewRemap => {
            drive_to_review(&mut machine);
            machine.decide(ReviewBranch::Remap);
            machine.install_map();
            machine.abandon();
        }
        Scenario::Wait(direction) => {
            drive_to_review(&mut machine);
            machine.decide(ReviewBranch::Wait);
            machine.check_wait(direction);
            if direction == WaitDirection::NewRoute {
                machine.add_route();
            } else if direction == WaitDirection::Remap {
                machine.install_map();
            }
            machine.abandon();
        }
        Scenario::RequestRemap => {
            drive_to_attempt(&mut machine);
            machine.request_remap();
            machine.install_map();
            machine.abandon();
        }
        Scenario::Revise => {
            drive_to_attempt(&mut machine);
            machine.revise();
            machine.install_map();
            machine.abandon();
        }
        Scenario::Abandon => {
            drive_to_attempt(&mut machine);
            machine.abandon();
        }
    }
    machine.assert_terminal_rejection();
    machine
}

#[test]
fn generated_state_machine_covers_all_phase_one_transitions_and_historical_audits() {
    let mut aggregate = Coverage::default();
    for seed in 0..(Scenario::ALL.len() as u64 * 2) {
        let first = run_seed(seed);
        let second = run_seed(seed);
        assert_eq!(
            first.trail, second.trail,
            "seed {seed} is not deterministic"
        );
        assert_eq!(
            first.configuration, second.configuration,
            "seed {seed} projection is not deterministic"
        );
        assert_eq!(
            first.coverage, second.coverage,
            "seed {seed} coverage is not deterministic"
        );
        aggregate.merge(&first.coverage);
    }

    assert_eq!(
        aggregate.transitions,
        TransitionKind::ALL.into_iter().collect(),
        "generated Trails must cover all twelve typed transitions"
    );
    assert_eq!(
        aggregate.review_branches,
        BTreeSet::from([
            ReviewBranch::Accept,
            ReviewBranch::Retry,
            ReviewBranch::Replace,
            ReviewBranch::Wait,
            ReviewBranch::Remap,
        ]),
        "generated Trails must cover every Review action"
    );
    assert_eq!(
        aggregate.wait_directions,
        BTreeSet::from([
            WaitDirection::Stay,
            WaitDirection::SameRoute,
            WaitDirection::NewRoute,
            WaitDirection::Remap,
        ]),
        "generated Trails must cover all four Wait directions"
    );
    assert_eq!(
        aggregate.historical_invariants,
        BTreeSet::from(["I6", "I17", "I19"]),
        "generated prefixes must exercise every replay-only historical audit"
    );
    assert_eq!(
        aggregate.terminal_rejections,
        Scenario::ALL.len() * 2,
        "every generated terminal projection must reject a later transition"
    );
}
