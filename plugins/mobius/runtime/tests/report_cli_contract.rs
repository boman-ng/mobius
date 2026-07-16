use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use uuid::Uuid;

const REPORT_FILES: [&str; 9] = [
    "meta.csv",
    "overview.csv",
    "stage-view.csv",
    "criterion-view.csv",
    "route-view.csv",
    "attempt-view.csv",
    "evidence-view.csv",
    "review-view.csv",
    "timeline.csv",
];

struct TestProject {
    root: PathBuf,
}

impl TestProject {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mobius-report-cli-{}", Uuid::new_v4()));
        fs::create_dir(&root).expect("create test project");
        Self { root }
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn mobius() -> &'static str {
    env!("CARGO_BIN_EXE_mobius")
}

fn mcp_call(project_root: &Path, name: &str, arguments: Value) -> Value {
    let sandbox_cwd = url::Url::from_file_path(project_root)
        .expect("project root must be an absolute local path")
        .to_string();
    let mut child = Command::new(mobius())
        .arg("mcp")
        .current_dir(project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Mobius MCP");
    let messages = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "report-contract-test", "version": "1"}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments,
                "_meta": {
                    "codex/sandbox-state-meta": {
                        "sandboxCwd": sandbox_cwd
                    }
                }
            }
        }),
    ];
    {
        let stdin = child.stdin.as_mut().expect("MCP stdin");
        for message in messages {
            serde_json::to_writer(&mut *stdin, &message).expect("write MCP request");
            stdin.write_all(b"\n").expect("terminate MCP request");
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for Mobius MCP");
    assert!(
        output.status.success(),
        "MCP failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses = String::from_utf8(output.stdout)
        .expect("MCP output is UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("valid MCP response"))
        .collect::<Vec<_>>();
    let response = responses
        .iter()
        .find(|response| response["id"] == 2)
        .expect("tool response");
    assert_eq!(
        response["result"]["isError"], false,
        "tool call failed: {response}"
    );
    response["result"]["structuredContent"].clone()
}

fn initialize(project: &TestProject) -> String {
    mcp_call(
        &project.root,
        "mobius_project_init",
        json!({
            "project_root": project.root,
            "request_id": "report-test-bootstrap"
        }),
    )["project_id"]
        .as_str()
        .expect("project identity")
        .to_owned()
}

fn objective_spec(objective_id: &str) -> Value {
    json!({
        "objective": objective_id,
        "revision": 1,
        "intended_outcome": "publish a trustworthy human report",
        "criteria": {
            "criterion-report": {
                "id": "criterion-report",
                "statement": "the report is derived from the pinned Trail",
                "verification_rule": "compare the report heads and digest with Core",
                "scope": "local"
            }
        },
        "boundaries": ["local project only"],
        "excluded_claims": ["CSV is a business fact source"]
    })
}

fn apply(
    project: &TestProject,
    project_id: &str,
    project_seq: u64,
    objective_seq: u64,
    request_id: &str,
    command: Value,
) -> Value {
    mcp_call(
        &project.root,
        "mobius_apply_transition",
        json!({
            "project_root": project.root,
            "project_id": project_id,
            "expected_heads": {
                "expected_project_seq": project_seq,
                "expected_objective_seq": objective_seq
            },
            "request_id": request_id,
            "command": command
        }),
    )
}

fn activate(
    project: &TestProject,
    project_id: &str,
    objective_id: &str,
    expected_project_seq: u64,
    request_id: &str,
) -> Value {
    let spec = objective_spec(objective_id);
    apply(
        project,
        project_id,
        expected_project_seq,
        0,
        request_id,
        json!({
            "activate_objective": {
                "objective_spec": spec,
                "confirmation": {
                    "project": project_id,
                    "action": "activate",
                    "objective_spec": {"objective": objective_id, "revision": 1},
                    "confirmed_payload": objective_spec(objective_id),
                    "heads": {
                        "expected_project_seq": expected_project_seq,
                        "expected_objective_seq": 0
                    },
                    "confirmed": true
                }
            }
        }),
    )
}

fn abandon(
    project: &TestProject,
    project_id: &str,
    objective_id: &str,
    expected_project_seq: u64,
    expected_objective_seq: u64,
) -> Value {
    let reason = "replace the completed report fixture";
    apply(
        project,
        project_id,
        expected_project_seq,
        expected_objective_seq,
        "report-test-abandon-a",
        json!({
            "abandon": {
                "reason": reason,
                "confirmation": {
                    "project": project_id,
                    "objective": objective_id,
                    "reason": reason,
                    "heads": {
                        "expected_project_seq": expected_project_seq,
                        "expected_objective_seq": expected_objective_seq
                    },
                    "confirmed": true
                }
            }
        }),
    )
}

fn run_cli(project_root: &Path, arguments: &[&str]) -> Output {
    Command::new(mobius())
        .args(arguments)
        .current_dir(project_root)
        .output()
        .expect("run Mobius CLI")
}

fn report(
    project: &TestProject,
    project_id: &str,
    objective_id: &str,
    session_ref: &str,
    slug: &str,
) -> Value {
    let output = run_cli(
        &project.root,
        &["report", project_id, objective_id, session_ref, slug],
    );
    assert!(
        output.status.success(),
        "report failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("report publication JSON")
}

fn generation_path(publication: &Value) -> PathBuf {
    PathBuf::from(
        publication["generation_path"]
            .as_str()
            .expect("generation path"),
    )
}

fn read_status(project: &TestProject, project_id: &str, objective_id: &str) -> Vec<u8> {
    let query = json!({"kind": "status", "objective_id": objective_id}).to_string();
    let output = run_cli(&project.root, &["read", project_id, &query]);
    assert!(
        output.status.success(),
        "read failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn real_trail_reports_are_complete_deterministic_isolated_and_read_only_on_failure() {
    let project = TestProject::new();
    let project_id = initialize(&project);
    let objective_a = "same-prefix-objective-a";
    let objective_b = "same-prefix-objective-b";

    let activated = activate(
        &project,
        &project_id,
        objective_a,
        0,
        "report-test-activate-a",
    );
    assert_eq!(activated["committed_project_seq"], 1);
    assert_eq!(activated["committed_objective_seq"], 1);

    let first = report(
        &project,
        &project_id,
        objective_a,
        "session-one",
        "same-slug",
    );
    let second = report(
        &project,
        &project_id,
        objective_a,
        "session-one",
        "same-slug",
    );
    let first_generation = generation_path(&first);
    let second_generation = generation_path(&second);
    assert_ne!(first_generation, second_generation);

    let names = fs::read_dir(&first_generation)
        .expect("read report generation")
        .map(|entry| {
            entry
                .expect("report entry")
                .file_name()
                .into_string()
                .expect("UTF-8 report name")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(names, REPORT_FILES.map(str::to_owned).into());
    for filename in REPORT_FILES {
        assert_eq!(
            fs::read(first_generation.join(filename)).expect("first report file"),
            fs::read(second_generation.join(filename)).expect("second report file"),
            "same-head refresh changed semantic rows in {filename}"
        );
    }
    let first_meta = fs::read_to_string(first_generation.join("meta.csv")).expect("first meta");
    assert!(first_meta.contains(&format!("objective_id,{objective_a}\n")));
    let run_a = first_generation
        .parent()
        .and_then(Path::parent)
        .expect("Objective A run directory");

    let abandoned = abandon(&project, &project_id, objective_a, 1, 1);
    assert_eq!(abandoned["committed_project_seq"], 2);
    assert_eq!(abandoned["committed_objective_seq"], 2);
    let activated = activate(
        &project,
        &project_id,
        objective_b,
        2,
        "report-test-activate-b",
    );
    assert_eq!(activated["committed_project_seq"], 3);
    assert_eq!(activated["committed_objective_seq"], 1);

    let third = report(
        &project,
        &project_id,
        objective_b,
        "session-one",
        "same-slug",
    );
    let third_generation = generation_path(&third);
    let run_b = third_generation
        .parent()
        .and_then(Path::parent)
        .expect("Objective B run directory");
    assert_ne!(run_a, run_b, "full Objective identity must isolate runs");
    let third_meta = fs::read_to_string(third_generation.join("meta.csv")).expect("third meta");
    assert!(third_meta.contains(&format!("objective_id,{objective_b}\n")));
    assert!(!third_meta.contains(&format!("objective_id,{objective_a}\n")));

    // Force a renderer failure after report_snapshot has read Core: admission only owns the
    // views root, while this deliberately invalid session entry is presentation-owned.
    let invalid_session = project
        .root
        .join(".mobius/views/codex-session-failing-session");
    fs::write(&invalid_session, b"not a directory").expect("create invalid session entry");
    let database = project.root.join(".mobius/mobius.sqlite3");
    let database_before = fs::read(&database).expect("database before failed report");
    let status_before = read_status(&project, &project_id, objective_b);
    let failed = run_cli(
        &project.root,
        &[
            "report",
            &project_id,
            objective_b,
            "failing-session",
            "same-slug",
        ],
    );
    assert!(
        !failed.status.success(),
        "invalid report path must fail closed"
    );
    assert_eq!(
        fs::read(&database).expect("database after failed report"),
        database_before,
        "report failure mutated SQLite bytes"
    );
    assert_eq!(
        read_status(&project, &project_id, objective_b),
        status_before,
        "report failure changed the Core read model"
    );
}

#[cfg(unix)]
#[test]
fn explicit_cli_report_repairs_unsafe_current_entries_without_following_them() {
    use std::os::unix::fs::symlink;

    let project = TestProject::new();
    let project_id = initialize(&project);
    let objective_id = "repair-current-objective";
    activate(
        &project,
        &project_id,
        objective_id,
        0,
        "report-test-activate-repair",
    );
    let first = report(
        &project,
        &project_id,
        objective_id,
        "repair-session",
        "repair-view",
    );
    let current = PathBuf::from(first["current_path"].as_str().expect("current path"));
    let run = current.parent().expect("run directory");
    let generations = run.join("generations");
    assert_eq!(fs::read_dir(&generations).unwrap().count(), 1);

    fs::write(&current, [0xff]).unwrap();
    report(
        &project,
        &project_id,
        objective_id,
        "repair-session",
        "repair-view",
    );
    assert!(fs::symlink_metadata(&current).unwrap().is_file());
    assert_eq!(fs::read_dir(&generations).unwrap().count(), 2);

    let external = project.root.join("external-current-target.csv");
    let external_bytes = b"external target remains unchanged";
    fs::write(&external, external_bytes).unwrap();
    fs::remove_file(&current).unwrap();
    symlink(&external, &current).unwrap();

    report(
        &project,
        &project_id,
        objective_id,
        "repair-session",
        "repair-view",
    );

    assert!(fs::symlink_metadata(&current).unwrap().is_file());
    assert_eq!(fs::read(&external).unwrap(), external_bytes);
    assert_eq!(fs::read_dir(&generations).unwrap().count(), 3);
    assert!(fs::read_dir(run).unwrap().any(|entry| {
        let entry = entry.unwrap();
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(".invalid-current-"))
            && fs::symlink_metadata(entry.path())
                .unwrap()
                .file_type()
                .is_symlink()
    }));

    fs::remove_file(&current).unwrap();
    fs::create_dir(&current).unwrap();
    fs::write(current.join("sentinel"), b"preserve quarantined entry").unwrap();

    report(
        &project,
        &project_id,
        objective_id,
        "repair-session",
        "repair-view",
    );

    assert!(fs::symlink_metadata(&current).unwrap().is_file());
    assert_eq!(fs::read_dir(&generations).unwrap().count(), 4);
    assert!(fs::read_dir(run).unwrap().any(|entry| {
        let entry = entry.unwrap();
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(".invalid-current-"))
            && entry.path().join("sentinel").is_file()
    }));
}

#[test]
fn cli_rejects_business_mutation_modes_without_creating_project_state() {
    for mode in ["apply_transition", "activate", "mutate", "project_init"] {
        let project = TestProject::new();
        let output = run_cli(&project.root, &[mode]);
        assert!(
            !output.status.success(),
            "{mode} unexpectedly became a CLI mode"
        );
        assert!(
            !project.root.join(".mobius").exists(),
            "rejected CLI mode {mode} created project state"
        );
    }
}
