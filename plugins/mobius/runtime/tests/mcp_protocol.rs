use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use uuid::Uuid;

const PROTOCOL_VERSION: &str = "2025-11-25";
const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("mobius-mcp-test-{}", Uuid::new_v4()));
        fs::create_dir(&path).expect("temporary MCP workspace must be creatable");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct McpProcess {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl McpProcess {
    fn spawn(project_root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mobius"))
            .arg("mcp")
            .current_dir(project_root)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("MCP process must start");
        let stdin = child.stdin.take().expect("MCP stdin must be piped");
        let stdout = BufReader::new(child.stdout.take().expect("MCP stdout must be piped"));
        Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout,
        }
    }

    fn write_value(&mut self, value: &Value) {
        let stdin = self.stdin.as_mut().expect("MCP stdin remains open");
        serde_json::to_writer(&mut *stdin, value).expect("request must serialize");
        stdin.write_all(b"\n").expect("request must be writable");
        stdin.flush().expect("request must flush");
    }

    fn write_raw_line(&mut self, line: &[u8]) {
        let stdin = self.stdin.as_mut().expect("MCP stdin remains open");
        stdin
            .write_all(line)
            .expect("raw MCP frame must be writable");
        stdin
            .write_all(b"\n")
            .expect("raw MCP frame must terminate");
        stdin.flush().expect("raw MCP frame must flush");
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.write_value(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }));
        let response = self.read_response();
        assert_eq!(response["id"], id, "notifications must not shift responses");
        response
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.write_value(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }));
    }

    fn read_response(&mut self) -> Value {
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .expect("MCP stdout must be readable");
        assert_ne!(read, 0, "MCP process closed before returning a response");
        let response: Value =
            serde_json::from_str(&line).expect("every MCP stdout line must be JSON");
        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response.is_object(), "an MCP response must be an object");
        response
    }

    fn initialize(&mut self) -> Value {
        self.request(
            1,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "mobius-protocol-test", "version": "1"}
            }),
        )
    }

    fn finish(mut self) {
        drop(self.stdin.take());

        let mut trailing_stdout = String::new();
        self.stdout
            .read_to_string(&mut trailing_stdout)
            .expect("remaining MCP stdout must be readable");

        let mut child = self.child.take().expect("MCP child remains owned");
        let status = child.wait().expect("MCP process must terminate at EOF");
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .expect("MCP stderr must be piped")
            .read_to_string(&mut stderr)
            .expect("MCP stderr must be readable");

        assert!(status.success(), "MCP process failed: {stderr}");
        assert!(
            trailing_stdout.is_empty(),
            "MCP emitted an unexpected stdout frame: {trailing_stdout:?}"
        );
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn assert_tool_success(response: &Value) -> &Value {
    assert!(response.get("error").is_none());
    assert_eq!(response["result"]["isError"], false);
    let structured = &response["result"]["structuredContent"];
    assert_eq!(response["result"]["content"][0]["type"], "text");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text must be a string");
    assert!(text.len() < 128, "tool text must remain a compact hint");
    assert!(!text.contains(&structured.to_string()));
    structured
}

fn sandbox_metadata(sandbox_cwd: &Path) -> Value {
    let sandbox_cwd = url::Url::from_file_path(sandbox_cwd)
        .expect("sandbox cwd must be an absolute local path")
        .to_string();
    json!({
        "codex/sandbox-state-meta": {
            "permissionProfile": null,
            "sandboxPolicy": {"type": "danger-full-access"},
            "codexLinuxSandboxExe": null,
            "sandboxCwd": sandbox_cwd,
            "useLegacyLandlock": false
        }
    })
}

fn call_tool(
    process: &mut McpProcess,
    id: u64,
    name: &str,
    arguments: Value,
    sandbox_cwd: &Path,
) -> Value {
    call_tool_with_thread(process, id, name, arguments, sandbox_cwd, None)
}

fn call_tool_with_thread(
    process: &mut McpProcess,
    id: u64,
    name: &str,
    arguments: Value,
    sandbox_cwd: &Path,
    thread_id: Option<&str>,
) -> Value {
    let mut metadata = sandbox_metadata(sandbox_cwd);
    if let Some(thread_id) = thread_id {
        metadata
            .as_object_mut()
            .expect("sandbox metadata is an object")
            .insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
    }
    process.request(
        id,
        "tools/call",
        json!({
            "name": name,
            "arguments": arguments,
            "_meta": metadata
        }),
    )
}

fn readonly_connection(root: &Path) -> Connection {
    let connection = Connection::open_with_flags(
        root.join(".mobius/mobius.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open Mobius test database read-only");
    connection
        .pragma_update(None, "query_only", true)
        .expect("enable query_only for test observation");
    connection
}

fn projected_object(root: &Path, objective: &str, kind: &str, id: &str) -> Value {
    let connection = readonly_connection(root);
    let mut statement = connection
        .prepare(
            "SELECT projection_bytes FROM object_projection
             WHERE objective_id = ?1 AND object_kind = ?2 ORDER BY object_id",
        )
        .expect("prepare projected object read");
    statement
        .query_map([objective, kind], |row| row.get::<_, Vec<u8>>(0))
        .expect("query projected objects")
        .map(|row| serde_json::from_slice::<Value>(&row.expect("read projected object")).unwrap())
        .find(|object| object[kind]["id"] == id)
        .unwrap_or_else(|| panic!("missing {kind} projection {id}"))
}

fn objective_projection(root: &Path, objective: &str) -> Value {
    let bytes = readonly_connection(root)
        .query_row(
            "SELECT projection_bytes FROM objective_projection WHERE objective_id = ?1",
            [objective],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .expect("read Objective projection");
    serde_json::from_slice(&bytes).expect("decode Objective projection")
}

fn current_review_packet(root: &Path, objective: &str) -> Value {
    let projection = objective_projection(root, objective);
    let packet_id =
        projection["objective_state"]["navigating"]["navigation"]["reviewing"]["packet"]
            .as_str()
            .expect("Reviewing projection contains a Packet id");
    projected_object(root, objective, "review_packet", packet_id)
}

fn readonly_audit(root: &Path, project_id: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_mobius"))
        .args(["audit", project_id])
        .current_dir(root)
        .env_clear()
        .output()
        .expect("start read-only audit CLI");
    assert!(
        output.status.success(),
        "read-only audit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("decode audit CLI output")
}

fn objective_spec(objective_id: &str, revision: u64) -> Value {
    json!({
        "objective": objective_id,
        "revision": revision,
        "intended_outcome": "prove automatic post-commit reports",
        "criteria": {
            "criterion-auto-report": {
                "id": "criterion-auto-report",
                "statement": "the report follows the committed Trail",
                "verification_rule": "compare current.csv with Core heads",
                "scope": "local"
            }
        },
        "boundaries": ["local project only"],
        "excluded_claims": ["reports are business state"]
    })
}

fn activation_arguments(
    root: &Path,
    project_id: &str,
    objective_id: &str,
    request_id: &str,
) -> Value {
    json!({
        "project_root": root,
        "project_id": project_id,
        "expected_heads": {"expected_project_seq": 0, "expected_objective_seq": 0},
        "request_id": request_id,
        "command": {
            "activate_objective": {
                "objective_spec": objective_spec(objective_id, 1),
                "confirmation": {
                    "project": project_id,
                    "action": "activate",
                    "objective_spec": {"objective": objective_id, "revision": 1},
                    "confirmed_payload": objective_spec(objective_id, 1),
                    "heads": {"expected_project_seq": 0, "expected_objective_seq": 0},
                    "confirmed": true
                }
            }
        }
    })
}

fn interaction_summary(label: &str) -> Value {
    json!({
        "interpreted_intent": format!("understand {label}"),
        "confirmed_boundaries": "- local project only",
        "verified_facts": "- verified from current source",
        "challenges_and_resolutions": "- separated the outcome from a candidate tactic",
        "route_notes": format!("- investigate {label} while designing the Route")
    })
}

fn core_receipt(value: &Value) -> Value {
    json!({
        "objective_id": value["objective_id"],
        "transition": value["transition"],
        "committed_project_seq": value["committed_project_seq"],
        "committed_objective_seq": value["committed_objective_seq"],
        "event_digest": value["event_digest"]
    })
}

fn automatic_report_map(objective_id: &str, revision: u64, include_followup: bool) -> Value {
    let primary_stage_id = "stage-auto-report-primary";
    let primary_criterion_id = "criterion-auto-report";
    let followup_stage_id = "stage-auto-report-followup";
    let followup_criterion_id = "criterion-auto-report-followup";
    let primary_contract = json!({
        "outcome": "the primary report fact is accepted",
        "criteria": [primary_criterion_id],
        "objective_boundaries": ["local project only"],
        "output": "accepted primary report fact"
    });
    let followup_contract = json!({
        "outcome": "the follow-up remains incomplete before remap",
        "criteria": [followup_criterion_id],
        "objective_boundaries": ["local project only"],
        "output": "unfinished follow-up"
    });

    let mut stages = serde_json::Map::from_iter([(
        primary_stage_id.to_owned(),
        json!({
            "id": primary_stage_id,
            "name": "Primary automatic report Stage",
            "outcome": "the primary report fact is accepted",
            "output": "accepted primary report fact",
            "kind": "ordinary"
        }),
    )]);
    let mut criteria = serde_json::Map::from_iter([(
        primary_criterion_id.to_owned(),
        objective_spec(objective_id, 1)["criteria"][primary_criterion_id].clone(),
    )]);
    let mut priorities = serde_json::Map::from_iter([(primary_stage_id.to_owned(), json!(1))]);
    let mut owners =
        serde_json::Map::from_iter([(primary_criterion_id.to_owned(), json!(primary_stage_id))]);
    let mut contracts =
        serde_json::Map::from_iter([(primary_stage_id.to_owned(), primary_contract)]);

    if include_followup {
        stages.insert(
            followup_stage_id.to_owned(),
            json!({
                "id": followup_stage_id,
                "name": "Follow-up automatic report Stage",
                "outcome": "the follow-up remains incomplete before remap",
                "output": "unfinished follow-up",
                "kind": "ordinary"
            }),
        );
        criteria.insert(
            followup_criterion_id.to_owned(),
            json!({
                "id": followup_criterion_id,
                "statement": "the follow-up is observable",
                "verification_rule": "inspect the follow-up",
                "scope": "local"
            }),
        );
        priorities.insert(followup_stage_id.to_owned(), json!(2));
        owners.insert(followup_criterion_id.to_owned(), json!(followup_stage_id));
        contracts.insert(followup_stage_id.to_owned(), followup_contract);
    }

    json!({
        "objective_spec": {"objective": objective_id, "revision": 1},
        "revision": revision,
        "stages": stages,
        "criteria": criteria,
        "dependencies": [],
        "priorities": priorities,
        "owners": owners,
        "contracts": contracts
    })
}

fn sole_auto_run(root: &Path, thread_id: &str) -> PathBuf {
    let runs = root
        .join(".mobius/views")
        .join(format!("codex-session-{thread_id}"))
        .join("runs");
    let entries = fs::read_dir(&runs)
        .expect("automatic session runs must exist")
        .map(|entry| entry.expect("run entry").path())
        .collect::<Vec<_>>();
    assert_eq!(
        entries.len(),
        1,
        "one Objective must have one automatic run"
    );
    entries.into_iter().next().unwrap()
}

fn generation_count(run: &Path) -> usize {
    fs::read_dir(run.join("generations"))
        .expect("generation directory must exist")
        .count()
}

fn singleton_object(key: &str, value: Value) -> Value {
    let mut object = serde_json::Map::new();
    object.insert(key.to_owned(), value);
    Value::Object(object)
}

fn e2e_objective_spec(objective_id: &str, criterion_id: &str) -> Value {
    let criterion = json!({
        "id": criterion_id,
        "statement": "the inspected observation satisfies the bounded local outcome",
        "verification_rule": "inspect the frozen observation and its provenance",
        "scope": "local"
    });
    json!({
        "objective": objective_id,
        "revision": 1,
        "intended_outcome": "reach Achieved through the public MCP contract",
        "criteria": singleton_object(criterion_id, criterion),
        "boundaries": ["local fixture only"],
        "excluded_claims": ["a delegated result is already Core Evidence"]
    })
}

fn e2e_contract(criterion_id: &str) -> Value {
    json!({
        "outcome": "one inspected observation is accepted",
        "criteria": [criterion_id],
        "objective_boundaries": ["local fixture only"],
        "output": "frozen local observation"
    })
}

fn e2e_map(objective_id: &str, criterion_id: &str, stage_id: &str, revision: u64) -> Value {
    let criterion =
        e2e_objective_spec(objective_id, criterion_id)["criteria"][criterion_id].clone();
    let stage = json!({
        "id": stage_id,
        "name": "inspect one candidate",
        "outcome": "one inspected observation is accepted",
        "output": "frozen local observation",
        "kind": "ordinary"
    });
    json!({
        "objective_spec": {"objective": objective_id, "revision": 1},
        "revision": revision,
        "stages": singleton_object(stage_id, stage),
        "criteria": singleton_object(criterion_id, criterion),
        "dependencies": [],
        "priorities": singleton_object(stage_id, json!(1)),
        "owners": singleton_object(criterion_id, json!(stage_id)),
        "contracts": singleton_object(stage_id, e2e_contract(criterion_id))
    })
}

fn apply_e2e_transition(
    process: &mut McpProcess,
    rpc_id: u64,
    root: &Path,
    project_id: &str,
    expected_seq: u64,
    request_id: &str,
    command: Value,
) -> Value {
    let response = call_tool(
        process,
        rpc_id,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {
                "expected_project_seq": expected_seq,
                "expected_objective_seq": expected_seq
            },
            "request_id": request_id,
            "command": command
        }),
        root,
    );
    let result = assert_tool_success(&response).clone();
    assert_eq!(result["committed_project_seq"], expected_seq + 1);
    assert_eq!(result["committed_objective_seq"], expected_seq + 1);
    result
}

struct AttemptingE2e {
    project_id: String,
    objective_id: String,
    criterion_id: String,
    stage_id: String,
    route_id: String,
    attempt_id: String,
    context: Value,
    heads: (u64, u64),
    next_rpc_id: u64,
}

fn advance_public_mcp_to_attempting(
    process: &mut McpProcess,
    root: &Path,
    lane: &str,
) -> AttemptingE2e {
    process.initialize();
    process.notify("notifications/initialized", json!({}));

    let initialized = call_tool(
        process,
        2,
        "mobius_project_init",
        json!({"project_root": root, "request_id": format!("{lane}-project-init")}),
        root,
    );
    let project_id = assert_tool_success(&initialized)["project_id"]
        .as_str()
        .expect("project_init returns a project id")
        .to_owned();
    let objective_id = format!("objective-{lane}");
    let criterion_id = format!("criterion-{lane}");
    let stage_id = format!("stage-{lane}");
    let route_id = format!("route-{lane}");
    let attempt_id = format!("attempt-{lane}");
    let specification = e2e_objective_spec(&objective_id, &criterion_id);

    apply_e2e_transition(
        process,
        3,
        root,
        &project_id,
        0,
        &format!("{lane}-activate"),
        json!({
            "activate_objective": {
                "objective_spec": specification,
                "confirmation": {
                    "project": project_id,
                    "action": "activate",
                    "objective_spec": {"objective": objective_id, "revision": 1},
                    "confirmed_payload": e2e_objective_spec(&objective_id, &criterion_id),
                    "heads": {"expected_project_seq": 0, "expected_objective_seq": 0},
                    "confirmed": true
                }
            }
        }),
    );

    let map = e2e_map(&objective_id, &criterion_id, &stage_id, 1);
    apply_e2e_transition(
        process,
        4,
        root,
        &project_id,
        1,
        &format!("{lane}-install-map"),
        json!({
            "install_map": {
                "map": map,
                "initial_routes": {},
                "cover": {
                    "map": {"objective": objective_id, "revision": 1},
                    "objective_spec": {"objective": objective_id, "revision": 1},
                    "verdict": "covered",
                    "rationale": "the single Stage owns the complete Criterion domain"
                },
                "carry": {}
            }
        }),
    );

    let structural_context = json!({
        "contract": e2e_contract(&criterion_id),
        "dependencies": {}
    });
    apply_e2e_transition(
        process,
        5,
        root,
        &project_id,
        2,
        &format!("{lane}-add-route"),
        json!({
            "add_route": {
                "route": {
                    "id": route_id,
                    "stage": stage_id,
                    "structural_context": structural_context,
                    "hypothesis": "one inspected observation is enough for review",
                    "assumptions": ["the local fixture remains stable"],
                    "rationale": "bounded public MCP E2E route"
                }
            }
        }),
    );
    apply_e2e_transition(
        process,
        6,
        root,
        &project_id,
        3,
        &format!("{lane}-select-route"),
        json!({"select_route": {"route": route_id}}),
    );

    let route = projected_object(root, &objective_id, "route", &route_id);
    let context = json!({
        "structural": route["route"]["structural_context"],
        "dependency_proofs": {}
    });

    apply_e2e_transition(
        process,
        8,
        root,
        &project_id,
        4,
        &format!("{lane}-start-attempt"),
        json!({
            "start_attempt": {
                "attempt": {
                    "id": attempt_id,
                    "route": route_id,
                    "ordinal": 1,
                    "bound": {"termination_condition": "one inspected observation is frozen"},
                    "context": context
                }
            }
        }),
    );

    AttemptingE2e {
        project_id,
        objective_id,
        criterion_id,
        stage_id,
        route_id,
        attempt_id,
        context,
        heads: (5, 5),
        next_rpc_id: 9,
    }
}

fn record_evidence_command(
    fixture: &AttemptingE2e,
    evidence_id: &str,
    observation: &str,
    provenance: Value,
) -> Value {
    json!({
        "record_evidence": {
            "evidence": {
                "id": evidence_id,
                "subject": {"attempt": fixture.attempt_id},
                "context": fixture.context,
                "purpose": "stage_review",
                "claims": singleton_object(&fixture.criterion_id, json!("supports")),
                "observation": {"inline": {"string": observation}},
                "provenance": provenance
            }
        }
    })
}

struct ReviewingE2e {
    attempting: AttemptingE2e,
    packet_id: String,
    next_rpc_id: u64,
}

fn advance_public_mcp_to_reviewing(
    process: &mut McpProcess,
    root: &Path,
    lane: &str,
) -> ReviewingE2e {
    let attempting = advance_public_mcp_to_attempting(process, root, lane);
    apply_e2e_transition(
        process,
        attempting.next_rpc_id,
        root,
        &attempting.project_id,
        5,
        &format!("{lane}-record-evidence"),
        record_evidence_command(
            &attempting,
            &format!("evidence-{lane}"),
            "one bounded observation is ready for branch review",
            json!({"string": "public MCP review-branch fixture"}),
        ),
    );
    apply_e2e_transition(
        process,
        attempting.next_rpc_id + 1,
        root,
        &attempting.project_id,
        6,
        &format!("{lane}-seal"),
        json!({
            "seal_attempt": {
                "attempt": attempting.attempt_id,
                "seal_reason": "submitted"
            }
        }),
    );

    let packet = current_review_packet(root, &attempting.objective_id);
    assert_eq!(packet["review_packet"]["attempt"], attempting.attempt_id);
    assert_eq!(
        packet["review_packet"]["evidence_set"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let packet_id = packet["review_packet"]["id"]
        .as_str()
        .expect("ReviewPacket projection contains an id")
        .to_owned();

    ReviewingE2e {
        attempting,
        packet_id,
        next_rpc_id: 12,
    }
}

fn review_branch_decision_command(
    fixture: &ReviewingE2e,
    decision_id: &str,
    action: Value,
) -> Value {
    json!({
        "decision": {
            "decision": {
                "id": decision_id,
                "packet": fixture.packet_id,
                "judgments": singleton_object(
                    &fixture.attempting.criterion_id,
                    json!("unknown")
                ),
                "findings": ["the current observation does not yet justify acceptance"],
                "action": action
            }
        }
    })
}

fn start_second_e2e_attempt(
    process: &mut McpProcess,
    rpc_id: u64,
    root: &Path,
    fixture: &AttemptingE2e,
    expected_seq: u64,
) {
    let started = apply_e2e_transition(
        process,
        rpc_id,
        root,
        &fixture.project_id,
        expected_seq,
        &format!("{}-start-attempt-2", fixture.objective_id),
        json!({
            "start_attempt": {
                "attempt": {
                    "id": format!("{}-2", fixture.attempt_id),
                    "route": fixture.route_id,
                    "ordinal": 2,
                    "bound": {
                        "termination_condition": "one replacement observation is frozen"
                    },
                    "context": fixture.context
                }
            }
        }),
    );
    assert_eq!(started["transition"], "start_attempt");
}

fn assert_public_route_rejected(
    process: &mut McpProcess,
    rpc_id: u64,
    root: &Path,
    fixture: &AttemptingE2e,
    expected_seq: u64,
    request_id: &str,
) {
    let rejected = call_tool(
        process,
        rpc_id,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": fixture.project_id,
            "expected_heads": {
                "expected_project_seq": expected_seq,
                "expected_objective_seq": expected_seq
            },
            "request_id": request_id,
            "command": {"select_route": {"route": fixture.route_id}}
        }),
        root,
    );
    assert!(rejected.get("error").is_none());
    assert_eq!(rejected["result"]["isError"], true);
    assert_eq!(
        rejected["result"]["structuredContent"]["code"],
        "transition_rejected"
    );
    assert!(
        rejected["result"]["structuredContent"]["message"]
            .as_str()
            .expect("transition error contains a message")
            .contains("route_rejected"),
        "the prior Route must be rejected at the public Core boundary"
    );
}

fn install_second_e2e_map(
    process: &mut McpProcess,
    rpc_id: u64,
    root: &Path,
    fixture: &AttemptingE2e,
    expected_seq: u64,
) {
    let installed = apply_e2e_transition(
        process,
        rpc_id,
        root,
        &fixture.project_id,
        expected_seq,
        &format!("{}-install-map-2", fixture.objective_id),
        json!({
            "install_map": {
                "map": e2e_map(
                    &fixture.objective_id,
                    &fixture.criterion_id,
                    &fixture.stage_id,
                    2
                ),
                "initial_routes": {},
                "cover": {
                    "map": {"objective": fixture.objective_id, "revision": 2},
                    "objective_spec": {"objective": fixture.objective_id, "revision": 1},
                    "verdict": "covered",
                    "rationale": "the replacement Map preserves complete Criterion ownership"
                },
                "carry": {}
            }
        }),
    );
    assert_eq!(installed["transition"], "install_map");
}

fn finish_public_mcp_achieved(
    process: &mut McpProcess,
    root: &Path,
    fixture: &AttemptingE2e,
    first_rpc_id: u64,
) {
    apply_e2e_transition(
        process,
        first_rpc_id,
        root,
        &fixture.project_id,
        6,
        &format!("{}-seal", fixture.objective_id),
        json!({
            "seal_attempt": {
                "attempt": fixture.attempt_id,
                "seal_reason": "submitted"
            }
        }),
    );

    let packet = current_review_packet(root, &fixture.objective_id);
    let packet_id = packet["review_packet"]["id"]
        .as_str()
        .expect("ReviewPacket projection contains an id");
    assert_eq!(packet["review_packet"]["attempt"], fixture.attempt_id);
    assert_eq!(
        packet["review_packet"]["evidence_set"]
            .as_array()
            .map(Vec::len),
        Some(1),
        "Core materializes exactly the admitted Evidence universe"
    );

    apply_e2e_transition(
        process,
        first_rpc_id + 2,
        root,
        &fixture.project_id,
        7,
        &format!("{}-accept", fixture.objective_id),
        json!({
            "decision": {
                "decision": {
                    "id": format!("decision-{}", fixture.objective_id),
                    "packet": packet_id,
                    "judgments": singleton_object(&fixture.criterion_id, json!("satisfied")),
                    "findings": [],
                    "action": "accept"
                }
            }
        }),
    );

    let status = objective_projection(root, &fixture.objective_id);
    assert_eq!(
        status["objective_state"]["achieved"]["objective"],
        fixture.objective_id
    );

    let audit = readonly_audit(root, &fixture.project_id);
    assert_eq!(audit["status"], "healthy");
    assert_eq!(audit["project_seq"], 8);
    assert_eq!(audit["checked_objectives"], 1);
    assert_eq!(audit["issues"]["items"], json!([]));
    assert_eq!(audit["issues"]["complete"], true);
}

const NO_CORE_MCP_RULE: &str = "Do not call any Mobius MCP tool.";
const NO_MANAGED_STATE_RULE: &str = "Do not read or write `.mobius/` managed state.";
const CORE_MCP_BOUNDARY_SIGNAL: &str = "Mobius MCP";
const MANAGED_STATE_BOUNDARY_SIGNAL: &str = ".mobius/";

fn full_verifier_delegation_task() -> Value {
    json!({
        "role": "verifier",
        "background": {
            "why_now": "independently inspect one bounded local observation",
            "current_state": ["the main agent has started one bounded attempt"],
            "confirmed_facts": [{
                "id": "BF1",
                "fact": "the supplied fixture is the only verification subject",
                "evidence": ["main-agent fixture inspection"]
            }],
            "materials": [{
                "id": "BM1",
                "locator": "inline local fixture",
                "purpose": "provide the immutable verification subject"
            }],
            "assumptions_to_check": [{
                "id": "BA1",
                "assumption": "the fixture contains the expected marker"
            }]
        },
        "objectives": [{
            "id": "O1",
            "objective": "return the directly observed marker with checkable provenance",
            "priority": "must"
        }],
        "boundaries": {
            "forbidden": [
                {"id": "F1", "rule": NO_CORE_MCP_RULE, "reason": "only the main agent owns business mutations"},
                {"id": "F2", "rule": NO_MANAGED_STATE_RULE, "reason": "managed state is private to Core"}
            ],
            "focus": [{
                "id": "FO1",
                "target": "inline local fixture",
                "purpose": "inspect only the supplied verification subject"
            }]
        },
        "role_input": {
            "subjects": [{"id": "VS1", "subject": "inline local fixture"}],
            "claims": [{"id": "VC1", "claim": "the fixture contains the expected marker"}],
            "checks": [{
                "id": "VK1",
                "check": "read the marker from the supplied fixture",
                "subject_ids": ["VS1"],
                "claim_ids": ["VC1"],
                "expected": "candidate observation from delegated verifier",
                "counterexample": "the expected marker is absent"
            }],
            "environment": [{"id": "VE1", "condition": "current host-native delegated thread", "required": true}]
        },
        "output_format": {
            "representation": "json",
            "template": "the complete common result envelope plus the complete verifier role_output",
            "constraints": ["return direct observations and native-item provenance; redact unrelated data"]
        },
        "done_when": [{
            "id": "D1",
            "condition": "the check returns one observed marker or an explicit blocker",
            "evidence_required": ["native item provenance"]
        }]
    })
}

fn full_verifier_result(runtime_identity: &str) -> Value {
    json!({
        "status": "completed",
        "summary": "the verifier returned one directly inspected marker",
        "objective_results": [{
            "objective_id": "O1",
            "status": "achieved",
            "result": "the expected marker was observed",
            "evidence": [runtime_identity]
        }],
        "assumption_results": [{
            "assumption_id": "BA1",
            "assessment": "confirmed",
            "impact": "the candidate observation can be inspected by the main agent",
            "evidence": [runtime_identity]
        }],
        "done_when_results": [{
            "done_when_id": "D1",
            "status": "satisfied",
            "evidence": [runtime_identity],
            "reason": "the native item contains the direct check result"
        }],
        "boundary_compliance": {"status": "compliant", "violations": []},
        "effects": [],
        "artifacts": [],
        "uncertainties": [],
        "blockers": [],
        "role_output": {
            "subject_results": [{"subject_id": "VS1", "status": "verified", "evidence": [runtime_identity]}],
            "claim_results": [{"claim_id": "VC1", "assessment": "supports", "evidence": [runtime_identity]}],
            "check_results": [{
                "check_id": "VK1",
                "status": "passed",
                "actual": "candidate observation from delegated verifier",
                "environment_ids": ["VE1"],
                "evidence": [runtime_identity]
            }],
            "discrepancies": [],
            "gaps": []
        }
    })
}

fn delegated_e2e_inputs() -> (String, Value, Value) {
    let runtime_identity = std::env::var("MOBIUS_NATIVE_RUNTIME_IDENTITY").ok();
    let task = std::env::var("MOBIUS_NATIVE_TASK_JSON").ok();
    let result = std::env::var("MOBIUS_NATIVE_RESULT_JSON").ok();
    match (runtime_identity, task, result) {
        (None, None, None) => {
            let runtime_identity = "native-test-thread-1/item-1".to_owned();
            let task = full_verifier_delegation_task();
            let result = full_verifier_result(&runtime_identity);
            (runtime_identity, task, result)
        }
        (Some(runtime_identity), Some(task), Some(result)) => (
            runtime_identity,
            serde_json::from_str(&task).expect("native delegation task must be valid JSON"),
            serde_json::from_str(&result).expect("native delegation result must be valid JSON"),
        ),
        _ => panic!("native delegation evidence requires identity, task, and result together"),
    }
}

fn delegated_effect(runtime_identity: &str) -> Value {
    json!({
        "id": "E1",
        "target_ref": "VS1",
        "target": "temporary verifier fixture",
        "operation": "executed",
        "authorization": {"status": "authorized", "refs": ["O1", "VS1"]},
        "status": "completed",
        "before": "temporary fixture available",
        "after": "temporary verification command returned",
        "provenance": [runtime_identity, "temporary verifier command"],
        "verification": ["the command result was inspected"],
        "unexpected": [],
        "residual_risks": [],
        "cleanup": {
            "status": "completed",
            "reason": "temporary fixture was removed",
            "responsible": "main agent",
            "evidence": [runtime_identity]
        }
    })
}

fn has_exact_fields(value: &Value, fields: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
    })
}

