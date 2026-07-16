use std::fs;
use std::path::{Path, PathBuf};

const CORE_TOOLS: [&str; 6] = [
    "mobius_project_init",
    "mobius_read",
    "mobius_capture_artifact",
    "mobius_read_artifact",
    "mobius_apply_transition",
    "mobius_audit",
];

const COPILOT_TOOLS: [&str; 4] = [
    "mobius_project_init",
    "mobius_read",
    "mobius_apply_transition",
    "mobius_audit",
];

const LOOP_TOOLS: [&str; 5] = [
    "mobius_read",
    "mobius_capture_artifact",
    "mobius_read_artifact",
    "mobius_apply_transition",
    "mobius_audit",
];

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

#[test]
fn composition_skills_require_explicit_mobius_objective_invocation() {
    for (skill_path, metadata_path) in [
        (
            "skills/mobius-copilot/SKILL.md",
            "skills/mobius-copilot/agents/openai.yaml",
        ),
        (
            "skills/mobius-loop/SKILL.md",
            "skills/mobius-loop/agents/openai.yaml",
        ),
    ] {
        let skill = read(skill_path);
        let frontmatter = skill
            .split("---")
            .nth(1)
            .expect("skill must have YAML frontmatter");
        assert!(frontmatter.contains("user explicitly"));
        assert!(frontmatter.contains("Mobius Objective"));
        assert!(frontmatter.contains("ordinary"));

        let metadata = read(metadata_path);
        assert_eq!(
            metadata.matches("allow_implicit_invocation: false").count(),
            1,
            "{metadata_path} must enforce explicit invocation in host metadata"
        );
        for dependency_field in [
            "dependencies:",
            "type: \"mcp\"",
            "value: \"mobius\"",
            "description: \"Project-local Mobius Core MCP server\"",
            "transport: \"stdio\"",
        ] {
            assert!(
                metadata.contains(dependency_field),
                "{metadata_path} is missing bundled MCP dependency field: {dependency_field}"
            );
        }
    }
}

#[test]
fn composition_uses_role_specific_core_mcp_tools_for_state() {
    for (role, skill, allowed) in [
        (
            "Copilot",
            read("skills/mobius-copilot/SKILL.md"),
            COPILOT_TOOLS.as_slice(),
        ),
        (
            "Loop",
            read("skills/mobius-loop/SKILL.md"),
            LOOP_TOOLS.as_slice(),
        ),
    ] {
        for tool in CORE_TOOLS {
            let listed = skill.contains(&format!("`{tool}`"));
            assert_eq!(
                listed,
                allowed.contains(&tool),
                "{role} Core tool boundary is wrong for {tool}"
            );
        }
        assert!(!skill.contains("python3"));
        assert!(!skill.contains("mobius.py"));
        assert!(!skill.contains("mobius report"));
    }
}

#[test]
fn delegated_lane_is_native_bounded_and_advisory() {
    let skill = read("skills/mobius-loop/SKILL.md");

    for requirement in [
        "current host's native Subagent workflow",
        "Do not call any Mobius Core MCP method",
        "Do not read or write `.mobius/` managed state",
        "Reject and rebuild an envelope when either rule is absent",
        "keep every Judge advisory",
        "zero or more independent Judges",
        "fixed role sequence or worker count.",
        "main agent",
    ] {
        assert!(skill.contains(requirement), "missing {requirement}");
    }
    for fixed_topology in ["four independent Analyst", "one Driver", "one Judge"] {
        assert!(
            !skill.contains(fixed_topology),
            "Composition must not require a fixed delegation topology: {fixed_topology}"
        );
    }
}

#[test]
fn completion_marker_is_loop_only_and_core_gated() {
    let copilot = read("skills/mobius-copilot/SKILL.md");
    let loop_skill = read("skills/mobius-loop/SKILL.md");
    assert!(!copilot.contains("MOBIUS_OBJECTIVE_ACHIEVED:"));
    assert_eq!(loop_skill.matches("MOBIUS_OBJECTIVE_ACHIEVED:").count(), 1);
    assert!(loop_skill.contains("fresh"));
    assert!(loop_skill.contains("`mobius_read`"));
    assert!(loop_skill.contains("reports `Achieved`"));
}

#[test]
fn composition_excludes_human_views_from_agent_inputs() {
    for skill in [
        read("skills/mobius-copilot/SKILL.md"),
        read("skills/mobius-loop/SKILL.md"),
    ] {
        assert!(skill.contains("CSV"));
        assert!(skill.contains("request, open, parse, or use"));
    }
}

