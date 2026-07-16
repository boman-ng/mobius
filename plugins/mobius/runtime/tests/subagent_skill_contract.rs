use std::fs;
use std::path::{Path, PathBuf};

fn plugin_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("runtime package must live directly under the plugin root")
        .to_path_buf()
}

fn skill_root() -> PathBuf {
    plugin_root().join("skills/mobius-subagent")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<String>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        .collect::<Result<Vec<_>, _>>()
        .expect("skill directory entries must be readable");
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .unwrap_or_else(|error| panic!("failed to inspect {}: {error}", path.display()));
        assert!(
            !metadata.file_type().is_symlink(),
            "skill package must not contain symlinks: {}",
            path.display()
        );
        if metadata.is_dir() {
            collect_files(root, &path, files);
        } else if metadata.is_file() {
            files.push(
                path.strip_prefix(root)
                    .expect("skill file must remain inside the skill root")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        } else {
            panic!(
                "skill package contains a non-file entry: {}",
                path.display()
            );
        }
    }
}

fn assert_contains_all(text: &str, context: &str, fragments: &[&str]) {
    for fragment in fragments {
        assert!(
            text.contains(fragment),
            "{context} is missing required contract fragment: {fragment}"
        );
    }
}

fn fenced_json_after<'a>(text: &'a str, heading: &str) -> &'a str {
    let section = text
        .split_once(heading)
        .unwrap_or_else(|| panic!("missing section heading: {heading}"))
        .1;
    let body = section
        .split_once("```json")
        .unwrap_or_else(|| panic!("missing JSON example after: {heading}"))
        .1;
    body.split_once("```")
        .unwrap_or_else(|| panic!("unterminated JSON example after: {heading}"))
        .0
}