fn task_has_forbidden_rule(task: &Value, signal: &str) -> bool {
    task["boundaries"]["forbidden"]
        .as_array()
        .is_some_and(|rules| {
            rules
                .iter()
                .filter_map(|candidate| candidate["rule"].as_str())
                .any(|rule| rule.contains(signal))
        })
}

fn result_covers_task_items(
    task_items: &Value,
    task_id: &str,
    result_items: &Value,
    result_id: &str,
    status_field: &str,
    accepted_status: &[&str],
) -> bool {
    let Some(task_items) = task_items.as_array() else {
        return false;
    };
    let Some(result_items) = result_items.as_array() else {
        return false;
    };
    task_items.len() == result_items.len()
        && task_items.iter().all(|task_item| {
            let Some(id) = task_item[task_id].as_str() else {
                return false;
            };
            result_items.iter().any(|result| {
                result[result_id] == id
                    && result[status_field]
                        .as_str()
                        .is_some_and(|status| accepted_status.contains(&status))
            })
        })
}

// This is deliberately test-local executable Composition evidence. It exercises the documented
// main-agent checks without introducing a production parser, shared schema, or worker ledger.
fn consume_full_delegated_result<F>(
    task: &Value,
    result: &Value,
    runtime_identity: &str,
    delegated_heads: (u64, u64),
    fixture: &AttemptingE2e,
    evidence_id: &str,
    submit: F,
) -> Result<Value, &'static str>
where
    F: FnOnce(Value) -> Value,
{
    if delegated_heads != fixture.heads {
        return Err("stale_baseline");
    }
    if !task_has_forbidden_rule(task, CORE_MCP_BOUNDARY_SIGNAL) {
        return Err("missing_forbidden_boundary_f1");
    }
    if !task_has_forbidden_rule(task, MANAGED_STATE_BOUNDARY_SIGNAL) {
        return Err("missing_forbidden_boundary_f2");
    }
    if runtime_identity.trim().is_empty() {
        return Err("runtime_identity_missing");
    }
    if !has_exact_fields(
        result,
        &[
            "status",
            "summary",
            "objective_results",
            "assumption_results",
            "done_when_results",
            "boundary_compliance",
            "effects",
            "artifacts",
            "uncertainties",
            "blockers",
            "role_output",
        ],
    ) {
        return Err("result_envelope_incomplete");
    }
    if result["status"] != "completed" {
        return Err("result_not_completed");
    }
    if result["summary"]
        .as_str()
        .is_none_or(|summary| summary.trim().is_empty())
        || !result_covers_task_items(
            &task["objectives"],
            "id",
            &result["objective_results"],
            "objective_id",
            "status",
            &["achieved"],
        )
        || !result_covers_task_items(
            &task["background"]["assumptions_to_check"],
            "id",
            &result["assumption_results"],
            "assumption_id",
            "assessment",
            &["confirmed"],
        )
        || !result_covers_task_items(
            &task["done_when"],
            "id",
            &result["done_when_results"],
            "done_when_id",
            "status",
            &["satisfied"],
        )
    {
        return Err("result_coverage_incomplete");
    }
    if result["boundary_compliance"]["status"] != "compliant"
        || !result["boundary_compliance"]["violations"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        return Err("boundary_violation");
    }
    if !result["uncertainties"]
        .as_array()
        .is_some_and(Vec::is_empty)
        || !result["blockers"].as_array().is_some_and(Vec::is_empty)
        || !result["artifacts"].as_array().is_some_and(|artifacts| {
            artifacts.iter().all(|artifact| {
                artifact["locator"]
                    .as_str()
                    .is_some_and(|locator| !locator.trim().is_empty())
            })
        })
    {
        return Err("unresolved_result");
    }

    let effects = result["effects"]
        .as_array()
        .ok_or("effect_inventory_missing")?;
    for effect in effects {
        if effect["status"] != "completed" {
            return Err("partial_effect");
        }
        if effect["authorization"]["status"] != "authorized" {
            return Err("unauthorized_effect");
        }
        if !matches!(
            effect["cleanup"]["status"].as_str(),
            Some("not_needed" | "completed")
        ) {
            return Err("cleanup_pending");
        }
        if !effect["provenance"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item == runtime_identity))
            || effect["verification"]
                .as_array()
                .is_none_or(|items| items.is_empty())
        {
            return Err("candidate_provenance_missing");
        }
    }

    if !has_exact_fields(
        &result["role_output"],
        &[
            "subject_results",
            "claim_results",
            "check_results",
            "discrepancies",
            "gaps",
        ],
    ) {
        return Err("role_output_incomplete");
    }
    if !result_covers_task_items(
        &task["role_input"]["subjects"],
        "id",
        &result["role_output"]["subject_results"],
        "subject_id",
        "status",
        &["verified"],
    ) || !result_covers_task_items(
        &task["role_input"]["claims"],
        "id",
        &result["role_output"]["claim_results"],
        "claim_id",
        "assessment",
        &["supports"],
    ) || !result_covers_task_items(
        &task["role_input"]["checks"],
        "id",
        &result["role_output"]["check_results"],
        "check_id",
        "status",
        &["passed"],
    ) || !result["role_output"]["discrepancies"]
        .as_array()
        .is_some_and(Vec::is_empty)
        || !result["role_output"]["gaps"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        return Err("role_output_incomplete");
    }
    let check = result["role_output"]["check_results"]
        .as_array()
        .and_then(|checks| checks.iter().find(|check| check["check_id"] == "VK1"))
        .filter(|check| check["status"] == "passed")
        .ok_or("candidate_check_missing")?;
    let observation = check["actual"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or("candidate_observation_missing")?;
    if !check["evidence"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item == runtime_identity))
    {
        return Err("candidate_provenance_missing");
    }
    let verifier_evidence = check["evidence"]
        .as_array()
        .expect("candidate provenance was validated")
        .iter()
        .filter_map(Value::as_str)
        .map(|value| json!({"string": value}))
        .collect::<Vec<_>>();
    let provenance = singleton_object(
        "object",
        json!({
            "runtime_identity": {"string": runtime_identity},
            "verifier_check": {"string": "VK1"},
            "verifier_evidence": {"list": verifier_evidence}
        }),
    );
    Ok(submit(record_evidence_command(
        fixture,
        evidence_id,
        observation,
        provenance,
    )))
}

