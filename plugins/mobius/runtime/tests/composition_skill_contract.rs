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

fn contains_prose(haystack: &str, needle: &str) -> bool {
    let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    normalize(haystack).contains(&normalize(needle))
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
    assert!(loop_skill.len() <= 7_472, "Loop hot path grew");
    assert!(
        copilot.len() + intent.len() <= 7_884 + 2_676,
        "activation path grew"
    );
    assert!(
        loop_skill.len() + review.len() <= 7_472 + 2_805,
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
    assert!(loop_skill.contains("After compaction, interruption, handoff"));
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
fn delegation_is_optional_fresh_and_outside_core_authority() {
    let loop_skill = read("skills/mobius-loop/SKILL.md");
    for contract in [
        "one bounded task has material value",
        "self-contained boundary",
        "baseline that will remain fresh",
        "Do not call any Mobius MCP tool.",
        "Do not read or write `.mobius/` managed state.",
        "candidates, never Evidence or Judgment",
        "Never pass a Core handle or mutation instruction",
        "one fresh Verifier",
        "never fix role order or worker count",
    ] {
        assert!(
            contains_prose(&loop_skill, contract),
            "Loop omitted {contract}"
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
