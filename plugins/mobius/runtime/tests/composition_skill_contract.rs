use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::{Value, json};

const WRITE_TOOLS: [&str; 4] = [
    "mobius_project_init",
    "mobius_capture_artifact",
    "mobius_apply_transition",
    "mobius_audit",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct AgentControlFixture {
    schema: String,
    cases: Vec<AgentControlCase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct AgentControlCase {
    id: String,
    input: AgentControlInput,
    expected: ControlFence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct AgentControlInput {
    read_shape: ReadShape,
    heads: HeadState,
    binding: BindingState,
    wrapper: WrapperState,
    structural_context: StructuralContextState,
    subject_context: SubjectContextState,
    review_closure: ReviewClosureState,
    artifact_integrity: ArtifactIntegrityState,
    wait_admission: WaitAdmissionState,
    context_freshness: ContextFreshness,
}

macro_rules! string_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
        #[serde(rename_all = "snake_case")]
        enum $name {
            $($variant),+
        }
    };
}

string_enum!(ReadShape {
    SupportedLiteral,
    Unsupported
});
string_enum!(HeadState { Current, Stale });
string_enum!(BindingState {
    Definitive,
    Unknown
});
string_enum!(WrapperState { Canonical, Wrong });
string_enum!(StructuralContextState { Exact, Missing });
string_enum!(SubjectContextState { Exact, Miscopied });
string_enum!(ReviewClosureState {
    Complete,
    Incomplete
});
string_enum!(ArtifactIntegrityState {
    Verified,
    Mismatched
});
string_enum!(WaitAdmissionState {
    Complete,
    OverBudget
});
string_enum!(ContextFreshness { Fresh, Remembered });
string_enum!(ControlState {
    Unknown,
    Reload,
    SeekingRoute,
    Reviewing,
    Waiting
});
string_enum!(FenceState { Closed });
string_enum!(MutationState { Blocked });

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ControlFence {
    state: ControlState,
    fence: FenceState,
    mutation: MutationState,
    discard: Vec<String>,
    recovery: String,
}

#[derive(Debug, Eq, PartialEq)]
enum ControlEvaluation {
    Fenced(ControlFence),
    Submitted,
}

fn fence(state: ControlState, discard: &[&str], recovery: &str) -> ControlEvaluation {
    ControlEvaluation::Fenced(ControlFence {
        state,
        fence: FenceState::Closed,
        mutation: MutationState::Blocked,
        discard: discard.iter().map(|item| (*item).to_owned()).collect(),
        recovery: recovery.to_owned(),
    })
}

fn control_fault_count(input: &AgentControlInput) -> usize {
    usize::from(input.read_shape == ReadShape::Unsupported)
        + usize::from(input.heads == HeadState::Stale)
        + usize::from(input.binding == BindingState::Unknown)
        + usize::from(input.wrapper == WrapperState::Wrong)
        + usize::from(input.structural_context == StructuralContextState::Missing)
        + usize::from(input.subject_context == SubjectContextState::Miscopied)
        + usize::from(input.review_closure == ReviewClosureState::Incomplete)
        + usize::from(input.artifact_integrity == ArtifactIntegrityState::Mismatched)
        + usize::from(input.wait_admission == WaitAdmissionState::OverBudget)
        + usize::from(input.context_freshness == ContextFreshness::Remembered)
}

fn evaluate_control_fault(
    input: &AgentControlInput,
    mut submit: impl FnMut(),
) -> ControlEvaluation {
    if input.read_shape == ReadShape::Unsupported {
        return fence(
            ControlState::Unknown,
            &["draft", "request_id", "remembered_heads"],
            "rerun standalone SQLite discovery and rebuild one supported literal read",
        );
    }
    if input.heads == HeadState::Stale {
        return fence(
            ControlState::Reload,
            &[
                "draft",
                "request_id",
                "semantic_decision",
                "confirmation",
                "closure_or_batch",
            ],
            "re-read both heads, compact state, and exact subject before new judgment",
        );
    }
    if input.binding == BindingState::Unknown {
        return fence(
            ControlState::Unknown,
            &["draft", "request_id", "remembered_binding"],
            "run standalone discovery or doctor and initialize only after explicit activation plus definitive absent binding",
        );
    }
    if input.wrapper == WrapperState::Wrong {
        return fence(
            ControlState::Reload,
            &["complete_command", "request_id"],
            "select the canonical transition template and rebuild every field",
        );
    }
    if input.structural_context == StructuralContextState::Missing {
        return fence(
            ControlState::SeekingRoute,
            &["route_draft", "request_id"],
            "copy exact StructuralContext from the current typed Stage contract and dependencies",
        );
    }
    if input.subject_context == SubjectContextState::Miscopied {
        return fence(
            ControlState::Reload,
            &["draft", "request_id", "semantic_decision"],
            "read the exact current object and copy typed subject and Context anew",
        );
    }
    if input.review_closure == ReviewClosureState::Incomplete {
        return fence(
            ControlState::Reviewing,
            &["review_closure", "decision_draft", "request_id"],
            "restart recursive exact-identity closure from the current Packet",
        );
    }
    if input.artifact_integrity == ArtifactIntegrityState::Mismatched {
        return fence(
            ControlState::Reviewing,
            &["review_closure", "decision_draft", "request_id"],
            "report degraded integrity and restore the exact bytes through explicit trusted recovery",
        );
    }
    if input.wait_admission == WaitAdmissionState::OverBudget {
        return fence(
            ControlState::Waiting,
            &["wait_batch", "judgment_draft", "request_id"],
            "retain summary only and wait for a complete same-snapshot admitted set",
        );
    }
    if input.context_freshness == ContextFreshness::Remembered {
        return fence(
            ControlState::Reload,
            &[
                "remembered_heads",
                "remembered_paths",
                "draft",
                "request_id",
                "closure_or_batch",
            ],
            "reload the Skill and host card, then read live state and its one state-specific recipe",
        );
    }

    submit();
    ControlEvaluation::Submitted
}

fn plugin_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("runtime must live directly under the plugin root")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = plugin_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn skills() -> [(String, String); 2] {
    [
        ("Copilot".to_owned(), read("skills/mobius-copilot/SKILL.md")),
        ("Loop".to_owned(), read("skills/mobius-loop/SKILL.md")),
    ]
}

fn contains_prose(haystack: &str, needle: &str) -> bool {
    let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    normalize(haystack).contains(&normalize(needle))
}

fn live_apply_transition_schema() -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mobius"))
        .arg("mcp")
        .current_dir(plugin_root())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start stdio MCP for its live tool schema");
    {
        let stdin = child.stdin.as_mut().expect("MCP stdin is piped");
        for request in [
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "composition-contract", "version": "1"}
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {}
            }),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
        ] {
            serde_json::to_writer(&mut *stdin, &request).expect("serialize MCP request");
            stdin.write_all(b"\n").expect("write MCP delimiter");
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("collect stdio MCP output");
    assert!(
        output.status.success(),
        "stdio MCP schema query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response = String::from_utf8(output.stdout)
        .expect("MCP stdout is UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("MCP response is JSON"))
        .find(|response| response["id"] == 2)
        .expect("tools/list response exists");
    response["result"]["tools"]
        .as_array()
        .expect("tools/list returns an array")
        .iter()
        .find(|tool| tool["name"] == "mobius_apply_transition")
        .expect("mutation tool is listed")["inputSchema"]
        .clone()
}

fn live_command_required_fields() -> BTreeMap<String, Vec<String>> {
    let schema = live_apply_transition_schema();
    schema["properties"]["command"]["oneOf"]
        .as_array()
        .expect("command schema has oneOf variants")
        .iter()
        .map(|variant| {
            let command = variant["required"]
                .as_array()
                .and_then(|required| required.first())
                .and_then(Value::as_str)
                .expect("command variant has one required external tag");
            let payload = &variant["properties"][command];
            let payload =
                payload
                    .get("$ref")
                    .and_then(Value::as_str)
                    .map_or(payload, |reference| {
                        let name = reference
                            .strip_prefix("#/$defs/")
                            .expect("payload ref remains in the input schema");
                        &schema["$defs"][name]
                    });
            let required = payload["required"]
                .as_array()
                .expect("command payload has required fields")
                .iter()
                .map(|field| field.as_str().unwrap().to_owned())
                .collect();
            (command.to_owned(), required)
        })
        .collect()
}

fn transition_template_entries() -> Vec<(String, Vec<String>, BTreeSet<String>)> {
    [
        "skills/mobius-copilot/references/transition-drafts.md",
        "skills/mobius-loop/references/transition-drafts.md",
    ]
    .into_iter()
    .flat_map(|path| {
        let reference = read(path);
        reference
            .split("```json\n")
            .skip(1)
            .map(|tail| {
                let block = tail
                    .split_once("\n```")
                    .expect("transition JSON fence is closed")
                    .0;
                serde_json::from_str::<Value>(block).expect("transition template is valid JSON")
            })
            .collect::<Vec<_>>()
    })
    .map(|entry| {
        let command = entry["command"].as_str().unwrap().to_owned();
        let required = entry["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let outer = entry["template"].as_object().unwrap();
        assert_eq!(outer.len(), 1, "template must have one command tag");
        let payload = outer
            .get(&command)
            .expect("template tag matches command")
            .as_object()
            .expect("command payload is an object")
            .keys()
            .cloned()
            .collect();
        (command, required, payload)
    })
    .collect()
}

#[test]
fn composition_requires_an_explicit_named_objective_and_action() {
    for (skill_path, metadata_path, expected_action) in [
        (
            "skills/mobius-copilot/SKILL.md",
            "skills/mobius-copilot/agents/openai.yaml",
            "Activate, revise, abandon",
        ),
        (
            "skills/mobius-loop/SKILL.md",
            "skills/mobius-loop/agents/openai.yaml",
            "Run or continue",
        ),
    ] {
        let skill = read(skill_path);
        let frontmatter = skill.split("---").nth(1).expect("skill frontmatter");
        assert!(frontmatter.contains("explicitly named Mobius Objective"));
        assert!(frontmatter.contains(expected_action));
        assert!(read(metadata_path).contains("allow_implicit_invocation: false"));
    }
    assert!(
        read("skills/mobius-copilot/SKILL.md")
            .contains("ordinary planning, optimization, review, or advice stays outside Mobius")
    );
}

#[test]
fn composition_hot_paths_are_smaller_and_state_driven() {
    let copilot = read("skills/mobius-copilot/SKILL.md");
    let loop_skill = read("skills/mobius-loop/SKILL.md");
    let intent = read("skills/mobius-copilot/references/intent-elicitation.md");
    let review = read("skills/mobius-loop/references/review-read.md");

    assert!(copilot.len() <= 7_884, "Copilot hot path grew");
    assert!(loop_skill.len() <= 7_640, "Loop hot path grew");
    assert!(
        copilot.len() + intent.len() <= 7_884 + 2_676,
        "activation path grew"
    );
    assert!(
        loop_skill.len() + review.len() <= 7_640 + 2_920,
        "review path grew"
    );
    assert!(
        !plugin_root()
            .join("skills/composition-agent-cards.md")
            .exists(),
        "a cross-skill card file would violate self-contained skill resources"
    );

    for (role, skill) in [("Copilot", &copilot), ("Loop", &loop_skill)] {
        for contract in [
            "## Keep one cockpit",
            "## Fence every submission",
            "Never retry an unchanged payload",
            "<this skill directory>/../../bin/mobius",
            "`command -v mobius`",
            "Never persist it or patch remembered heads",
        ] {
            assert!(contains_prose(skill, contract), "{role} omitted {contract}");
        }
    }
    assert!(loop_skill.contains("After compaction/interruption/handoff"));
}

#[test]
fn composition_has_one_bounded_sql_read_path_and_four_write_tools() {
    for (role, skill) in skills() {
        for tool in WRITE_TOOLS {
            assert!(
                skill.contains(&format!("`{tool}`")),
                "{role} omitted {tool}"
            );
        }
        for contract in [
            "3.40.1",
            "`type -P sqlite3`",
            "Never guess `/usr/bin/sqlite3`",
            "--safe --readonly --batch --bail --init /dev/null --line",
            "PRAGMA query_only=ON; BEGIN;",
            "sqlite_text(v)",
            "shell_word(v)",
            "with `'\"'\"'`",
            "Replace the whole quoted",
            "SELECT *",
            "finite ordered Trail",
            "untrusted data",
        ] {
            assert!(
                contains_prose(&skill, contract),
                "{role} omitted {contract}"
            );
        }
        for removed in ["`mobius_read`", "`mobius_read_artifact`", "Agent ORM"] {
            assert!(!skill.contains(removed), "{role} retained {removed}");
        }
    }
}

#[test]
fn live_state_router_preserves_model_and_composition_ownership() {
    let copilot = read("skills/mobius-copilot/SKILL.md");
    let loop_skill = read("skills/mobius-loop/SKILL.md");

    for reason in ["Initial", "SpecRevised", "Remap", "WaitRevealedDrift"] {
        assert!(copilot.contains(reason), "Copilot omitted {reason}");
        assert!(loop_skill.contains(reason), "Loop omitted {reason}");
    }
    for state in [
        "SeekingRoute",
        "Ready(s,r)",
        "Attempting(s,r,a)",
        "Reviewing(s,r,a,P)",
        "Waiting(s,r,b)",
        "Achieved",
        "Abandoned",
    ] {
        assert!(loop_skill.contains(state), "Loop omitted {state}");
    }
    assert!(copilot.contains("Explicit activation + no active Objective"));
    assert!(copilot.contains("initial_routes={}"));
    assert!(loop_skill.contains("Design every Route yourself"));
    assert!(loop_skill.contains("Only while preparing"));
    assert!(loop_skill.contains("Never load either recipe elsewhere"));
}

#[test]
fn activation_uses_one_complete_interaction_sibling_shape() {
    let copilot = read("skills/mobius-copilot/SKILL.md");
    let intent = read("skills/mobius-copilot/references/intent-elicitation.md");

    for contract in [
        "references/intent-elicitation.md",
        "one ObjectiveSpec",
        "one minimal acyclic Map",
        "complete five-field top-level `interaction`",
        "`interaction_path`",
    ] {
        assert!(copilot.contains(contract), "Copilot omitted {contract}");
    }
    for contract in [
        "\"project_root\"",
        "\"expected_heads\"",
        "\"command\"",
        "\"interaction\"",
        "\"objective_spec\"",
        "\"confirmation\"",
        "\"interpreted_intent\"",
        "\"route_notes\"",
        "not a transcript",
    ] {
        assert!(
            intent.contains(contract),
            "intent recipe omitted {contract}"
        );
    }
}

#[test]
fn review_and_wait_recipes_are_loaded_only_for_their_live_state() {
    let loop_skill = read("skills/mobius-loop/SKILL.md");
    let review = read("skills/mobius-loop/references/review-read.md");
    let wait = read("skills/mobius-loop/references/wait-read.md");

    for contract in [
        "Packet:   expected | read | kind/id verified",
        "`projection_bytes`",
        "`COUNT(*)`",
        "prior-session read",
        "Create one fresh required Judge task",
        "Only a valid, current, complete Judge result",
        "Unresolved Judge findings block `accept`",
        "Re-read both heads, live `Reviewing` state",
    ] {
        assert!(
            contains_prose(&review, contract),
            "review recipe omitted {contract}"
        );
    }
    for contract in [
        "WITH current_wait AS MATERIALIZED",
        "matching AS MATERIALIZED",
        "matching_count",
        "complete admitted set or none",
        "keeps the Objective `Waiting`",
        "no `LIMIT`",
    ] {
        assert!(wait.contains(contract), "wait recipe omitted {contract}");
    }
    assert!(loop_skill.contains("In `Reviewing` only"));
    assert!(loop_skill.contains("In `Waiting` only"));
}

#[test]
fn delegated_work_is_fresh_and_stage_review_always_invokes_judge() {
    let loop_skill = read("skills/mobius-loop/SKILL.md");
    for contract in [
        "one bounded task has material value",
        "self-contained boundary",
        "a fresh baseline",
        "Do not call any Mobius MCP tool.",
        "Do not read or write `.mobius/` managed state.",
        "candidates, never Evidence or Judgment",
        "Never pass a Core handle or mutation instruction",
        "Driver and Verifier remain optional",
        "Every Stage Review creates one required Judge",
        "after recursive Packet closure and material freeze",
        "Missing, unavailable, degraded, stale, partial, or inconclusive Judge advice blocks `accept`",
        "main completes formal Review",
    ] {
        assert!(
            contains_prose(&loop_skill, contract),
            "Loop omitted {contract}"
        );
    }
}

#[test]
fn evidence_bundle_recipe_gates_freshness_without_shortening_machine_identity() {
    let loop_skill = read("skills/mobius-loop/SKILL.md");
    let recipe = read("skills/mobius-loop/references/evidence-bundle.md");

    for contract in [
        "In `Attempting`",
        "references/evidence-bundle.md",
        "before accepting externally dependent observations or sealing",
        "In `Reviewing` only",
        "closure and applicability",
    ] {
        assert!(
            contains_prose(&loop_skill, contract),
            "Loop omitted Evidence Bundle routing contract {contract}"
        );
    }
    for contract in [
        "mobius.evidence-bundle.v1",
        "mobius.canonical-json.v1",
        "repository_worktree | artifact_set | external_object_set | intrinsic",
        "reject canonical bytes over 131072 bytes",
        "`sha256:` plus 64 lowercase hexadecimal characters",
        "`sha256:…<last 7 hex>` only as a display hint",
        "Never recover, compare, freeze, submit, or admit by a short suffix",
        "Before `RecordEvidence`",
        "Before `SealAttempt`",
        "Before `Decision`",
        "current-applicable",
        "superseded",
        "unverifiable",
        "RequestRemap",
        "new Objective",
        "deterministic test oracle only",
    ] {
        assert!(
            contains_prose(&recipe, contract),
            "Evidence Bundle recipe omitted {contract}"
        );
    }
}

#[test]
fn canonical_transition_drafts_match_the_live_mcp_schema() {
    let live = live_command_required_fields();
    let entries = transition_template_entries();
    let mut covered = BTreeSet::new();

    for (command, required, payload_fields) in entries {
        let live_required = live
            .get(&command)
            .unwrap_or_else(|| panic!("template names unknown command {command}"));
        assert_eq!(
            &required, live_required,
            "template required-field order drifted for {command}"
        );
        assert_eq!(
            payload_fields,
            required.iter().cloned().collect(),
            "template payload fields drifted for {command}"
        );
        covered.insert(command);
    }

    assert_eq!(
        covered,
        live.keys().cloned().collect(),
        "the two Composition skills must cover every mutation command"
    );
}

#[test]
fn risk_gate_requires_one_fresh_judge_per_stage_review_without_vote_authority() {
    let gate = read("skills/mobius-loop/references/risk-gate.md");
    for contract in [
        "Build one Stage Risk Card",
        "migration/storage",
        "concurrency/lease/locking",
        "filesystem/path/symlink/permissions",
        "crash/recovery/durability",
        "cross-stage integration",
        "Evidence independence and coverage",
        "Start it only after the relevant effects have happened and stabilized",
        "Do not require a Driver for every Attempt",
        "one required Judge execution per Stage Review",
        "Additional Judges require distinct information value",
        "There is no Judge-free Composition accept path",
        "Do not call any Mobius MCP tool.",
        "Do not read or write `.mobius/` managed state.",
        "exactly one complete public result envelope",
        "Changed objective, role, authorization, baseline, or frozen material requires a fresh task",
        "Apply the Judge freeze gate",
        "A missing Judge task or result is `absent`",
        "blocks `accept`",
        "Judge output is advice only",
        "closed enums in the generic Judge contract",
        "never accept arbitrary coverage text as a status",
        "Close every material, question, criterion, risk, finding, recommendation, artifact, and evidence reference",
    ] {
        assert!(
            contains_prose(&gate, contract),
            "risk gate omitted {contract}"
        );
    }
    for forbidden in [
        "Driver is mandatory",
        "Judge: candidate | skip",
        "majority vote",
        "named review skill is required",
        "private review result format",
    ] {
        assert!(!gate.contains(forbidden), "risk gate retained {forbidden}");
    }
}

#[test]
fn deterministic_agent_control_faults_all_close_the_fence() {
    let fixture: AgentControlFixture =
        serde_json::from_str(&read("runtime/tests/fixtures/agent-control-faults-v1.json"))
            .expect("agent-control fixture is valid JSON");
    assert_eq!(fixture.schema, "mobius.agent-control-faults.v1");
    assert_eq!(fixture.cases.len(), 10);
    let ids = fixture
        .cases
        .iter()
        .map(|case| case.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 10, "fault ids must be unique");
    for case in &fixture.cases {
        assert_eq!(
            control_fault_count(&case.input),
            1,
            "{} must isolate exactly one control fault",
            case.id
        );
        assert!(!case.expected.discard.is_empty());
        assert!(!case.expected.recovery.trim().is_empty());

        let mut submissions = 0;
        let actual = evaluate_control_fault(&case.input, || submissions += 1);
        assert_eq!(actual, ControlEvaluation::Fenced(case.expected.clone()));
        assert_eq!(
            submissions, 0,
            "{} reached the mutation submission closure",
            case.id
        );
    }

    let mut safe = fixture.cases[0].input.clone();
    safe.read_shape = ReadShape::SupportedLiteral;
    assert_eq!(control_fault_count(&safe), 0);
    let mut submissions = 0;
    assert_eq!(
        evaluate_control_fault(&safe, || submissions += 1),
        ControlEvaluation::Submitted
    );
    assert_eq!(
        submissions, 1,
        "the safe-path probe must exercise submission"
    );

    let evaluation = read("../../docs/agent-control-evaluation.md");
    for contract in [
        "at least ten allowlist-redacted representative real sessions",
        "at least one hundred transition drafts",
        "C1 is therefore `not_evaluated`",
        "does not add `inspect`",
        "Reaching a trigger permits only an ADR proposal",
    ] {
        assert!(
            contains_prose(&evaluation, contract),
            "C1 evaluation omitted {contract}"
        );
    }
}

#[test]
fn views_and_completion_remain_bounded() {
    let copilot = read("skills/mobius-copilot/SKILL.md");
    let loop_skill = read("skills/mobius-loop/SKILL.md");
    assert!(contains_prose(
        &copilot,
        "Reports and CSV files are presentation"
    ));
    assert!(contains_prose(
        &loop_skill,
        "Reports and CSV files are presentation"
    ));
    assert!(!copilot.contains("MOBIUS_OBJECTIVE_ACHIEVED:"));
    assert_eq!(loop_skill.matches("MOBIUS_OBJECTIVE_ACHIEVED:").count(), 1);
    assert!(loop_skill.contains("<shell_word(packaged-mobius)> audit"));
    assert!(loop_skill.contains("require a healthy result"));
}