#[test]
fn clean_stdio_mcp_direct_main_loop_reaches_achieved_and_healthy_audit() {
    let workspace = Workspace::new();
    let launcher_workspace = Workspace::new();
    let root = workspace.path().canonicalize().unwrap();
    let mut process = McpProcess::spawn(launcher_workspace.path());
    let fixture = advance_public_mcp_to_attempting(&mut process, &root, "direct-e2e");

    let recorded = apply_e2e_transition(
        &mut process,
        fixture.next_rpc_id,
        &root,
        &fixture.project_id,
        5,
        "direct-e2e-record-evidence",
        record_evidence_command(
            &fixture,
            "evidence-direct-e2e",
            "direct observation inspected by the main agent",
            json!({"string": "main agent directly inspected the local fixture"}),
        ),
    );
    assert_eq!(recorded["transition"], "record_evidence");

    finish_public_mcp_achieved(&mut process, &root, &fixture, fixture.next_rpc_id + 1);
    process.finish();
}

#[test]
fn clean_stdio_mcp_review_and_wait_branch_matrix_preserves_route_semantics() {
    for branch in [
        "retry",
        "replace",
        "review_remap",
        "wait_stay",
        "wait_same_route",
        "wait_new_route",
        "wait_remap",
    ] {
        let workspace = Workspace::new();
        let launcher_workspace = Workspace::new();
        let root = workspace.path().canonicalize().unwrap();
        let mut process = McpProcess::spawn(launcher_workspace.path());
        let lane = format!("review-{branch}-e2e");
        let fixture = advance_public_mcp_to_reviewing(&mut process, &root, &lane);
        let wait_id = format!("wait-{lane}");
        let action = match branch {
            "retry" => json!("retry"),
            "replace" => json!("replace"),
            "review_remap" => json!({
                "remap": {"reason": "the Stage structure needs a replacement Map"}
            }),
            branch if branch.starts_with("wait_") => json!({
                "wait": {
                    "id": wait_id,
                    "stage": fixture.attempting.stage_id,
                    "context": fixture.attempting.context,
                    "cause": "one external observation is not available yet",
                    "responsible_party": "bounded local fixture",
                    "resume_condition": "a fresh wait-resolution observation is frozen"
                }
            }),
            _ => unreachable!(),
        };

        let decided = apply_e2e_transition(
            &mut process,
            fixture.next_rpc_id,
            &root,
            &fixture.attempting.project_id,
            7,
            &format!("{lane}-decision"),
            review_branch_decision_command(&fixture, &format!("decision-{lane}"), action),
        );
        assert_eq!(decided["transition"], "decision", "wrong branch: {branch}");

        let status = objective_projection(&root, &fixture.attempting.objective_id);
        let navigation = &status["objective_state"]["navigating"]["navigation"];

        let final_seq = match branch {
            "retry" => {
                assert_eq!(
                    navigation["ready"]["route"], fixture.attempting.route_id,
                    "retry must preserve the current Route"
                );
                start_second_e2e_attempt(
                    &mut process,
                    fixture.next_rpc_id + 2,
                    &root,
                    &fixture.attempting,
                    8,
                );
                9
            }
            "replace" => {
                assert_eq!(
                    navigation["seeking_route"]["stage"], fixture.attempting.stage_id,
                    "replace must return the Stage to SeekingRoute"
                );
                assert_public_route_rejected(
                    &mut process,
                    fixture.next_rpc_id + 2,
                    &root,
                    &fixture.attempting,
                    8,
                    &format!("{lane}-reselect-rejected-route"),
                );
                8
            }
            "review_remap" => {
                assert_eq!(
                    status["objective_state"]["mapping"]["reason"]["remap"],
                    "the Stage structure needs a replacement Map"
                );
                install_second_e2e_map(
                    &mut process,
                    fixture.next_rpc_id + 2,
                    &root,
                    &fixture.attempting,
                    8,
                );
                9
            }
            branch if branch.starts_with("wait_") => {
                assert_eq!(navigation["waiting"]["route"], fixture.attempting.route_id);
                assert_eq!(navigation["waiting"]["wait_condition"], wait_id);
                let direction = branch
                    .strip_prefix("wait_")
                    .expect("wait branch has a direction");
                let wait_evidence_id = format!("evidence-{lane}-resolution");
                let checked = apply_e2e_transition(
                    &mut process,
                    fixture.next_rpc_id + 2,
                    &root,
                    &fixture.attempting.project_id,
                    8,
                    &format!("{lane}-check-wait"),
                    json!({
                        "check_wait": {
                            "wait_condition": wait_id,
                            "evidence": singleton_object(
                                &wait_evidence_id,
                                json!({
                                    "id": wait_evidence_id,
                                    "subject": {"wait_condition": wait_id},
                                    "context": fixture.attempting.context,
                                    "purpose": "wait_resolution",
                                    "claims": {},
                                    "observation": {
                                        "inline": {
                                            "string": "the external observation is now available"
                                        }
                                    },
                                    "provenance": {
                                        "string": "public MCP wait-resolution fixture"
                                    }
                                })
                            ),
                            "judgment": {
                                "wait_condition": wait_id,
                                "evidence_set": [wait_evidence_id],
                                "direction": direction,
                                "rationale": "the complete fresh batch resolves the wait"
                            }
                        }
                    }),
                );
                assert_eq!(checked["transition"], "check_wait");

                let resumed = objective_projection(&root, &fixture.attempting.objective_id);
                let objective_state = &resumed["objective_state"];
                match direction {
                    "stay" => {
                        assert_eq!(
                            objective_state["navigating"]["navigation"]["waiting"]["route"],
                            fixture.attempting.route_id,
                            "CheckWait(stay) must preserve the current Route"
                        );
                        9
                    }
                    "same_route" => {
                        assert_eq!(
                            objective_state["navigating"]["navigation"]["ready"]["route"],
                            fixture.attempting.route_id,
                            "CheckWait(same_route) must preserve and resume the current Route"
                        );
                        start_second_e2e_attempt(
                            &mut process,
                            fixture.next_rpc_id + 4,
                            &root,
                            &fixture.attempting,
                            9,
                        );
                        10
                    }
                    "new_route" => {
                        assert_eq!(
                            objective_state["navigating"]["navigation"]["seeking_route"]["stage"],
                            fixture.attempting.stage_id
                        );
                        assert_public_route_rejected(
                            &mut process,
                            fixture.next_rpc_id + 4,
                            &root,
                            &fixture.attempting,
                            9,
                            &format!("{lane}-reselect-rejected-route"),
                        );
                        9
                    }
                    "remap" => {
                        assert_eq!(objective_state["mapping"]["reason"], "wait_revealed_drift");
                        install_second_e2e_map(
                            &mut process,
                            fixture.next_rpc_id + 4,
                            &root,
                            &fixture.attempting,
                            9,
                        );
                        10
                    }
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        };

        let audit = readonly_audit(&root, &fixture.attempting.project_id);
        assert_eq!(audit["status"], "healthy", "wrong branch: {branch}");
        assert_eq!(audit["project_seq"], final_seq, "wrong branch: {branch}");
        assert_eq!(
            audit["issues"]["items"],
            json!([]),
            "wrong branch: {branch}"
        );
        process.finish();
    }
}

#[test]
fn clean_stdio_mcp_delegated_composition_gates_full_result_then_main_reaches_achieved() {
    let workspace = Workspace::new();
    let launcher_workspace = Workspace::new();
    let root = workspace.path().canonicalize().unwrap();
    let mut process = McpProcess::spawn(launcher_workspace.path());
    let fixture = advance_public_mcp_to_attempting(&mut process, &root, "delegated-e2e");
    let (runtime_identity, task, result) = delegated_e2e_inputs();
    assert!(result.get("record_evidence").is_none());
    assert!(result.get("command").is_none());

    for case in [
        "stale-baseline",
        "partial-result",
        "failed-result",
        "partial-effect",
        "unauthorized-effect",
        "cleanup-pending",
        "missing-f1",
        "missing-f2",
        "missing-provenance",
    ] {
        let mut rejected_task = task.clone();
        let mut rejected_result = result.clone();
        let mut delegated_heads = (5, 5);
        let expected = match case {
            "stale-baseline" => {
                delegated_heads = (4, 4);
                "stale_baseline"
            }
            "partial-result" => {
                rejected_result["status"] = json!("partial");
                "result_not_completed"
            }
            "failed-result" => {
                rejected_result["status"] = json!("failed");
                "result_not_completed"
            }
            "partial-effect" => {
                rejected_result["effects"] = json!([delegated_effect(&runtime_identity)]);
                rejected_result["effects"][0]["status"] = json!("partial");
                "partial_effect"
            }
            "unauthorized-effect" => {
                rejected_result["effects"] = json!([delegated_effect(&runtime_identity)]);
                rejected_result["effects"][0]["authorization"]["status"] = json!("unauthorized");
                "unauthorized_effect"
            }
            "cleanup-pending" => {
                rejected_result["effects"] = json!([delegated_effect(&runtime_identity)]);
                rejected_result["effects"][0]["cleanup"]["status"] = json!("pending");
                "cleanup_pending"
            }
            "missing-f1" | "missing-f2" => {
                let missing = if case == "missing-f1" { "F1" } else { "F2" };
                rejected_task["boundaries"]["forbidden"]
                    .as_array_mut()
                    .expect("fixture forbidden boundaries are an array")
                    .retain(|boundary| boundary["id"] != missing);
                if case == "missing-f1" {
                    "missing_forbidden_boundary_f1"
                } else {
                    "missing_forbidden_boundary_f2"
                }
            }
            "missing-provenance" => {
                rejected_result["role_output"]["check_results"][0]["evidence"] = json!([]);
                "candidate_provenance_missing"
            }
            _ => unreachable!(),
        };

        let mut submit_calls = 0;
        let rejected = consume_full_delegated_result(
            &rejected_task,
            &rejected_result,
            &runtime_identity,
            delegated_heads,
            &fixture,
            &format!("evidence-rejected-{case}"),
            |_| {
                submit_calls += 1;
                json!({"unexpected": "submission"})
            },
        )
        .expect_err("unfit delegated result must fail before a Core submission");
        assert_eq!(rejected, expected, "wrong rejection for {case}");
        assert_eq!(submit_calls, 0, "{case} reached the Core submit closure");
    }

    let unchanged = objective_projection(&root, &fixture.objective_id);
    assert_eq!(
        unchanged["objective_state"]["navigating"]["navigation"]["attempting"]["attempt"],
        fixture.attempt_id
    );

    let mut submit_calls = 0;
    let submitted = consume_full_delegated_result(
        &task,
        &result,
        &runtime_identity,
        (5, 5),
        &fixture,
        "evidence-delegated-e2e",
        |command| {
            submit_calls += 1;
            assert!(has_exact_fields(&command, &["record_evidence"]));
            call_tool(
                &mut process,
                fixture.next_rpc_id + 1,
                "mobius_apply_transition",
                json!({
                    "project_root": root,
                    "project_id": fixture.project_id,
                    "expected_heads": {
                        "expected_project_seq": 5,
                        "expected_objective_seq": 5
                    },
                    "request_id": "delegated-e2e-main-record-evidence",
                    "command": command
                }),
                &root,
            )
        },
    )
    .expect("fresh compliant result is translated by the main agent");
    assert_eq!(submit_calls, 1);
    let submitted = assert_tool_success(&submitted);
    assert_eq!(submitted["transition"], "record_evidence");
    assert_eq!(submitted["committed_project_seq"], 6);
    assert_eq!(submitted["committed_objective_seq"], 6);

    finish_public_mcp_achieved(&mut process, &root, &fixture, fixture.next_rpc_id + 2);
    process.finish();
}

#[test]
fn real_stdio_session_lists_only_mutation_tools_and_rejects_removed_reads() {
    let workspace = Workspace::new();
    let launcher_workspace = Workspace::new();
    let root = workspace
        .path()
        .canonicalize()
        .expect("workspace must canonicalize");
    let root_text = root.to_string_lossy().into_owned();
    let mut process = McpProcess::spawn(launcher_workspace.path());

    let initialized = process.initialize();
    assert_eq!(initialized["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(initialized["result"]["serverInfo"]["name"], "mobius");
    let instructions = initialized["result"]["instructions"]
        .as_str()
        .expect("server instructions");
    assert!(instructions.contains("untrusted data"));
    assert!(instructions.contains("SQLite"));
    assert!(instructions.contains("read-only safe mode"));
    assert_eq!(
        initialized["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert_eq!(
        initialized["result"]["capabilities"]["experimental"]["codex/sandbox-state-meta"],
        json!({})
    );

    process.notify("notifications/initialized", json!({}));
    process.notify(
        "notifications/progress",
        json!({"progressToken": "ignored", "progress": 1}),
    );

    let listed = process.request(2, "tools/list", json!({}));
    let tools = listed["result"]["tools"]
        .as_array()
        .expect("tools/list must return an array");
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "mobius_project_init",
            "mobius_capture_artifact",
            "mobius_apply_transition",
            "mobius_audit"
        ]
    );
    assert!(tools.iter().all(|tool| tool["inputSchema"].is_object()));
    assert!(tools.iter().all(|tool| tool["outputSchema"].is_object()));
    assert!(tools.iter().all(|tool| {
        tool["description"]
            .as_str()
            .is_some_and(|description| description.contains("isError=true"))
    }));

    let initialized_project = call_tool(
        &mut process,
        3,
        "mobius_project_init",
        json!({"project_root": root_text, "request_id": "mcp-project-init-1"}),
        &root,
    );
    let project_id = assert_tool_success(&initialized_project)["project_id"]
        .as_str()
        .expect("project_init must return a project id")
        .to_owned();
    let removed_read = call_tool(&mut process, 4, "mobius_read", json!({}), &root);
    assert_eq!(removed_read["error"]["code"], -32602);
    assert_eq!(removed_read["error"]["message"], "unknown tool name");

    let audit = call_tool(
        &mut process,
        5,
        "mobius_audit",
        json!({
            "binding": {"project_root": root_text, "project_id": project_id}
        }),
        &root,
    );
    assert_eq!(audit["result"]["isError"], true);
    assert_eq!(
        audit["result"]["structuredContent"]["code"],
        "maintenance_required"
    );
    assert_eq!(readonly_audit(&root, &project_id)["status"], "healthy");

    let removed_artifact_read =
        call_tool(&mut process, 6, "mobius_read_artifact", json!({}), &root);
    assert_eq!(removed_artifact_read["error"]["code"], -32602);

    let caller_packet = call_tool(
        &mut process,
        7,
        "mobius_apply_transition",
        json!({
            "project_root": root_text,
            "project_id": project_id,
            "expected_heads": {
                "expected_project_seq": 0,
                "expected_objective_seq": 0
            },
            "request_id": "forbidden-caller-packet",
            "command": {
                "seal_attempt": {
                    "attempt": "attempt-1",
                    "seal_reason": "submitted",
                    "packet": {}
                }
            }
        }),
        &root,
    );
    assert!(caller_packet.get("error").is_none());
    assert_eq!(caller_packet["result"]["isError"], true);
    assert_eq!(
        caller_packet["result"]["structuredContent"]["code"],
        "invalid_tool_input"
    );
    assert!(
        caller_packet["result"]["structuredContent"]["message"]
            .as_str()
            .expect("tool error message")
            .contains("packet")
    );

    process.finish();
}

#[test]
fn official_thread_metadata_drives_only_best_effort_post_commit_reports() {
    let workspace = Workspace::new();
    let launcher_workspace = Workspace::new();
    let root = workspace.path().canonicalize().unwrap();
    let mut process = McpProcess::spawn(launcher_workspace.path());
    process.initialize();
    process.notify("notifications/initialized", json!({}));

    let initialized = call_tool_with_thread(
        &mut process,
        2,
        "mobius_project_init",
        json!({"project_root": root, "request_id": "auto-report-init"}),
        &root,
        Some("thread-auto-report"),
    );
    let project_id = assert_tool_success(&initialized)["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let objective_id = "objective-auto-report";
    let activation = activation_arguments(&root, &project_id, objective_id, "auto-report-activate");
    let activated = call_tool_with_thread(
        &mut process,
        3,
        "mobius_apply_transition",
        activation.clone(),
        &root,
        Some("thread-auto-report"),
    );
    let activated_value = assert_tool_success(&activated);
    assert_eq!(activated_value["committed_project_seq"], 1);
    assert_eq!(activated_value["committed_objective_seq"], 1);
    let response_text = serde_json::to_string(activated_value).unwrap();
    for presentation_detail in [
        "current.csv",
        "generation_path",
        "refresh_task",
        "report_log",
    ] {
        assert!(!response_text.contains(presentation_detail));
    }

    let run = sole_auto_run(&root, "thread-auto-report");
    let current = run.join("current.csv");
    assert!(
        fs::read_to_string(&current)
            .unwrap()
            .contains(",1,1,mobius.report.v1")
    );
    let initial_current_bytes = fs::read(&current).unwrap();
    assert_eq!(generation_count(&run), 1);

    let retried = call_tool_with_thread(
        &mut process,
        4,
        "mobius_apply_transition",
        activation.clone(),
        &root,
        Some("thread-auto-report-two"),
    );
    assert_eq!(assert_tool_success(&retried), activated_value);
    assert_eq!(
        generation_count(&run),
        1,
        "a same-head idempotent retry must not publish another generation"
    );
    let second_run = sole_auto_run(&root, "thread-auto-report-two");
    let second_current = second_run.join("current.csv");
    assert!(
        fs::read_to_string(&second_current)
            .unwrap()
            .contains(",1,1,mobius.report.v1")
    );
    assert_eq!(generation_count(&second_run), 1);

    for (id, thread_id) in [
        (40, "thread-auto-report-missing"),
        (41, "thread-auto-report-malformed"),
        (42, "thread-auto-report-incomplete"),
        (43, "thread-auto-report-tampered-digest"),
        (44, "thread-auto-report-future-heads"),
        (45, "thread-auto-report-wrong-kind"),
        (46, "thread-auto-report-symlink"),
    ] {
        let initialized_run = call_tool_with_thread(
            &mut process,
            id,
            "mobius_apply_transition",
            activation.clone(),
            &root,
            Some(thread_id),
        );
        assert_eq!(assert_tool_success(&initialized_run), activated_value);
    }
    let missing_run = sole_auto_run(&root, "thread-auto-report-missing");
    let missing_current = missing_run.join("current.csv");
    fs::remove_file(&missing_current).unwrap();
    let malformed_run = sole_auto_run(&root, "thread-auto-report-malformed");
    let malformed_current = malformed_run.join("current.csv");
    let malformed_bytes = b"malformed,current\n";
    fs::write(&malformed_current, malformed_bytes).unwrap();
    let incomplete_run = sole_auto_run(&root, "thread-auto-report-incomplete");
    let incomplete_generation = fs::read_dir(incomplete_run.join("generations"))
        .unwrap()
        .next()
        .expect("one incomplete report generation")
        .unwrap()
        .path();
    fs::remove_file(incomplete_generation.join("meta.csv")).unwrap();
    let tampered_run = sole_auto_run(&root, "thread-auto-report-tampered-digest");
    let tampered_current = tampered_run.join("current.csv");
    let tampered_current_bytes = fs::read(&tampered_current).unwrap();
    let tampered_generation = fs::read_dir(tampered_run.join("generations"))
        .unwrap()
        .next()
        .expect("one tampered report generation")
        .unwrap()
        .path();
    let tampered_meta_path = tampered_generation.join("meta.csv");
    let original_tampered_meta = fs::read_to_string(&tampered_meta_path).unwrap();
    let tampered_meta =
        original_tampered_meta.replacen("trail_digest,sha256:", "trail_digest,sha256:tampered-", 1);
    assert_ne!(tampered_meta, original_tampered_meta);
    fs::write(&tampered_meta_path, &tampered_meta).unwrap();

    let future_run = sole_auto_run(&root, "thread-auto-report-future-heads");
    let future_current = future_run.join("current.csv");
    let future_current_text = fs::read_to_string(&future_current).unwrap().replacen(
        ",1,1,mobius.report.v1",
        ",99,99,mobius.report.v1",
        1,
    );
    fs::write(&future_current, &future_current_text).unwrap();
    let future_current_bytes = fs::read(&future_current).unwrap();
    let future_generation = fs::read_dir(future_run.join("generations"))
        .unwrap()
        .next()
        .expect("one future-head report generation")
        .unwrap()
        .path();
    let future_meta_path = future_generation.join("meta.csv");
    let original_future_meta = fs::read_to_string(&future_meta_path).unwrap();
    let future_meta = original_future_meta
        .replacen("project_seq,1", "project_seq,99", 1)
        .replacen("objective_seq,1", "objective_seq,99", 1);
    assert_ne!(future_meta, original_future_meta);
    fs::write(&future_meta_path, &future_meta).unwrap();

    let wrong_kind_run = sole_auto_run(&root, "thread-auto-report-wrong-kind");
    let wrong_kind_current = wrong_kind_run.join("current.csv");
    fs::remove_file(&wrong_kind_current).unwrap();
    fs::create_dir(&wrong_kind_current).unwrap();
    fs::write(wrong_kind_current.join("sentinel"), b"preserve directory").unwrap();

    #[cfg(unix)]
    let (symlink_current, symlink_target) = {
        use std::os::unix::fs::symlink;

        let symlink_run = sole_auto_run(&root, "thread-auto-report-symlink");
        let symlink_current = symlink_run.join("current.csv");
        let symlink_target = root.join("external-auto-current.csv");
        fs::write(&symlink_target, b"preserve external target").unwrap();
        fs::remove_file(&symlink_current).unwrap();
        symlink(&symlink_target, &symlink_current).unwrap();
        (symlink_current, symlink_target)
    };

    for (id, thread_id) in [
        (50, "thread-auto-report-missing"),
        (51, "thread-auto-report-malformed"),
        (52, "thread-auto-report-incomplete"),
        (53, "thread-auto-report-tampered-digest"),
        (54, "thread-auto-report-future-heads"),
        (55, "thread-auto-report-wrong-kind"),
        (56, "thread-auto-report-symlink"),
    ] {
        let retried = call_tool_with_thread(
            &mut process,
            id,
            "mobius_apply_transition",
            activation.clone(),
            &root,
            Some(thread_id),
        );
        assert_eq!(assert_tool_success(&retried), activated_value);
    }
    assert!(!missing_current.exists());
    assert_eq!(fs::read(&malformed_current).unwrap(), malformed_bytes);
    assert!(!incomplete_generation.join("meta.csv").exists());
    assert_eq!(fs::read(&tampered_current).unwrap(), tampered_current_bytes);
    assert_eq!(
        fs::read_to_string(&tampered_meta_path).unwrap(),
        tampered_meta
    );
    assert_eq!(fs::read(&future_current).unwrap(), future_current_bytes);
    assert_eq!(fs::read_to_string(&future_meta_path).unwrap(), future_meta);
    assert!(wrong_kind_current.is_dir());
    assert_eq!(
        fs::read(wrong_kind_current.join("sentinel")).unwrap(),
        b"preserve directory"
    );
    #[cfg(unix)]
    {
        assert!(
            fs::symlink_metadata(&symlink_current)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(&symlink_target).unwrap(),
            b"preserve external target"
        );
    }

    let revised = call_tool_with_thread(
        &mut process,
        5,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 1, "expected_objective_seq": 1},
            "request_id": "auto-report-revise",
            "command": {
                "revise_objective": {
                    "objective_spec": objective_spec(objective_id, 2),
                    "confirmation": {
                        "project": project_id,
                        "action": "revise",
                        "objective_spec": {"objective": objective_id, "revision": 2},
                        "confirmed_payload": objective_spec(objective_id, 2),
                        "heads": {"expected_project_seq": 1, "expected_objective_seq": 1},
                        "confirmed": true
                    }
                }
            }
        }),
        &root,
        Some("thread-auto-report"),
    );
    assert_eq!(assert_tool_success(&revised)["committed_project_seq"], 2);
    let stale_retry = call_tool_with_thread(
        &mut process,
        57,
        "mobius_apply_transition",
        activation.clone(),
        &root,
        Some("thread-auto-report"),
    );
    assert_eq!(assert_tool_success(&stale_retry), activated_value);
    assert!(
        fs::read_to_string(&current)
            .unwrap()
            .contains(",1,1,mobius.report.v1")
    );
    assert!(
        fs::read_to_string(&second_current)
            .unwrap()
            .contains(",1,1,mobius.report.v1")
    );
    assert_eq!(generation_count(&run), 1);
    assert_eq!(generation_count(&second_run), 1);

    let reason = "finish the automatic report fixture";
    let abandon_arguments = json!({
        "project_root": root,
        "project_id": project_id,
        "expected_heads": {"expected_project_seq": 2, "expected_objective_seq": 2},
        "request_id": "auto-report-abandon",
        "command": {
            "abandon": {
                "reason": reason,
                "confirmation": {
                    "project": project_id,
                    "objective": objective_id,
                    "reason": reason,
                    "heads": {"expected_project_seq": 2, "expected_objective_seq": 2},
                    "confirmed": true
                }
            }
        }
    });
    let abandoned = call_tool_with_thread(
        &mut process,
        6,
        "mobius_apply_transition",
        abandon_arguments.clone(),
        &root,
        Some("thread-auto-report"),
    );
    assert_eq!(assert_tool_success(&abandoned)["committed_project_seq"], 3);
    assert!(
        fs::read_to_string(&current)
            .unwrap()
            .contains(",3,3,mobius.report.v1")
    );
    assert!(
        fs::read_to_string(&second_current)
            .unwrap()
            .contains(",3,3,mobius.report.v1")
    );
    assert_eq!(generation_count(&run), 2);
    assert_eq!(generation_count(&second_run), 2);

    fs::write(&current, &initial_current_bytes).unwrap();
    let terminal_generation_count = generation_count(&run);
    for (id, arguments) in [(58, activation.clone()), (59, abandon_arguments.clone())] {
        let replayed = call_tool_with_thread(
            &mut process,
            id,
            "mobius_apply_transition",
            arguments,
            &root,
            Some("thread-auto-report"),
        );
        assert_tool_success(&replayed);
        assert_eq!(fs::read(&current).unwrap(), initial_current_bytes);
        assert_eq!(generation_count(&run), terminal_generation_count);
    }
    assert!(!missing_current.exists());
    assert_eq!(generation_count(&missing_run), 1);
    assert_eq!(fs::read(&malformed_current).unwrap(), malformed_bytes);
    assert_eq!(generation_count(&malformed_run), 1);
    assert!(!incomplete_generation.join("meta.csv").exists());
    assert_eq!(generation_count(&incomplete_run), 1);
    assert_eq!(fs::read(&tampered_current).unwrap(), tampered_current_bytes);
    assert_eq!(
        fs::read_to_string(&tampered_meta_path).unwrap(),
        tampered_meta
    );
    assert_eq!(generation_count(&tampered_run), 1);
    assert_eq!(fs::read(&future_current).unwrap(), future_current_bytes);
    assert_eq!(fs::read_to_string(&future_meta_path).unwrap(), future_meta);
    assert_eq!(generation_count(&future_run), 1);
    assert!(wrong_kind_current.is_dir());
    assert_eq!(generation_count(&wrong_kind_run), 1);
    #[cfg(unix)]
    {
        assert!(
            fs::symlink_metadata(&symlink_current)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(&symlink_target).unwrap(),
            b"preserve external target"
        );
    }
    process.finish();

    let no_session_workspace = Workspace::new();
    let root = no_session_workspace.path().canonicalize().unwrap();
    let mut process = McpProcess::spawn(&root);
    process.initialize();
    process.notify("notifications/initialized", json!({}));
    let initialized = call_tool(
        &mut process,
        10,
        "mobius_project_init",
        json!({"project_root": root, "request_id": "no-session-init"}),
        &root,
    );
    let project_id = assert_tool_success(&initialized)["project_id"]
        .as_str()
        .unwrap();
    let mut activation = activation_arguments(
        &root,
        project_id,
        "objective-no-session",
        "no-session-activate",
    );
    activation["interaction"] = interaction_summary("an interaction without a session");
    let activated = call_tool(
        &mut process,
        11,
        "mobius_apply_transition",
        activation,
        &root,
    );
    let activated = assert_tool_success(&activated);
    assert_eq!(activated["committed_project_seq"], 1);
    assert!(activated.get("interaction_path").is_none());
    assert_eq!(fs::read_dir(root.join(".mobius/views")).unwrap().count(), 0);
    process.finish();

    let broken_view_workspace = Workspace::new();
    let root = broken_view_workspace.path().canonicalize().unwrap();
    let mut process = McpProcess::spawn(&root);
    process.initialize();
    process.notify("notifications/initialized", json!({}));
    let initialized = call_tool(
        &mut process,
        20,
        "mobius_project_init",
        json!({"project_root": root, "request_id": "broken-view-init"}),
        &root,
    );
    let project_id = assert_tool_success(&initialized)["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::write(
        root.join(".mobius/views/codex-session-broken-thread"),
        b"blocks only the derived run path",
    )
    .unwrap();
    let mut activation = activation_arguments(
        &root,
        &project_id,
        "objective-broken-view",
        "broken-view-activate",
    );
    activation["interaction"] = interaction_summary("a presentation write failure");
    let activated = call_tool_with_thread(
        &mut process,
        21,
        "mobius_apply_transition",
        activation,
        &root,
        Some("broken-thread"),
    );
    let activated = assert_tool_success(&activated);
    assert_eq!(activated["committed_project_seq"], 1);
    assert!(activated.get("interaction_path").is_none());
    let read = objective_projection(&root, "objective-broken-view");
    assert_eq!(
        read["objective_state"]["mapping"]["objective"],
        "objective-broken-view"
    );
    process.finish();
}

#[test]
fn accepted_objective_interactions_write_one_deletable_route_design_summary() {
    let workspace = Workspace::new();
    let launcher_workspace = Workspace::new();
    let root = workspace.path().canonicalize().unwrap();
    let mut process = McpProcess::spawn(launcher_workspace.path());
    process.initialize();
    process.notify("notifications/initialized", json!({}));

    let initialized = call_tool_with_thread(
        &mut process,
        2,
        "mobius_project_init",
        json!({"project_root": root, "request_id": "interaction-init"}),
        &root,
        Some("thread-copilot-interaction"),
    );
    let project_id = assert_tool_success(&initialized)["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let objective_id = "objective-copilot-interaction";
    let mut activation =
        activation_arguments(&root, &project_id, objective_id, "interaction-activate");
    activation["interaction"] = interaction_summary("the initial intent");
    let activated = call_tool_with_thread(
        &mut process,
        3,
        "mobius_apply_transition",
        activation.clone(),
        &root,
        Some("thread-copilot-interaction"),
    );
    let activated_value = assert_tool_success(&activated).clone();
    let interaction_path = PathBuf::from(
        activated_value["interaction_path"]
            .as_str()
            .expect("accepted interaction returns its path"),
    );
    assert!(interaction_path.starts_with(
        root.join(".mobius/views/codex-session-thread-copilot-interaction/interactions")
    ));
    assert_eq!(
        interaction_path.file_name().and_then(|name| name.to_str()),
        Some("interaction.md")
    );
    let activated_markdown = fs::read_to_string(&interaction_path).unwrap();
    assert!(activated_markdown.contains("- Objective: objective-copilot-interaction"));
    assert!(activated_markdown.contains("- Revision: 1"));
    assert!(activated_markdown.contains("understand the initial intent"));

    let mut idempotent_activation = activation.clone();
    idempotent_activation["interaction"] = interaction_summary("the current replayed intent");
    let replayed = call_tool_with_thread(
        &mut process,
        30,
        "mobius_apply_transition",
        idempotent_activation,
        &root,
        Some("thread-copilot-interaction"),
    );
    let replayed_value = assert_tool_success(&replayed);
    assert_eq!(core_receipt(replayed_value), core_receipt(&activated_value));
    assert_eq!(
        replayed_value["interaction_path"],
        activated_value["interaction_path"]
    );
    let replayed_markdown = fs::read_to_string(&interaction_path).unwrap();
    assert!(replayed_markdown.contains("understand the current replayed intent"));
    assert!(!replayed_markdown.contains("understand the initial intent"));

    let revision_command = json!({
        "revise_objective": {
            "objective_spec": objective_spec(objective_id, 2),
            "confirmation": {
                "project": project_id,
                "action": "revise",
                "objective_spec": {"objective": objective_id, "revision": 2},
                "confirmed_payload": objective_spec(objective_id, 2),
                "heads": {"expected_project_seq": 1, "expected_objective_seq": 1},
                "confirmed": true
            }
        }
    });
    let mut rejected_arguments = json!({
        "project_root": root,
        "project_id": project_id,
        "expected_heads": {"expected_project_seq": 0, "expected_objective_seq": 0},
        "request_id": "interaction-revise-rejected",
        "command": revision_command,
        "interaction": interaction_summary("a rejected revision")
    });
    rejected_arguments["command"]["revise_objective"]["confirmation"]["heads"] =
        json!({"expected_project_seq": 0, "expected_objective_seq": 0});
    let rejected = call_tool_with_thread(
        &mut process,
        4,
        "mobius_apply_transition",
        rejected_arguments,
        &root,
        Some("thread-copilot-interaction"),
    );
    assert_eq!(rejected["result"]["isError"], true);
    assert_eq!(
        fs::read_to_string(&interaction_path).unwrap(),
        replayed_markdown
    );

    let revised = call_tool_with_thread(
        &mut process,
        5,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 1, "expected_objective_seq": 1},
            "request_id": "interaction-revise",
            "command": revision_command,
            "interaction": interaction_summary("the revised intent")
        }),
        &root,
        Some("thread-copilot-interaction"),
    );
    let revised_path = PathBuf::from(
        assert_tool_success(&revised)["interaction_path"]
            .as_str()
            .unwrap(),
    );
    assert_eq!(revised_path, interaction_path);
    let revised_markdown = fs::read_to_string(&revised_path).unwrap();
    assert!(revised_markdown.contains("- Revision: 2"));
    assert!(revised_markdown.contains("- Action: revise"));
    assert!(revised_markdown.contains("understand the revised intent"));
    assert!(!revised_markdown.contains("understand the initial intent"));

    let mut stale_activation = activation;
    stale_activation["interaction"] = interaction_summary("a stale activation replay");
    let stale_replay = call_tool_with_thread(
        &mut process,
        6,
        "mobius_apply_transition",
        stale_activation,
        &root,
        Some("thread-copilot-interaction"),
    );
    let stale_value = assert_tool_success(&stale_replay);
    assert_eq!(core_receipt(stale_value), core_receipt(&activated_value));
    assert!(stale_value.get("interaction_path").is_none());
    assert_eq!(
        fs::read_to_string(&interaction_path).unwrap(),
        revised_markdown
    );

    let reason = "interaction is forbidden on abandonment";
    let forbidden = call_tool_with_thread(
        &mut process,
        7,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 2, "expected_objective_seq": 2},
            "request_id": "interaction-forbidden-abandon",
            "command": {
                "abandon": {
                    "reason": reason,
                    "confirmation": {
                        "project": project_id,
                        "objective": objective_id,
                        "reason": reason,
                        "heads": {"expected_project_seq": 2, "expected_objective_seq": 2},
                        "confirmed": true
                    }
                }
            },
            "interaction": interaction_summary("a forbidden transition")
        }),
        &root,
        Some("thread-copilot-interaction"),
    );
    assert_eq!(forbidden["result"]["isError"], true);
    assert_eq!(
        forbidden["result"]["structuredContent"]["code"],
        "invalid_tool_input"
    );
    assert_eq!(
        fs::read_to_string(&interaction_path).unwrap(),
        revised_markdown
    );

    fs::remove_file(&revised_path).unwrap();
    assert!(!revised_path.exists());
    let audit = readonly_audit(&root, &project_id);
    assert_eq!(audit["status"], "healthy");
    assert_eq!(audit["project_seq"], 2);
    assert_eq!(
        objective_projection(&root, objective_id)["objective_state"]["mapping"]["objective"],
        objective_id
    );
    process.finish();
}