fn collect_source_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
    {
        let path = entry
            .expect("source directory entry must be readable")
            .path();
        if path.is_dir() {
            collect_source_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn package_is_exactly_the_thin_instruction_surface() {
    let root = skill_root();
    assert!(root.is_dir(), "missing skill package: {}", root.display());

    let mut files = Vec::new();
    collect_files(&root, &root, &mut files);
    assert_eq!(
        files,
        [
            "SKILL.md",
            "agents/openai.yaml",
            "references/role-profiles.md"
        ],
        "the generic delegation package must remain one skill, host metadata, and one reference"
    );

    let skill = read(&root.join("SKILL.md"));
    assert!(skill.starts_with("---\nname: mobius-subagent\ndescription:"));
    assert_contains_all(
        &skill,
        "SKILL.md",
        &[
            "main agent determines that delegation is useful",
            "[role profiles](references/role-profiles.md)",
            "Do not create a worker ledger, queue, scheduler, registry, heartbeat, memory, transport, or Runtime mirror.",
        ],
    );

    let metadata = read(&root.join("agents/openai.yaml"));
    assert_eq!(
        metadata.matches("allow_implicit_invocation: true").count(),
        1,
        "Subagent discovery must remain available without explicit user invocation"
    );
    assert!(
        !metadata.contains("dependencies:"),
        "the independent Subagent skill must not acquire a downstream tool dependency"
    );
}

#[test]
fn package_has_no_model_core_path_api_or_schema_knowledge() {
    let root = skill_root();
    let package = format!(
        "{}\n{}\n{}",
        read(&root.join("SKILL.md")),
        read(&root.join("agents/openai.yaml")),
        read(&root.join("references/role-profiles.md"))
    );

    let forbidden_exact = [
        "Model Core",
        "Core API",
        "Core MCP",
        "ObjectiveState",
        "ObjectiveSpec",
        "MapRevision",
        "NavState",
        "ReviewPacket",
        "ReviewDecision",
        "WaitCondition",
        "FrozenEvidence",
        "CoreSnapshot",
        "SealAttempt",
        "ActivateObjective",
        "InstallMap",
    ];
    for token in forbidden_exact {
        assert!(
            !package.contains(token),
            "generic delegation package contains forbidden model/runtime knowledge: {token}"
        );
    }

    let lowercase = package.to_ascii_lowercase();
    for token in [
        ".mobius",
        "mobius.sqlite3",
        "plugins/mobius/runtime",
        "core mcp",
        "mobius_project_init",
        "mobius_apply_transition",
        "file://",
        "/home/",
        "/tmp/",
        "include_str!",
    ] {
        assert!(
            !lowercase.contains(token),
            "generic delegation package contains forbidden path or implementation knowledge: {token}"
        );
    }

    assert!(
        !package.contains("../"),
        "skill references must stay inside their one-level package"
    );

    let mut runtime_sources = vec![plugin_root().join("runtime/Cargo.toml")];
    collect_source_files(&plugin_root().join("runtime/src"), &mut runtime_sources);
    for path in runtime_sources {
        let source = read(&path).to_ascii_lowercase();
        for token in [
            "mobius-subagent",
            "role-profiles.md",
            "skills/mobius-subagent",
        ] {
            assert!(
                !source.contains(token),
                "runtime must not import or know the generic delegation package: {} contains {token}",
                path.display()
            );
        }
    }
}

#[test]
fn common_envelopes_preserve_required_semantics() {
    let skill = read(&skill_root().join("SKILL.md"));
    let basic = fenced_json_after(&skill, "## Build the task envelope");
    assert_contains_all(
        basic,
        "basic envelope",
        &[
            "\"role\"",
            "\"background\"",
            "\"why_now\"",
            "\"current_state\"",
            "\"confirmed_facts\"",
            "\"materials\"",
            "\"assumptions_to_check\"",
            "\"objectives\"",
            "\"boundaries\"",
            "\"forbidden\"",
            "\"focus\"",
            "\"role_input\"",
            "\"output_format\"",
            "\"done_when\"",
        ],
    );
    assert!(
        !basic.contains("\"allowed\""),
        "the canonical envelope must omit a positive allowlist by default"
    );

    let result = fenced_json_after(&skill, "## Require one public result");
    assert_contains_all(
        result,
        "public result",
        &[
            "\"status\"",
            "\"summary\"",
            "\"objective_results\"",
            "\"assumption_results\"",
            "\"done_when_results\"",
            "\"boundary_compliance\"",
            "\"effects\"",
            "\"artifacts\"",
            "\"uncertainties\"",
            "\"blockers\"",
            "\"role_output\"",
            "\"authorization\"",
            "authorized | unauthorized | ambiguous",
            "completed | partial | failed | rolled_back",
            "\"before\"",
            "\"after\"",
            "\"provenance\"",
            "\"verification\"",
            "\"unexpected\"",
            "\"residual_risks\"",
            "\"cleanup\"",
            "not_needed | completed | pending",
        ],
    );
    assert_eq!(result.matches("\"effects\": [").count(), 1);
    assert_eq!(result.matches("\"artifacts\": [").count(), 1);

    assert_contains_all(
        &skill,
        "common workflow",
        &[
            "Omit `allowed` by default.",
            "an `id`, `action`, `target`, and `constraints`",
            "host's officially supported native Subagent workflow",
            "Consume the native final output, items, status, and usage directly",
            "spawn, configuration, Runtime, and permission failures as failures",
            "Serialize Drivers whose modifications overlap",
            "Start a Verifier after the relevant effects have occurred and stabilized.",
            "Keep all IDs local to this task.",
            "Never turn model count, votes, Runtime success, or recommendations into automatic acceptance.",
            "Keep the result advisory and candidate-only.",
        ],
    );
}

#[test]
fn all_five_role_profiles_are_complete_without_parallel_inventories() {
    let profiles = read(&skill_root().join("references/role-profiles.md"));
    assert_contains_all(
        &profiles,
        "role profiles",
        &[
            "## Scout",
            "\"roots\"",
            "\"inspection_requests\"",
            "\"root_results\"",
            "\"inspection_results\"",
            "\"facts\"",
            "\"inferences\"",
            "## Researcher",
            "\"questions\"",
            "\"source_requirements\"",
            "\"sources\"",
            "\"answers\"",
            "\"source_conflicts\"",
            "## Driver",
            "\"change_targets\"",
            "\"validations\"",
            "\"target_results\"",
            "\"commands\"",
            "\"validation_results\"",
            "## Verifier",
            "\"subjects\"",
            "\"claims\"",
            "\"checks\"",
            "\"subject_results\"",
            "\"claim_results\"",
            "\"check_results\"",
            "## Judge",
            "\"materials\"",
            "\"freeze\"",
            "\"criteria\"",
            "\"known_risks\"",
            "\"material_results\"",
            "\"criterion_assessments\"",
            "\"risk_assessments\"",
            "\"recommended_disposition\"",
        ],
    );

    assert!(
        !profiles.contains("\"effects\": ["),
        "role profiles must reference the common effect inventory instead of copying it"
    );
    assert!(
        !profiles.contains("\"artifacts\": ["),
        "role profiles must reference the common artifact inventory instead of copying it"
    );
    assert!(
        !profiles.contains("\"changes\":"),
        "Driver must not create a parallel changes inventory"
    );
}

#[test]
fn judge_gate_closes_freeze_and_coverage_bypasses() {
    let profiles = read(&skill_root().join("references/role-profiles.md"));
    assert_contains_all(
        &profiles,
        "Judge contract",
        &[
            "inline | content_digest | immutable_version | immutable_object_id",
            "Freeze matched and required coverage complete",
            "Freeze matched but required coverage partial",
            "Freeze mismatched or stale",
            "Freeze unverifiable or material inaccessible",
            "Map a mismatched freeze to material status `stale`",
            "| `partial` | `inconclusive` | `inconclusive` |",
            "| `unverifiable` | `inconclusive` | `inconclusive` |",
            "Findings and recommendations cannot bypass the gate.",
            "Keep all outputs advisory.",
        ],
    );
}

#[test]
fn role_and_model_policy_pin_the_gpt_5_6_matrix_and_driver_inherits() {
    let skill = read(&skill_root().join("SKILL.md"));
    assert_contains_all(
        &skill,
        "role and model policy",
        &[
            "`scout`",
            "`researcher`",
            "`driver`",
            "`verifier`",
            "`judge`",
            "GPT-5.6 Luna / `medium`",
            "GPT-5.6 Terra / `medium`",
            "GPT-5.6 Luna / `high`",
            "GPT-5.6 Sol / `medium`",
            "Inherit the Main Agent's model and reasoning effort",
            "Apply this GPT-5.6 model matrix to default native Mobius spawns",
            "The different-family Judge path below is the only model-selection exception",
            "uses the model and effort configured by the Host",
            "report a configuration failure and stop instead of substituting another model or effort",
            "use the Host-configured `mobius-judge` custom agent to spawn the same `judge` role",
            "task envelope, Judge role profile, freeze gate, and result contract remain identical",
            "actual agent, model, and provider are Runtime execution facts",
            "treat the independent-perspective requirement as unmet",
            "Do not infer model family from the custom-agent name or provider",
            "instead of substituting the default Judge",
            "permission, model, provider, status, and usage objects as the only execution facts",
            "Select a role by its work function",
        ],
    );
    assert_eq!(
        skill.matches("| `judge` |").count(),
        1,
        "the package must expose one Judge role contract"
    );
    for forbidden in [
        "`judge` internal",
        "`judge` external",
        "External Judge",
        "Kimi",
        "judge_kind",
        "reviewer_kind",
        "review_route",
        "external independent review is outside this native role matrix",
    ] {
        assert!(
            !skill.contains(forbidden),
            "the Skill must not introduce a Judge variant or branded route: {forbidden}"
        );
    }
}
