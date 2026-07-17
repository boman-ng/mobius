use std::fs;
use std::path::{Path, PathBuf};

const WRITE_TOOLS: [&str; 4] = [
    "mobius_project_init",
    "mobius_capture_artifact",
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

fn skills() -> [(String, String); 2] {
    [
        ("Copilot".to_owned(), read("skills/mobius-copilot/SKILL.md")),
        ("Loop".to_owned(), read("skills/mobius-loop/SKILL.md")),
    ]
}

#[test]
fn composition_requires_explicit_mobius_objective_invocation() {
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
        let frontmatter = skill.split("---").nth(1).expect("skill frontmatter");
        assert!(frontmatter.contains("explicitly"));
        assert!(frontmatter.contains("Mobius Objective"));
        assert!(read(metadata_path).contains("allow_implicit_invocation: false"));
    }
}

#[test]
fn composition_has_one_sql_read_path_and_four_mcp_write_tools() {
    for (role, skill) in skills() {
        for tool in WRITE_TOOLS {
            assert!(
                skill.contains(&format!("`{tool}`")),
                "{role} omitted {tool}"
            );
        }
        for removed in ["`mobius_read`", "`mobius_read_artifact`", "Agent ORM"] {
            assert!(!skill.contains(removed), "{role} retained {removed}");
        }
        for contract in [
            "3.40.1",
            "--safe --readonly --batch --bail --init /dev/null --line",
            "PRAGMA query_only=ON; BEGIN;",
            "sqlite_text(v)",
            "shell_word(sql)",
            r#"after replacing each ' with '"'"'"#,
            "Apply `shell_word` once, after the SQL is complete",
            "replace the whole `'<objective-id>'` token",
            "SELECT *",
            "finite `LIMIT`",
            "Re-read both heads",
            "untrusted data",
            "Read-only audit uses `mobius audit <project-id>`",
        ] {
            assert!(skill.contains(contract), "{role} omitted {contract}");
        }
    }
}

#[test]
fn loop_makes_formal_review_and_wait_completeness_explicit() {
    let skill = read("skills/mobius-loop/SKILL.md");
    assert!(skill.contains("references/wait-read.md"));
    assert!(skill.contains("references/review-read.md"));
    assert!(skill.contains("Do not load that recipe for other states"));
    assert!(skill.contains("exact review material"));
    assert!(skill.contains("A Decision is forbidden until exact row"));

    let wait_recipe = read("skills/mobius-loop/references/wait-read.md");
    for contract in [
        "WITH current_wait AS MATERIALIZED",
        "matching AS MATERIALIZED",
        "stats AS MATERIALIZED",
        "matching_count",
        "within_budget",
        "complete admitted set or none",
        "keeps the Objective `Waiting`",
        "no `LIMIT`",
    ] {
        assert!(
            wait_recipe.contains(contract),
            "Wait recipe omitted {contract}"
        );
    }
}

#[test]
fn delegated_lane_stays_optional_and_outside_managed_state() {
    let skill = read("skills/mobius-loop/SKILL.md");
    for contract in [
        "Do not call any Mobius MCP tool.",
        "Do not read or write `.mobius/` managed state.",
        "Do not impose a fixed role\nsequence or worker count",
        "candidate, never Evidence or Judgment",
    ] {
        assert!(skill.contains(contract), "Loop omitted {contract}");
    }
}

#[test]
fn ownership_views_and_completion_remain_bounded() {
    let copilot = read("skills/mobius-copilot/SKILL.md");
    let loop_skill = read("skills/mobius-loop/SKILL.md");

    for reason in [
        "`Initial`",
        "`SpecRevised`",
        "`Remap`",
        "`WaitRevealedDrift`",
    ] {
        assert!(copilot.contains(reason));
        assert!(loop_skill.contains(reason));
    }
    for skill in [&copilot, &loop_skill] {
        assert!(skill.contains("Reports and CSV files are presentation"));
    }
    assert!(!copilot.contains("MOBIUS_OBJECTIVE_ACHIEVED:"));
    assert_eq!(loop_skill.matches("MOBIUS_OBJECTIVE_ACHIEVED:").count(), 1);
    assert!(loop_skill.contains("read-only `mobius audit <project-id>` to be healthy"));
}

#[test]
fn copilot_clarifies_intent_and_loop_owns_every_route() {
    let copilot = read("skills/mobius-copilot/SKILL.md");
    let elicitation = read("skills/mobius-copilot/references/intent-elicitation.md");
    let loop_skill = read("skills/mobius-loop/SKILL.md");
    let interaction_read = read("skills/mobius-loop/references/interaction-read.md");

    for contract in [
        "references/intent-elicitation.md",
        "Ask one important question at a time",
        "not unquestionable decisions",
        "never asked to design a Map or Route",
        "`initial_routes` set to `{}`",
        "top-level `interaction` object",
        "`interaction_path`",
    ] {
        assert!(copilot.contains(contract), "Copilot omitted {contract}");
    }
    for contract in [
        "intended outcome",
        "observable criteria",
        "unresolved tensions",
        "Stop asking when the remaining uncertainty concerns only how to execute a Route",
        "not a transcript",
    ] {
        assert!(
            elicitation.contains(contract),
            "elicitation recipe omitted {contract}"
        );
    }
    for contract in [
        "design every Route",
        "All initial, added, and replacement Routes",
        "references/interaction-read.md",
        "Only while preparing an `AddRoute`",
        "current `SeekingRoute` Stage",
    ] {
        assert!(loop_skill.contains(contract), "Loop omitted {contract}");
    }
    for contract in [
        "exactly one path matches both",
        "fixed leading metadata block through `- Action:`",
        "current Core state + ObjectiveSpec + Map + StructuralContext",
        "> project facts reverified now",
        "> interaction.md background and hints",
        "cannot be Evidence, Judgment, Decision, proof, completion, Map recovery",
        "sole main-agent exception",
        "read another view as business input",
    ] {
        assert!(
            interaction_read.contains(contract),
            "interaction read recipe omitted {contract}"
        );
    }

    let ownership_contract = format!("{copilot}\n{elicitation}\n{loop_skill}");
    for contradiction in [
        "ask the human to design a Map",
        "ask the human to design a Route",
        "human confirms the Map",
        "human confirms the Route",
        "Copilot designs every Route",
        "Copilot designs the initial Route",
    ] {
        assert!(
            !ownership_contract.contains(contradiction),
            "Route ownership contract contains contradiction: {contradiction}"
        );
    }
}