#[test]
fn carry_completing_install_map_refreshes_all_existing_terminal_reports() {
    let workspace = Workspace::new();
    let launcher_workspace = Workspace::new();
    let root = workspace.path().canonicalize().unwrap();
    let mut process = McpProcess::spawn(launcher_workspace.path());
    process.initialize();
    process.notify("notifications/initialized", json!({}));

    let initialized = call_tool_with_thread(
        &mut process,
        2,
        "mobius_project_init",
        json!({"project_root": root, "request_id": "carry-report-init"}),
        &root,
        Some("thread-carry-report"),
    );
    let project_id = assert_tool_success(&initialized)["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let objective_id = "objective-carry-report";
    let primary_stage_id = "stage-auto-report-primary";
    let activation =
        activation_arguments(&root, &project_id, objective_id, "carry-report-activate");
    let activated = call_tool_with_thread(
        &mut process,
        3,
        "mobius_apply_transition",
        activation.clone(),
        &root,
        Some("thread-carry-report"),
    );
    let activated_value = assert_tool_success(&activated);
    assert_eq!(activated_value["committed_project_seq"], 1);
    let activated_again = call_tool_with_thread(
        &mut process,
        30,
        "mobius_apply_transition",
        activation,
        &root,
        Some("thread-carry-report-two"),
    );
    assert_eq!(assert_tool_success(&activated_again), activated_value);
    let run = sole_auto_run(&root, "thread-carry-report");
    let current = run.join("current.csv");
    let second_run = sole_auto_run(&root, "thread-carry-report-two");
    let second_current = second_run.join("current.csv");
    assert!(
        fs::read_to_string(&current)
            .unwrap()
            .contains(",1,1,mobius.report.v1")
    );
    assert!(
        fs::read_to_string(&second_current)
            .unwrap()
            .contains(",1,1,mobius.report.v1")
    );

    let first_map = automatic_report_map(objective_id, 1, true);
    let installed = call_tool(
        &mut process,
        4,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 1, "expected_objective_seq": 1},
            "request_id": "carry-report-map-1",
            "command": {"install_map": {
                "map": first_map,
                "initial_routes": {},
                "cover": {
                    "map": {"objective": objective_id, "revision": 1},
                    "objective_spec": {"objective": objective_id, "revision": 1},
                    "verdict": "covered",
                    "rationale": "the two Stages cover the report fixture"
                },
                "carry": {}
            }}
        }),
        &root,
    );
    assert_eq!(assert_tool_success(&installed)["committed_project_seq"], 2);

    let primary_contract = json!({
        "outcome": "the primary report fact is accepted",
        "criteria": ["criterion-auto-report"],
        "objective_boundaries": ["local project only"],
        "output": "accepted primary report fact"
    });
    let route_id = "route-carry-report-primary";
    let added = call_tool(
        &mut process,
        5,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 2, "expected_objective_seq": 2},
            "request_id": "carry-report-add-route",
            "command": {"add_route": {"route": {
                "id": route_id,
                "stage": primary_stage_id,
                "structural_context": {"contract": primary_contract, "dependencies": {}},
                "hypothesis": "one report fact can be accepted",
                "assumptions": ["the local fixture remains stable"],
                "rationale": "exercise terminal carry report refresh"
            }}}
        }),
        &root,
    );
    assert_eq!(assert_tool_success(&added)["committed_project_seq"], 3);
    let selected = call_tool(
        &mut process,
        6,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 3, "expected_objective_seq": 3},
            "request_id": "carry-report-select-route",
            "command": {"select_route": {"route": route_id}}
        }),
        &root,
    );
    assert_eq!(assert_tool_success(&selected)["committed_project_seq"], 4);

    let route = projected_object(&root, objective_id, "route", route_id);
    let context = json!({
        "structural": route["route"]["structural_context"],
        "dependency_proofs": {}
    });
    let attempt_id = "attempt-carry-report-primary";
    let started = call_tool(
        &mut process,
        8,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 4, "expected_objective_seq": 4},
            "request_id": "carry-report-start",
            "command": {"start_attempt": {"attempt": {
                "id": attempt_id,
                "route": route_id,
                "ordinal": 1,
                "bound": {"termination_condition": "the primary report fact is frozen"},
                "context": context
            }}}
        }),
        &root,
    );
    assert_eq!(assert_tool_success(&started)["committed_project_seq"], 5);
    let recorded = call_tool(
        &mut process,
        9,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 5, "expected_objective_seq": 5},
            "request_id": "carry-report-evidence",
            "command": {"record_evidence": {"evidence": {
                "id": "evidence-carry-report-primary",
                "subject": {"attempt": attempt_id},
                "context": context,
                "purpose": "stage_review",
                "claims": {"criterion-auto-report": "supports"},
                "observation": {"inline": {"string": "the primary report fact is observed"}},
                "provenance": {"string": "public MCP terminal report regression"}
            }}}
        }),
        &root,
    );
    assert_eq!(assert_tool_success(&recorded)["committed_project_seq"], 6);
    let sealed = call_tool(
        &mut process,
        10,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 6, "expected_objective_seq": 6},
            "request_id": "carry-report-seal",
            "command": {"seal_attempt": {
                "attempt": attempt_id,
                "seal_reason": "submitted"
            }}
        }),
        &root,
    );
    assert_eq!(assert_tool_success(&sealed)["committed_project_seq"], 7);

    let material = current_review_packet(&root, objective_id);
    let packet_id = material["review_packet"]["id"].as_str().unwrap().to_owned();
    let accepted = call_tool(
        &mut process,
        12,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 7, "expected_objective_seq": 7},
            "request_id": "carry-report-accept",
            "command": {"decision": {"decision": {
                "id": "decision-carry-report-primary",
                "packet": packet_id,
                "judgments": {"criterion-auto-report": "satisfied"},
                "findings": [],
                "action": "accept"
            }}}
        }),
        &root,
    );
    assert_eq!(assert_tool_success(&accepted)["committed_project_seq"], 8);
    assert!(
        fs::read_to_string(&current)
            .unwrap()
            .contains(",1,1,mobius.report.v1")
    );

    let remapped = call_tool(
        &mut process,
        13,
        "mobius_apply_transition",
        json!({
            "project_root": root,
            "project_id": project_id,
            "expected_heads": {"expected_project_seq": 8, "expected_objective_seq": 8},
            "request_id": "carry-report-remap",
            "command": {"request_remap": {
                "reason": "remove the unfinished follow-up and carry the accepted proof"
            }}
        }),
        &root,
    );
    assert_eq!(assert_tool_success(&remapped)["committed_project_seq"], 9);

    let carry_install = json!({
        "project_root": root,
        "project_id": project_id,
        "expected_heads": {"expected_project_seq": 9, "expected_objective_seq": 9},
        "request_id": "carry-report-map-2",
        "command": {"install_map": {
            "map": automatic_report_map(objective_id, 2, false),
            "initial_routes": {},
            "cover": {
                "map": {"objective": objective_id, "revision": 2},
                "objective_spec": {"objective": objective_id, "revision": 1},
                "verdict": "covered",
                "rationale": "the carried primary Stage covers the Objective"
            },
            "carry": {"stage-auto-report-primary": "valid"}
        }}
    });
    let completed = call_tool_with_thread(
        &mut process,
        14,
        "mobius_apply_transition",
        carry_install.clone(),
        &root,
        Some("thread-carry-report-two"),
    );
    let completed_value = assert_tool_success(&completed);
    assert_eq!(completed_value["committed_project_seq"], 10);
    assert!(
        fs::read_to_string(&current)
            .unwrap()
            .contains(",10,10,mobius.report.v1")
    );
    assert!(
        fs::read_to_string(&second_current)
            .unwrap()
            .contains(",10,10,mobius.report.v1")
    );
    assert_eq!(generation_count(&run), 2);
    assert_eq!(generation_count(&second_run), 2);

    let retried = call_tool_with_thread(
        &mut process,
        15,
        "mobius_apply_transition",
        carry_install,
        &root,
        Some("thread-carry-report-two"),
    );
    assert_eq!(assert_tool_success(&retried), completed_value);
    assert_eq!(generation_count(&run), 2);
    assert_eq!(generation_count(&second_run), 2);

    let status = objective_projection(&root, objective_id);
    let proof = &status["objective_state"]["achieved"]["manifest"];
    assert_eq!(proof.as_object().map(serde_json::Map::len), Some(1));
    assert_eq!(proof[primary_stage_id], "decision-carry-report-primary");
    process.finish();
}

