use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

const FIXTURE_TEXT: &str = include_str!("fixtures/proof-impact-v1.json");
const FIXTURE_SCHEMA: &str = "mobius.proof-impact-fixture.v1";
const EXPECTED_SCENARIO_IDS: [&str; 6] = [
    "achieved-late-lease-defect",
    "incomplete-ownership-scope-fails-closed",
    "s3-lease-timing-token-change",
    "s3-symlink-security-boundary-change",
    "s4-migration-bootstrap-ownership-change",
    "s5-exclusive-docs-proven-disjoint",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema: String,
    boundary: Boundary,
    scenarios: Vec<Scenario>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Boundary {
    test_only: bool,
    deterministic: bool,
    runtime_parser: bool,
    core_state: bool,
    production_api: bool,
    persistence: bool,
    ledger: bool,
    business_transition: bool,
    business_fact: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Scenario {
    id: String,
    accepted_proof: AcceptedProof,
    context: Context,
    change: MaterialChange,
    expected: ExpectedOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct AcceptedProof {
    decision_id: String,
    decision_state: DecisionState,
    owning_stage: String,
    frozen_scope: Vec<String>,
    dependencies: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DecisionState {
    Accepted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Context {
    objective_state: ObjectiveState,
    current_stage: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum ObjectiveState {
    Navigating,
    Achieved,
    Abandoned,
}

impl ObjectiveState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Achieved | Self::Abandoned)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct MaterialChange {
    surfaces: Vec<String>,
    affected_dependencies: Vec<String>,
    scope_knowledge: Knowledge,
    dependency_knowledge: Knowledge,
    identity_knowledge: Knowledge,
    ownership: Ownership,
    material_kinds: Vec<MaterialKind>,
    semantic_change: SemanticChange,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Knowledge {
    Complete,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Ownership {
    Unchanged,
    ImplementationChanged,
    CriterionChanged,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum MaterialKind {
    Documentation,
    FilesystemSecurity,
    GlobalConfiguration,
    LeaseConcurrency,
    MigrationBootstrap,
    SharedFilesystem,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum SemanticChange {
    None,
    StageContract,
    DependencyTopology,
    AcceptanceUnderstanding,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ExpectedOutcome {
    disposition: Disposition,
    lifecycle: Lifecycle,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum Disposition {
    Unaffected,
    NeedsReverification,
    RequiresRemap,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Lifecycle {
    action: LifecycleAction,
    transition_intent: TransitionIntent,
    proof_carry: ProofCarry,
    map_handling: MapHandling,
    terminal_follow_up: TerminalFollowUp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum LifecycleAction {
    ContinueWithoutRemap,
    RequestRemapReverify,
    RequestRemapReviseAndReverify,
    RequestRemapFailClosed,
    ReportAuditFindingOrNewObjective,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum TransitionIntent {
    None,
    RequestRemap,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ProofCarry {
    NoOperation,
    InvalidateAffectedAndTransitive,
    OnlyExplicitlyValid,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum MapHandling {
    NoOperation,
    InstallRevisionSameStructure,
    ReviseStructureBeforeInstall,
    ReassessBeforeInstall,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum TerminalFollowUp {
    None,
    AuditFindingOrUserAuthorizedNewObjective,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Evaluation {
    disposition: Disposition,
    lifecycle: Lifecycle,
    implicit_fact_effects: FactEffects,
    terminal_reopened: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FactEffects {
    evidence: FactEffect,
    decision: FactEffect,
    trail: FactEffect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FactEffect {
    Unchanged,
}

const NO_FACT_EFFECTS: FactEffects = FactEffects {
    evidence: FactEffect::Unchanged,
    decision: FactEffect::Unchanged,
    trail: FactEffect::Unchanged,
};

fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE_TEXT).expect("proof-impact fixture must satisfy its closed schema")
}

fn locators_overlap(left: &str, right: &str) -> bool {
    fn contains(parent: &str, candidate: &str) -> bool {
        candidate == parent
            || candidate
                .strip_prefix(parent)
                .is_some_and(|suffix| suffix.starts_with('/'))
    }

    contains(left, right) || contains(right, left)
}

fn is_cross_cutting(kind: MaterialKind) -> bool {
    matches!(
        kind,
        MaterialKind::FilesystemSecurity
            | MaterialKind::GlobalConfiguration
            | MaterialKind::LeaseConcurrency
            | MaterialKind::MigrationBootstrap
            | MaterialKind::SharedFilesystem
    )
}

fn disposition(scenario: &Scenario) -> Disposition {
    let change = &scenario.change;
    if change.scope_knowledge == Knowledge::Incomplete
        || change.dependency_knowledge == Knowledge::Incomplete
        || change.identity_knowledge == Knowledge::Incomplete
        || change.ownership == Ownership::Incomplete
    {
        return Disposition::Unknown;
    }

    if change.ownership == Ownership::CriterionChanged
        || change.semantic_change != SemanticChange::None
    {
        return Disposition::RequiresRemap;
    }

    let scope_intersects = change.surfaces.iter().any(|surface| {
        scenario
            .accepted_proof
            .frozen_scope
            .iter()
            .any(|frozen| locators_overlap(surface, frozen))
    });
    let dependency_intersects = change.affected_dependencies.iter().any(|dependency| {
        scenario
            .accepted_proof
            .dependencies
            .iter()
            .any(|frozen| locators_overlap(dependency, frozen))
    });
    let cross_cutting = change.material_kinds.iter().copied().any(is_cross_cutting);

    if scope_intersects || dependency_intersects || cross_cutting {
        Disposition::NeedsReverification
    } else {
        Disposition::Unaffected
    }
}

fn lifecycle(objective_state: ObjectiveState, disposition: Disposition) -> Lifecycle {
    if objective_state.is_terminal() {
        return Lifecycle {
            action: LifecycleAction::ReportAuditFindingOrNewObjective,
            transition_intent: TransitionIntent::None,
            proof_carry: ProofCarry::NoOperation,
            map_handling: MapHandling::NoOperation,
            terminal_follow_up: TerminalFollowUp::AuditFindingOrUserAuthorizedNewObjective,
        };
    }

    match disposition {
        Disposition::Unaffected => Lifecycle {
            action: LifecycleAction::ContinueWithoutRemap,
            transition_intent: TransitionIntent::None,
            proof_carry: ProofCarry::NoOperation,
            map_handling: MapHandling::NoOperation,
            terminal_follow_up: TerminalFollowUp::None,
        },
        Disposition::NeedsReverification => Lifecycle {
            action: LifecycleAction::RequestRemapReverify,
            transition_intent: TransitionIntent::RequestRemap,
            proof_carry: ProofCarry::InvalidateAffectedAndTransitive,
            map_handling: MapHandling::InstallRevisionSameStructure,
            terminal_follow_up: TerminalFollowUp::None,
        },
        Disposition::RequiresRemap => Lifecycle {
            action: LifecycleAction::RequestRemapReviseAndReverify,
            transition_intent: TransitionIntent::RequestRemap,
            proof_carry: ProofCarry::OnlyExplicitlyValid,
            map_handling: MapHandling::ReviseStructureBeforeInstall,
            terminal_follow_up: TerminalFollowUp::None,
        },
        Disposition::Unknown => Lifecycle {
            action: LifecycleAction::RequestRemapFailClosed,
            transition_intent: TransitionIntent::RequestRemap,
            proof_carry: ProofCarry::OnlyExplicitlyValid,
            map_handling: MapHandling::ReassessBeforeInstall,
            terminal_follow_up: TerminalFollowUp::None,
        },
    }
}

fn evaluate(scenario: &Scenario) -> Evaluation {
    let disposition = disposition(scenario);
    Evaluation {
        disposition,
        lifecycle: lifecycle(scenario.context.objective_state, disposition),
        implicit_fact_effects: NO_FACT_EFFECTS,
        terminal_reopened: false,
    }
}

fn unique_nonempty(values: &[String], label: &str) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.is_empty() {
            return Err(format!("{label} contains an empty value"));
        }
        if !seen.insert(value) {
            return Err(format!("{label} contains duplicate {value}"));
        }
    }
    Ok(())
}

fn validate_scenario(scenario: &Scenario) -> Result<(), String> {
    if scenario.id.is_empty()
        || scenario.accepted_proof.decision_id.is_empty()
        || scenario.accepted_proof.owning_stage.is_empty()
    {
        return Err("scenario, Decision, and owning Stage IDs must be non-empty".to_owned());
    }
    if scenario.accepted_proof.decision_state != DecisionState::Accepted {
        return Err("proof-impact fixtures must start from accepted Decisions".to_owned());
    }
    unique_nonempty(&scenario.accepted_proof.frozen_scope, "frozen scope")?;
    unique_nonempty(&scenario.accepted_proof.dependencies, "dependencies")?;
    unique_nonempty(&scenario.change.surfaces, "changed surfaces")?;
    unique_nonempty(
        &scenario.change.affected_dependencies,
        "affected dependencies",
    )?;
    if scenario.change.material_kinds.is_empty()
        || scenario
            .change
            .material_kinds
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != scenario.change.material_kinds.len()
    {
        return Err("material kinds must be non-empty and unique".to_owned());
    }
    match scenario.context.objective_state {
        ObjectiveState::Navigating if scenario.context.current_stage.is_none() => {
            Err("Navigating requires a current Stage".to_owned())
        }
        state if state.is_terminal() && scenario.context.current_stage.is_some() => {
            Err("terminal Objectives cannot have a current Stage".to_owned())
        }
        _ => Ok(()),
    }
}

#[test]
fn fixture_is_bounded_synthetic_and_has_exact_scenario_coverage() {
    let fixture = fixture();
    assert_eq!(fixture.schema, FIXTURE_SCHEMA);
    assert!(fixture.boundary.test_only);
    assert!(fixture.boundary.deterministic);
    assert!(!fixture.boundary.runtime_parser);
    assert!(!fixture.boundary.core_state);
    assert!(!fixture.boundary.production_api);
    assert!(!fixture.boundary.persistence);
    assert!(!fixture.boundary.ledger);
    assert!(!fixture.boundary.business_transition);
    assert!(!fixture.boundary.business_fact);

    let expected_ids = EXPECTED_SCENARIO_IDS.into_iter().collect::<BTreeSet<_>>();
    let mut scenario_ids = BTreeSet::new();
    let mut decision_ids = BTreeSet::new();
    for scenario in &fixture.scenarios {
        validate_scenario(scenario)
            .unwrap_or_else(|error| panic!("scenario {} is invalid: {error}", scenario.id));
        assert!(
            scenario_ids.insert(scenario.id.as_str()),
            "duplicate scenario ID {}",
            scenario.id
        );
        assert!(
            decision_ids.insert(scenario.accepted_proof.decision_id.as_str()),
            "duplicate Decision ID {}",
            scenario.accepted_proof.decision_id
        );
    }
    assert_eq!(fixture.scenarios.len(), EXPECTED_SCENARIO_IDS.len());
    assert_eq!(scenario_ids, expected_ids);

    let normalized = FIXTURE_TEXT.to_ascii_lowercase();
    for forbidden in [
        "/home/",
        "/users/",
        "/root/",
        "file://",
        "session",
        "secret",
        "provider",
        "\"model",
        "transcript",
        "digest",
    ] {
        assert!(
            !normalized.contains(forbidden),
            "fixture contains forbidden environment or identity material: {forbidden}"
        );
    }
}

#[test]
fn evaluator_derives_every_disposition_and_its_lifecycle_action_deterministically() {
    let fixture = fixture();
    let unchanged_fixture = fixture.clone();
    let mut forward = BTreeMap::new();
    let mut navigating_actions = BTreeMap::new();

    for scenario in &fixture.scenarios {
        let first = evaluate(scenario);
        let second = evaluate(scenario);
        assert_eq!(first, second, "{} was not deterministic", scenario.id);
        assert_eq!(
            first.disposition, scenario.expected.disposition,
            "{} disposition regressed",
            scenario.id
        );
        assert_eq!(
            first.lifecycle, scenario.expected.lifecycle,
            "{} lifecycle action regressed",
            scenario.id
        );
        assert_eq!(first.implicit_fact_effects, NO_FACT_EFFECTS);
        assert!(!first.terminal_reopened);
        if scenario.context.objective_state == ObjectiveState::Navigating {
            navigating_actions
                .entry(first.disposition)
                .or_insert(first.lifecycle.action);
        }
        forward.insert(scenario.id.clone(), first);
    }

    assert_eq!(
        navigating_actions,
        BTreeMap::from([
            (
                Disposition::Unaffected,
                LifecycleAction::ContinueWithoutRemap
            ),
            (
                Disposition::NeedsReverification,
                LifecycleAction::RequestRemapReverify,
            ),
            (
                Disposition::RequiresRemap,
                LifecycleAction::RequestRemapReviseAndReverify,
            ),
            (
                Disposition::Unknown,
                LifecycleAction::RequestRemapFailClosed,
            ),
        ])
    );

    let reverse = fixture
        .scenarios
        .iter()
        .rev()
        .map(|scenario| (scenario.id.clone(), evaluate(scenario)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(forward, reverse, "evaluation must not depend on order");
    assert_eq!(fixture, unchanged_fixture, "evaluation mutated its fixture");
}

#[test]
fn semantic_precedence_fails_closed_and_terminal_state_never_reopens() {
    let fixture = fixture();
    let mut probe = fixture
        .scenarios
        .iter()
        .find(|scenario| scenario.id == "s5-exclusive-docs-proven-disjoint")
        .expect("disjoint probe scenario exists")
        .clone();

    assert_eq!(disposition(&probe), Disposition::Unaffected);

    probe.change.scope_knowledge = Knowledge::Incomplete;
    assert_eq!(disposition(&probe), Disposition::Unknown);
    assert_eq!(
        evaluate(&probe).lifecycle.action,
        LifecycleAction::RequestRemapFailClosed
    );

    probe.change.scope_knowledge = Knowledge::Complete;
    probe.change.ownership = Ownership::CriterionChanged;
    assert_eq!(disposition(&probe), Disposition::RequiresRemap);

    probe.change.ownership = Ownership::Unchanged;
    probe.change.semantic_change = SemanticChange::StageContract;
    assert_eq!(disposition(&probe), Disposition::RequiresRemap);

    probe.change.semantic_change = SemanticChange::DependencyTopology;
    assert_eq!(disposition(&probe), Disposition::RequiresRemap);

    probe.change.semantic_change = SemanticChange::None;
    probe.change.surfaces = probe.accepted_proof.frozen_scope.clone();
    assert_eq!(disposition(&probe), Disposition::NeedsReverification);

    probe.change.material_kinds = vec![MaterialKind::Documentation];
    probe.accepted_proof.frozen_scope = vec!["src".to_owned()];
    probe.change.surfaces = vec!["src/lib.rs".to_owned()];
    assert_eq!(
        disposition(&probe),
        Disposition::NeedsReverification,
        "a changed child locator overlaps its frozen parent"
    );

    probe.accepted_proof.frozen_scope = vec!["src/domain/reducer.rs".to_owned()];
    probe.change.surfaces = vec!["src/domain".to_owned()];
    assert_eq!(
        disposition(&probe),
        Disposition::NeedsReverification,
        "a changed parent locator overlaps its frozen child"
    );

    probe.accepted_proof.frozen_scope = vec!["src/domain".to_owned()];
    probe.change.surfaces = vec!["docs/release.md".to_owned()];
    probe.change.material_kinds = vec![MaterialKind::GlobalConfiguration];
    assert_eq!(
        disposition(&probe),
        Disposition::NeedsReverification,
        "cross-cutting material cannot be declared unaffected from path disjointness alone"
    );

    probe.context.objective_state = ObjectiveState::Achieved;
    probe.context.current_stage = None;
    let terminal = evaluate(&probe);
    assert_eq!(terminal.disposition, Disposition::NeedsReverification);
    assert_eq!(
        terminal.lifecycle.action,
        LifecycleAction::ReportAuditFindingOrNewObjective
    );
    assert_eq!(terminal.lifecycle.transition_intent, TransitionIntent::None);
    assert_eq!(terminal.lifecycle.proof_carry, ProofCarry::NoOperation);
    assert_eq!(terminal.lifecycle.map_handling, MapHandling::NoOperation);
    assert_eq!(terminal.implicit_fact_effects, NO_FACT_EFFECTS);
    assert!(!terminal.terminal_reopened);

    probe.context.objective_state = ObjectiveState::Abandoned;
    assert_eq!(evaluate(&probe).lifecycle, terminal.lifecycle);
}