#[test]
fn copilot_is_the_only_objective_contract_skill_identity() {
    let skill = read("skills/mobius-copilot/SKILL.md");
    let interface = read("skills/mobius-copilot/agents/openai.yaml");

    assert!(skill.starts_with("---\nname: mobius-copilot\ndescription:"));
    assert!(interface.contains("display_name: \"Mobius Copilot\""));
    assert!(interface.contains("$mobius-copilot"));
    let forbidden_concepts = skill
        .split(|character: char| !character.is_ascii_alphabetic())
        .map(str::to_ascii_lowercase)
        .filter(|word| matches!(word.as_str(), "plan" | "planning"))
        .collect::<Vec<_>>();
    assert!(
        forbidden_concepts.is_empty(),
        "Copilot must not retain the former contract-skill concept: {forbidden_concepts:?}"
    );
}

#[test]
fn map_installation_ownership_follows_mapping_reason() {
    let copilot = read("skills/mobius-copilot/SKILL.md");
    let loop_skill = read("skills/mobius-loop/SKILL.md");

    let activate = copilot
        .split("### Activate a new Objective")
        .nth(1)
        .expect("Copilot omitted the activation branch")
        .split("### Revise the active Objective")
        .next()
        .expect("activation branch must be bounded");
    let revise = copilot
        .split("### Revise the active Objective")
        .nth(1)
        .expect("Copilot omitted the revision branch")
        .split("### Abandon the active Objective")
        .next()
        .expect("revision branch must be bounded");
    let abandon = copilot
        .split("### Abandon the active Objective")
        .nth(1)
        .expect("Copilot omitted the abandonment branch")
        .split("The Copilot is the sole Composition owner")
        .next()
        .expect("abandonment branch must be bounded");

    assert!(activate.contains("`ActivateObjective`"));
    assert!(activate.contains("accepted Mapping\ncontinuation above"));
    assert!(activate.contains("`MappingReason` to be `Initial`"));
    assert!(!activate.contains("`ReviseObjective`"));
    assert!(!activate.contains("`Abandon`"));

    assert!(revise.contains("`ReviseObjective`"));
    assert!(revise.contains("accepted Mapping\ncontinuation above"));
    assert!(revise.contains("`MappingReason` to be `SpecRevised`"));
    assert!(!revise.contains("`ActivateObjective`"));
    assert!(!revise.contains("`Abandon`"));

    assert!(abandon.contains("`Abandon`"));
    assert!(!abandon.contains("`ActivateObjective`"));
    assert!(!abandon.contains("`ReviseObjective`"));
    assert!(!abandon.contains("`InstallMap`"));
    assert!(copilot.contains("sole Composition owner"));
    assert!(copilot.contains("`$mobius-loop` owns Map installation for"));

    assert!(loop_skill.contains("$mobius-copilot"));
    assert!(loop_skill.contains("requires an already active Objective"));
    assert!(loop_skill.contains("For `Remap` or `WaitRevealedDrift`"));
    assert!(loop_skill.contains("submit `InstallMap` only when"));
    assert!(loop_skill.contains("For `Initial` or `SpecRevised`"));
    for transition in ["`ActivateObjective`", "`ReviseObjective`", "`Abandon`"] {
        assert!(
            !loop_skill.contains(transition),
            "Loop must not own the contract transition {transition}"
        );
    }
}

#[test]
fn copilot_resumes_durable_initial_and_spec_revised_mapping() {
    let copilot = read("skills/mobius-copilot/SKILL.md");
    let continuation = copilot
        .split("### Continue an accepted Mapping state")
        .nth(1)
        .expect("Copilot omitted the durable Mapping continuation entry")
        .split("### Activate a new Objective")
        .next()
        .expect("Mapping continuation entry must be bounded");

    assert!(continuation.contains(
        "both immediately after an accepted contract transition\nand after interruption"
    ));
    assert!(continuation.contains("`Initial`: shape and install the Map"));
    assert!(continuation.contains("`SpecRevised`: shape and install the replacement Map"));
    assert!(continuation.contains("reported next actions to permit `InstallMap`"));
    assert!(continuation.contains("do not submit `ActivateObjective` or `ReviseObjective`"));
    assert!(continuation.contains("do not ask the user to reconfirm"));
    assert!(continuation.contains("current typed ObjectiveSpec and heads read from\nCore"));
    assert!(copilot.contains(
        "For a stale `InstallMap`\ncontinuation, rebuild against the live Mapping state and heads without reconfirming"
    ));
}