#[test]
fn tool_calls_require_fresh_absolute_host_admission_metadata() {
    let admitted_workspace = Workspace::new();
    let other_workspace = Workspace::new();
    let launcher_workspace = Workspace::new();
    let admitted_root = admitted_workspace.path().join("workspace with space");
    fs::create_dir(&admitted_root).unwrap();
    let admitted_root = admitted_root.canonicalize().unwrap();
    let other_root = other_workspace.path().canonicalize().unwrap();
    let mut process = McpProcess::spawn(launcher_workspace.path());

    process.initialize();
    process.notify("notifications/initialized", json!({}));

    let missing = process.request(
        2,
        "tools/call",
        json!({
            "name": "mobius_project_init",
            "arguments": {
                "project_root": admitted_root,
                "request_id": "missing-host-context"
            }
        }),
    );
    assert_eq!(missing["result"]["isError"], true);
    assert_eq!(
        missing["result"]["structuredContent"]["code"],
        "host_admission_context_invalid"
    );

    let relative = process.request(
        3,
        "tools/call",
        json!({
            "name": "mobius_project_init",
            "arguments": {
                "project_root": admitted_root,
                "request_id": "relative-host-context"
            },
            "_meta": {"codex/sandbox-state-meta": {"sandboxCwd": "."}}
        }),
    );
    assert_eq!(relative["result"]["isError"], true);
    assert_eq!(
        relative["result"]["structuredContent"]["code"],
        "host_admission_context_invalid"
    );

    let crossed = call_tool(
        &mut process,
        4,
        "mobius_project_init",
        json!({
            "project_root": other_root,
            "request_id": "cross-workspace-context"
        }),
        &admitted_root,
    );
    assert_eq!(crossed["result"]["isError"], true);
    assert_eq!(
        crossed["result"]["structuredContent"]["code"],
        "project_admission_failed"
    );
    assert!(!other_root.join(".mobius").exists());

    let admitted = call_tool(
        &mut process,
        5,
        "mobius_project_init",
        json!({
            "project_root": admitted_root,
            "request_id": "fresh-host-context"
        }),
        &admitted_root,
    );
    assert_tool_success(&admitted);

    process.finish();
}

#[test]
fn malformed_and_oversized_frames_fail_once_then_the_next_frame_recovers() {
    let workspace = Workspace::new();
    let mut process = McpProcess::spawn(workspace.path());

    let cases: [&[u8]; 4] = [
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":",
        br#"{"jsonrpc":"2.0","id":2,"id":3,"method":"ping"}"#,
        br#"{"jsonrpc":"2.0","id":4,"method":"ping"} {}"#,
        b"\xff",
    ];
    for (index, frame) in cases.into_iter().enumerate() {
        process.write_raw_line(frame);
        process.write_value(&json!({
            "jsonrpc": "2.0",
            "id": 100 + index,
            "method": "ping",
            "params": {}
        }));

        let rejected = process.read_response();
        assert_eq!(rejected["id"], Value::Null);
        assert_eq!(rejected["error"]["code"], -32700);
        let recovered = process.read_response();
        assert_eq!(recovered["id"], 100 + index);
        assert_eq!(recovered["result"], json!({}));
    }

    let mut oversized =
        br#"{"jsonrpc":"2.0","id":9,"method":"ping","params":{"padding":""#.to_vec();
    oversized.resize(MAX_MESSAGE_BYTES + 1, b'a');
    oversized.extend_from_slice(br#""}}"#);
    process.write_raw_line(&oversized);
    process.write_value(&json!({
        "jsonrpc": "2.0",
        "id": 200,
        "method": "ping",
        "params": {}
    }));

    let rejected = process.read_response();
    assert_eq!(rejected["id"], Value::Null);
    assert_eq!(rejected["error"]["code"], -32700);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .expect("oversize error message")
            .contains("byte limit")
    );
    let recovered = process.read_response();
    assert_eq!(recovered["id"], 200);
    assert_eq!(recovered["result"], json!({}));

    process.finish();
}
