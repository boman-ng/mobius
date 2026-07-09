#!/usr/bin/env python3
"""Regression tests for the Mobius v0.5 contract."""

from __future__ import annotations

import importlib.util
import csv
import json
import subprocess
import sys
import types
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
PLUGIN_ROOT = REPO_ROOT / "plugins" / "mobius"
MOBIUS = PLUGIN_ROOT / "scripts" / "mobius.py"


def run(args: list[str], *, input_text: str | None = None, check: bool = True) -> dict[str, object]:
    result = subprocess.run(
        [sys.executable, str(MOBIUS), *args],
        input=input_text,
        text=True,
        capture_output=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise AssertionError(f"command failed {args}\nstdout={result.stdout}\nstderr={result.stderr}")
    return json.loads(result.stdout or "{}")


def compact(value: object) -> str:
    return json.dumps(value, separators=(",", ":"))


def ledger(root: Path, name: str, session_id: str = "s1", slug: str = "2026-07-09-demo") -> Path:
    return root.joinpath(".mobius", "runs", f"codex-session-{session_id}", slug, name)


def rows(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def create_locked_objective(root: Path, session_id: str = "s1", slug: str = "2026-07-09-demo") -> None:
    run(["--project-root", str(root), "objective-start", "--session-id", session_id, "--slug", slug, "--title", "Demo", "--user-request", "Deliver the demo Objective"])
    run(
        [
            "--project-root",
            str(root),
            "contract-add-work-item",
            "--session-id",
            session_id,
            "--objective-slug",
            slug,
            "--id",
            "W1",
            "--title",
            "Build",
            "--description",
            "Implement verifiable behavior",
            "--scope-json",
            compact({"allowed_paths": ["src/**", "tests/**"], "forbidden_paths": [".mobius/**"], "non_claims": [], "invariants": ["tests pass"]}),
            "--work-json",
            compact({"target_refs": ["src/**"], "deliverables": ["behavior"], "deleted_paths": []}),
            "--gate-json",
            compact({"entry": ["contract locked"], "exit": ["criterion passes"], "verifiers": ["test_result", "checkpoint_review"]}),
            "--recovery-json",
            compact({"rollback_boundary": "selected files", "restart_rule": "new Route Run", "escalation_rule": "no viable Route"}),
            "--timebox-json",
            compact({"route_run_timebox_ms": 1000, "budget_axis": "harness_internal_time"}),
            "--criteria-json",
            compact(
                [
                    {
                        "id": "C1",
                        "requirement": "Test command passes",
                        "observable_outcome": "command exits 0",
                        "evidence_required": [{"type": "test_result", "name": "pytest"}],
                        "verifier": [{"type": "test_result"}, {"type": "checkpoint_review"}, {"type": "exit_review"}],
                        "review_focus": [{"question": "Does proof cover the behavior?"}],
                        "required": True,
                    }
                ]
            ),
        ]
    )
    assert run(["--project-root", str(root), "contract-validate", "--session-id", session_id, "--objective-slug", slug])["ok"] is True
    assert run(["--project-root", str(root), "contract-lock", "--session-id", session_id, "--objective-slug", slug])["gate"] == "ready"


def create_checkpoint_passed_objective(root: Path, session_id: str = "s1", slug: str = "2026-07-09-demo") -> dict[str, object]:
    create_locked_objective(root, session_id, slug)
    run(["--project-root", str(root), "route-run-start", "--session-id", session_id, "--objective-slug", slug, "--work-item-id", "W1"])
    run(
        [
            "--project-root",
            str(root),
            "evidence-add",
            "--session-id",
            session_id,
            "--objective-slug",
            slug,
            "--type",
            "test_result",
            "--summary",
            "pytest passed",
            "--supports",
            "C1",
            "--artifact-json",
            compact({"type": "test_result", "name": "pytest", "exit_code": 0}),
        ]
    )
    target = run(["--project-root", str(root), "review-target-create", "--session-id", session_id, "--objective-slug", slug, "--review-mode", "checkpoint_review", "--work-item-id", "W1"])
    run(
        [
            "--project-root",
            str(root),
            "review-judgment-record",
            "--session-id",
            session_id,
            "--objective-slug",
            slug,
            "--review-target-id",
            target["review_target_id"],
            "--verdict",
            "pass",
            "--checked-criteria-json",
            compact(["C1"]),
            "--feedback-action",
            "none",
        ]
    )
    return run(["--project-root", str(root), "review-target-create", "--session-id", session_id, "--objective-slug", slug, "--review-mode", "exit_review"])


def test_objective_route_review_and_exit_verdict(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    first = run(["--project-root", str(tmp_path), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])
    assert first["next_required_action"] == "start_route_run"

    route = run(["--project-root", str(tmp_path), "route-run-start", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--work-item-id", "W1"])
    assert route["route_run_id"] == "route_run_001"
    budget = ledger(tmp_path, "budget.csv").read_text(encoding="utf-8")
    assert "harness_internal" in budget
    assert "route_run_started" in budget

    run(
        [
            "--project-root",
            str(tmp_path),
            "evidence-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--type",
            "test_result",
            "--summary",
            "pytest passed",
            "--supports",
            "C1",
            "--artifact-json",
            compact({"type": "test_result", "name": "pytest", "argv": ["pytest"], "cwd": ".", "exit_code": 0}),
        ]
    )
    target = run(["--project-root", str(tmp_path), "review-target-create", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--review-mode", "checkpoint_review", "--work-item-id", "W1"])
    assert target["review_target"]["schema"] == "mobius.review_target"
    judgment = run(
        [
            "--project-root",
            str(tmp_path),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            target["review_target_id"],
            "--verdict",
            "pass",
            "--checked-criteria-json",
            compact(["C1"]),
            "--feedback-action",
            "none",
        ]
    )
    assert judgment["persisted"] is True
    assert run(["--project-root", str(tmp_path), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])["next_required_action"] == "create_exit_review_target"

    exit_target = run(["--project-root", str(tmp_path), "review-target-create", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--review-mode", "exit_review"])
    run(
        [
            "--project-root",
            str(tmp_path),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            exit_target["review_target_id"],
            "--verdict",
            "pass",
            "--checked-criteria-json",
            compact(["C1"]),
            "--feedback-action",
            "none",
        ]
    )
    final = run(["--project-root", str(tmp_path), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])
    assert final["loop"]["terminal_verdict"] == "accepted"


def test_v0_5_ledger_schema(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    expected_ledgers = {
        "objective.md",
        "objective.csv",
        "work_items.csv",
        "criteria.csv",
        "routes.csv",
        "route_runs.csv",
        "budget.csv",
        "evidence.csv",
        "review_targets.csv",
        "review_judgments.csv",
        "review_runs.csv",
        "verdict.csv",
    }
    objective_path = ledger(tmp_path, "objective.csv").parent
    present = {path.name for path in objective_path.iterdir() if path.is_file()}
    assert expected_ledgers <= present
    for old_name in (
        "accept" + "ance.csv",
        "pack" + "ets.csv",
        "c" + "v.csv",
        "review_" + "attempts.csv",
    ):
        assert old_name not in present
    assert ledger(tmp_path, "budget.csv").read_text(encoding="utf-8").splitlines()[0].split(",") == [
        "schema",
        "id",
        "objective_id",
        "work_item_id",
        "criterion_id",
        "route_id",
        "route_run_id",
        "review_target_id",
        "review_run_id",
        "tool_call_id",
        "event_kind",
        "clock_domain",
        "metered",
        "source",
        "started_at",
        "finished_at",
        "duration_ms",
        "consumed_ms",
        "remaining_ms",
        "failure_kind",
        "created_at",
    ]


def test_retry_count_budgeting_is_rejected(tmp_path: Path) -> None:
    run(["--project-root", str(tmp_path), "objective-start", "--session-id", "s1", "--slug", "bad", "--title", "Bad", "--user-request", "Bad"])
    run(
        [
            "--project-root",
            str(tmp_path),
            "contract-add-work-item",
            "--session-id",
            "s1",
            "--objective-slug",
            "bad",
            "--id",
            "W1",
            "--title",
            "Bad",
            "--description",
            "Bad",
            "--scope-json",
            compact({"allowed_paths": ["src/**"], "forbidden_paths": [".mobius/**"]}),
            "--work-json",
            compact({"target_refs": ["src/**"], "deliverables": []}),
            "--gate-json",
            compact({"entry": [], "exit": [], "verifiers": []}),
            "--recovery-json",
            compact({"rollback_boundary": "", "restart_rule": "", "escalation_rule": ""}),
            "--timebox-json",
            compact({"route_run_timebox_ms": 1000, "max_stage_attempts": 3}),
            "--criteria-json",
            compact([{"id": "C1", "requirement": "x", "observable_outcome": "x", "evidence_required": [{"type": "command_result"}], "verifier": [{"type": "command_result"}]}]),
        ]
    )
    payload = run(["--project-root", str(tmp_path), "contract-validate", "--session-id", "s1", "--objective-slug", "bad"], check=False)
    assert payload["ok"] is False
    assert any("retry-count" in error for error in payload["errors"])


def test_budget_ledger_validation(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    bad_domain = run(
        [
            "--project-root",
            str(tmp_path),
            "budget-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--event-kind",
            "tool_call",
            "--clock-domain",
            "wall_clock",
            "--source",
            "test",
        ],
        check=False,
    )
    assert bad_domain["ok"] is False
    assert any("invalid clock domain" in error for error in bad_domain["errors"])
    bad_mixed = run(
        [
            "--project-root",
            str(tmp_path),
            "budget-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--event-kind",
            "tool_call",
            "--clock-domain",
            "mixed",
            "--metered",
            "true",
            "--source",
            "test",
            "--duration-ms",
            "50",
        ],
        check=False,
    )
    assert bad_mixed["ok"] is False
    assert any("explicit consumed_ms" in error for error in bad_mixed["errors"])


def test_mixed_tool_time_requires_explicit_consumption(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    bad = run(
        [
            "--project-root",
            str(tmp_path),
            "budget-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--event-kind",
            "tool_call",
            "--clock-domain",
            "mixed",
            "--metered",
            "true",
            "--source",
            "test",
            "--duration-ms",
            "50",
        ],
        check=False,
    )
    assert bad["ok"] is False
    good = run(
        [
            "--project-root",
            str(tmp_path),
            "budget-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--event-kind",
            "tool_call",
            "--clock-domain",
            "mixed",
            "--metered",
            "true",
            "--source",
            "test",
            "--duration-ms",
            "50",
            "--consumed-ms",
            "5",
        ]
    )
    assert good["budget_id"] == "budget_001"


def test_route_run_timebox_regression(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    route = run(["--project-root", str(tmp_path), "route-run-start", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--work-item-id", "W1"])
    route_run_id = str(route["route_run_id"])
    external = run(
        [
            "--project-root",
            str(tmp_path),
            "budget-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--event-kind",
            "tool_call",
            "--clock-domain",
            "external_detached",
            "--metered",
            "true",
            "--source",
            "detached-job",
            "--duration-ms",
            "5000",
            "--route-run-id",
            route_run_id,
        ]
    )
    assert external["route_run_consumed_ms"] == 0
    assert rows(ledger(tmp_path, "route_runs.csv"))[-1]["status"] == "running"
    harness = run(
        [
            "--project-root",
            str(tmp_path),
            "budget-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--event-kind",
            "model_generation",
            "--clock-domain",
            "harness_internal",
            "--metered",
            "true",
            "--source",
            "test",
            "--duration-ms",
            "1000",
            "--route-run-id",
            route_run_id,
        ]
    )
    assert harness["route_run_remaining_ms"] == 0
    route_run = rows(ledger(tmp_path, "route_runs.csv"))[-1]
    assert route_run["status"] == "expired"
    assert route_run["failure_kind"] == "timebox_expired"
    assert run(["--project-root", str(tmp_path), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])["next_required_action"] == "start_route_run"


def test_tool_time_accounting_regression(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    route = run(["--project-root", str(tmp_path), "route-run-start", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--work-item-id", "W1"])
    route_run_id = str(route["route_run_id"])
    unknown = run(
        [
            "--project-root",
            str(tmp_path),
            "budget-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--event-kind",
            "tool_call",
            "--clock-domain",
            "unknown",
            "--metered",
            "false",
            "--source",
            "unclassified-tool",
            "--duration-ms",
            "9000",
            "--consumed-ms",
            "9000",
            "--route-run-id",
            route_run_id,
        ]
    )
    assert unknown["route_run_consumed_ms"] == 0
    assert rows(ledger(tmp_path, "route_runs.csv"))[-1]["status"] == "running"
    blocking = run(
        [
            "--project-root",
            str(tmp_path),
            "budget-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--event-kind",
            "tool_call",
            "--clock-domain",
            "external_blocking",
            "--metered",
            "true",
            "--source",
            "slow-external-tool",
            "--duration-ms",
            "9000",
            "--consumed-ms",
            "9000",
            "--route-run-id",
            route_run_id,
        ]
    )
    assert blocking["route_run_consumed_ms"] == 0
    mixed = run(
        [
            "--project-root",
            str(tmp_path),
            "budget-add",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--event-kind",
            "tool_call",
            "--clock-domain",
            "mixed",
            "--metered",
            "true",
            "--source",
            "mixed-tool",
            "--duration-ms",
            "9000",
            "--consumed-ms",
            "1000",
            "--route-run-id",
            route_run_id,
        ]
    )
    assert mixed["route_run_consumed_ms"] == 1000
    assert rows(ledger(tmp_path, "route_runs.csv"))[-1]["status"] == "expired"


def test_codex_session_timing_import_regression(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    session_jsonl = tmp_path / "session.jsonl"
    session_jsonl.write_text(
        "\n".join(
            [
                json.dumps({"type": "ignored"}),
                json.dumps({"type": "message", "timestamp": "2026-07-09T00:00:00+00:00"}),
                json.dumps({"type": "tool_call", "started_at": "2026-07-09T00:00:00+00:00", "finished_at": "2026-07-09T00:00:01.500000+00:00", "tool_call_id": "tool_1"}),
                json.dumps({"type": "model_generation", "started_at": "2026-07-09T00:00:02+00:00", "duration_ms": 250, "tool_call_id": "model_1"}),
                json.dumps({"type": "tool_call", "clock_domain": "external_blocking", "started_at": "2026-07-09T00:00:03+00:00", "duration_ms": 5000, "tool_call_id": "tool_2"}),
                json.dumps({"type": "tool_call", "clock_domain": "external_detached", "started_at": "2026-07-09T00:00:04+00:00", "duration_ms": 5000, "tool_call_id": "tool_3"}),
                json.dumps({"type": "tool_call", "clock_domain": "mixed", "started_at": "2026-07-09T00:00:05+00:00", "duration_ms": 5000, "consumed_ms": 50, "tool_call_id": "tool_4"}),
                json.dumps({"type": "tool_call", "clock_domain": "mixed", "metered": True, "started_at": "2026-07-09T00:00:06+00:00", "duration_ms": 5000, "tool_call_id": "tool_5"}),
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    imported = run(["--project-root", str(tmp_path), "codex-session-import", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--session-jsonl", str(session_jsonl)])
    assert imported["imported_events"] == 7
    budget_rows = rows(ledger(tmp_path, "budget.csv"))
    by_tool = {row["tool_call_id"]: row for row in budget_rows if row["source"].startswith("codex_session_jsonl:")}
    message_row = next(row for row in budget_rows if row["event_kind"] == "message")
    assert "precision=timestamp_only" in message_row["source"]
    assert message_row["clock_domain"] == "unknown"
    assert message_row["metered"] == "false"
    assert "precision=paired_timestamps_ms" in by_tool["tool_1"]["source"]
    assert by_tool["tool_1"]["clock_domain"] == "unknown"
    assert by_tool["tool_1"]["duration_ms"] == "1500"
    assert by_tool["tool_1"]["consumed_ms"] == "0"
    assert by_tool["model_1"]["clock_domain"] == "harness_internal"
    assert by_tool["model_1"]["metered"] == "true"
    assert by_tool["model_1"]["consumed_ms"] == "250"
    assert by_tool["tool_2"]["clock_domain"] == "external_blocking"
    assert by_tool["tool_2"]["metered"] == "false"
    assert by_tool["tool_3"]["clock_domain"] == "external_detached"
    assert by_tool["tool_3"]["consumed_ms"] == "0"
    assert by_tool["tool_4"]["clock_domain"] == "mixed"
    assert by_tool["tool_4"]["metered"] == "true"
    assert by_tool["tool_4"]["consumed_ms"] == "50"
    assert by_tool["tool_5"]["clock_domain"] == "mixed"
    assert by_tool["tool_5"]["metered"] == "false"
    assert by_tool["tool_5"]["failure_kind"] == "missing_consumed_ms"


def load_review_module():
    class FakeFastMCP:
        def __init__(self, *args, **kwargs) -> None:
            pass

        def tool(self):
            def decorator(func):
                return func

            return decorator

        def run(self, *args, **kwargs) -> None:
            pass

    previous = {name: sys.modules.get(name) for name in ("mcp", "mcp.server", "mcp.server.fastmcp")}
    fake_mcp = types.ModuleType("mcp")
    fake_server = types.ModuleType("mcp.server")
    fake_fastmcp = types.ModuleType("mcp.server.fastmcp")
    fake_fastmcp.FastMCP = FakeFastMCP
    sys.modules["mcp"] = fake_mcp
    sys.modules["mcp.server"] = fake_server
    sys.modules["mcp.server.fastmcp"] = fake_fastmcp
    previous_dont_write_bytecode = sys.dont_write_bytecode
    sys.dont_write_bytecode = True
    try:
        spec = importlib.util.spec_from_file_location("mobius_review_under_test", PLUGIN_ROOT / "scripts" / "mobius_review_mcp.py")
        assert spec and spec.loader
        module = importlib.util.module_from_spec(spec)
        sys.modules["mobius_review_under_test"] = module
        spec.loader.exec_module(module)
        return module
    finally:
        for name, value in previous.items():
            if value is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = value
        sys.dont_write_bytecode = previous_dont_write_bytecode


def test_review_mcp_records_checkpoint_judgment(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    run(["--project-root", str(tmp_path), "route-run-start", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--work-item-id", "W1"])
    run(["--project-root", str(tmp_path), "evidence-add", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--type", "test_result", "--summary", "ok", "--supports", "C1", "--artifact-json", compact({"type": "test_result", "exit_code": 0})])
    target = run(["--project-root", str(tmp_path), "review-target-create", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--review-mode", "checkpoint_review", "--work-item-id", "W1"])
    module = load_review_module()
    reviewer_output = """MOBIUS_REVIEW_RESULT
REVIEWER: codex-subagent
REVIEW_MODE: checkpoint_review
VERDICT: pass
CHECKED_CRITERION_IDS: ["C1"]
UNCHECKED_CRITERION_IDS: []
BLOCKING_FINDINGS: []
REQUIRED_REVISIONS: []
EVIDENCE_CHECKED: ["evidence_001"]
FEEDBACK_ACTION: none
NOTES: ok
END_MOBIUS_REVIEW_RESULT"""
    recorded = module.mobius_review_record_checkpoint_judgment(str(tmp_path), "s1", "2026-07-09-demo", "W1", review_target=target["review_target"], codex_subagent_result=reviewer_output)
    assert recorded["ok"] is True
    assert recorded["persisted"] is True
    assert recorded["review_judgment_id"] == "review_judgment_001"


def test_contradictory_pass_judgment_is_rejected_before_state_mutation(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    run(["--project-root", str(tmp_path), "route-run-start", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--work-item-id", "W1"])
    run(["--project-root", str(tmp_path), "evidence-add", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--type", "test_result", "--summary", "ok", "--supports", "C1", "--artifact-json", compact({"type": "test_result", "exit_code": 0})])
    target = run(["--project-root", str(tmp_path), "review-target-create", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--review-mode", "checkpoint_review", "--work-item-id", "W1"])
    bad = run(
        [
            "--project-root",
            str(tmp_path),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            target["review_target_id"],
            "--verdict",
            "pass",
            "--checked-criteria-json",
            compact(["C1"]),
            "--blocking-findings-json",
            compact(["still ambiguous"]),
            "--required-revisions-json",
            compact(["add proof"]),
            "--feedback-action",
            "add_evidence",
        ],
        check=False,
    )
    assert bad["ok"] is False
    assert rows(ledger(tmp_path, "review_judgments.csv")) == []
    assert rows(ledger(tmp_path, "criteria.csv"))[-1]["status"] == "unknown"


def test_review_mcp_rejects_ambiguous_pass_result(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    run(["--project-root", str(tmp_path), "route-run-start", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--work-item-id", "W1"])
    run(["--project-root", str(tmp_path), "evidence-add", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--type", "test_result", "--summary", "ok", "--supports", "C1", "--artifact-json", compact({"type": "test_result", "exit_code": 0})])
    target = run(["--project-root", str(tmp_path), "review-target-create", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--review-mode", "checkpoint_review", "--work-item-id", "W1"])
    module = load_review_module()
    reviewer_output = """MOBIUS_REVIEW_RESULT
REVIEWER: codex-subagent
REVIEW_MODE: checkpoint_review
VERDICT: pass
CHECKED_CRITERION_IDS: ["C1"]
UNCHECKED_CRITERION_IDS: []
BLOCKING_FINDINGS: ["ambiguous evidence"]
REQUIRED_REVISIONS: []
EVIDENCE_CHECKED: ["evidence_001"]
FEEDBACK_ACTION: none
NOTES: contradictory
END_MOBIUS_REVIEW_RESULT"""
    recorded = module.mobius_review_record_checkpoint_judgment(str(tmp_path), "s1", "2026-07-09-demo", "W1", review_target=target["review_target"], codex_subagent_result=reviewer_output)
    assert recorded["ok"] is False
    assert recorded["persisted"] is False
    assert rows(ledger(tmp_path, "review_judgments.csv")) == []


def test_review_mcp_rejects_inline_target_mode_mismatch(tmp_path: Path) -> None:
    exit_target = create_checkpoint_passed_objective(tmp_path)
    checkpoint_target = rows(ledger(tmp_path, "review_targets.csv"))[0]
    checkpoint_inline = json.loads(checkpoint_target["target_json"])
    module = load_review_module()
    exit_reviewer_output = """MOBIUS_REVIEW_RESULT
REVIEWER: codex-subagent
REVIEW_MODE: exit_review
VERDICT: pass
CHECKED_CRITERION_IDS: ["C1"]
UNCHECKED_CRITERION_IDS: []
BLOCKING_FINDINGS: []
REQUIRED_REVISIONS: []
EVIDENCE_CHECKED: ["evidence_001"]
FEEDBACK_ACTION: none
NOTES: ok
END_MOBIUS_REVIEW_RESULT"""
    before = rows(ledger(tmp_path, "review_judgments.csv"))
    wrong_exit = module.mobius_review_record_exit_judgment(str(tmp_path), "s1", "2026-07-09-demo", review_target=checkpoint_inline, codex_subagent_result=exit_reviewer_output)
    assert wrong_exit["ok"] is False
    assert wrong_exit["persisted"] is False
    assert "mode mismatch" in wrong_exit["errors"][0]
    assert rows(ledger(tmp_path, "review_judgments.csv")) == before

    checkpoint_reviewer_output = """MOBIUS_REVIEW_RESULT
REVIEWER: codex-subagent
REVIEW_MODE: checkpoint_review
VERDICT: pass
CHECKED_CRITERION_IDS: ["C1"]
UNCHECKED_CRITERION_IDS: []
BLOCKING_FINDINGS: []
REQUIRED_REVISIONS: []
EVIDENCE_CHECKED: ["evidence_001"]
FEEDBACK_ACTION: none
NOTES: ok
END_MOBIUS_REVIEW_RESULT"""
    wrong_checkpoint = module.mobius_review_record_checkpoint_judgment(str(tmp_path), "s1", "2026-07-09-demo", "W1", review_target=exit_target["review_target"], codex_subagent_result=checkpoint_reviewer_output)
    assert wrong_checkpoint["ok"] is False
    assert wrong_checkpoint["persisted"] is False
    assert "mode mismatch" in wrong_checkpoint["errors"][0]
    assert rows(ledger(tmp_path, "review_judgments.csv")) == before


def test_review_target_one_shot_regression(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    run(["--project-root", str(tmp_path), "route-run-start", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--work-item-id", "W1"])
    run(["--project-root", str(tmp_path), "evidence-add", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--type", "test_result", "--summary", "ok", "--supports", "C1", "--artifact-json", compact({"type": "test_result", "exit_code": 0})])
    target = run(["--project-root", str(tmp_path), "review-target-create", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--review-mode", "checkpoint_review", "--work-item-id", "W1"])
    argv = [
        "--project-root",
        str(tmp_path),
        "review-judgment-record",
        "--session-id",
        "s1",
        "--objective-slug",
        "2026-07-09-demo",
        "--review-target-id",
        target["review_target_id"],
        "--verdict",
        "unknown",
        "--checked-criteria-json",
        compact([]),
        "--feedback-action",
        "add_evidence",
    ]
    assert run(argv)["persisted"] is True
    second = run(argv, check=False)
    assert second["ok"] is False
    assert any("already has a judgment" in error for error in second["errors"])


def test_review_judgment_classification_regression(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    run(["--project-root", str(tmp_path), "route-run-start", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--work-item-id", "W1"])
    run(["--project-root", str(tmp_path), "evidence-add", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--type", "test_result", "--summary", "ok", "--supports", "C1", "--artifact-json", compact({"type": "test_result", "exit_code": 0})])
    target = run(["--project-root", str(tmp_path), "review-target-create", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--review-mode", "checkpoint_review", "--work-item-id", "W1"])
    run(
        [
            "--project-root",
            str(tmp_path),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            target["review_target_id"],
            "--verdict",
            "unknown",
            "--checked-criteria-json",
            compact([]),
            "--feedback-action",
            "retry_review",
        ]
    )
    review_run = rows(ledger(tmp_path, "review_runs.csv"))[-1]
    assert review_run["failure_kind"] == "review_infrastructure"
    assert review_run["retryable"] == "true"
    assert run(["--project-root", str(tmp_path), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])["next_required_action"] == "create_review_target"


def test_review_feedback_loop_regression(tmp_path: Path) -> None:
    create_locked_objective(tmp_path)
    run(["--project-root", str(tmp_path), "route-run-start", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--work-item-id", "W1"])
    run(["--project-root", str(tmp_path), "evidence-add", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--type", "test_result", "--summary", "initial evidence", "--supports", "C1", "--artifact-json", compact({"type": "test_result", "exit_code": 0})])
    target = run(["--project-root", str(tmp_path), "review-target-create", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--review-mode", "checkpoint_review", "--work-item-id", "W1"])
    run(
        [
            "--project-root",
            str(tmp_path),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            target["review_target_id"],
            "--verdict",
            "fail",
            "--checked-criteria-json",
            compact([]),
            "--feedback-action",
            "add_evidence",
        ]
    )
    feedback = run(["--project-root", str(tmp_path), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])
    assert feedback["next_required_action"] == "add_evidence"
    assert feedback["loop"]["review_feedback_action"] == "add_evidence"
    run(["--project-root", str(tmp_path), "evidence-add", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--type", "test_result", "--summary", "expanded evidence", "--supports", "C1", "--artifact-json", compact({"type": "test_result", "exit_code": 0, "case": "expanded"})])
    assert run(["--project-root", str(tmp_path), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])["next_required_action"] == "create_review_target"


def test_exit_review_feedback_loop_regression(tmp_path: Path) -> None:
    add_root = tmp_path / "add"
    add_root.mkdir()
    add_target = create_checkpoint_passed_objective(add_root)
    run(
        [
            "--project-root",
            str(add_root),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            add_target["review_target_id"],
            "--verdict",
            "fail",
            "--checked-criteria-json",
            compact([]),
            "--feedback-action",
            "add_evidence",
        ]
    )
    feedback = run(["--project-root", str(add_root), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])
    assert feedback["next_required_action"] == "add_evidence"
    assert feedback["loop"]["review_feedback_action"] == "add_evidence"
    assert len([row for row in rows(ledger(add_root, "review_targets.csv")) if row["review_mode"] == "exit_review"]) == 1
    run(["--project-root", str(add_root), "evidence-add", "--session-id", "s1", "--objective-slug", "2026-07-09-demo", "--type", "test_result", "--summary", "exit review evidence", "--supports", "C1", "--artifact-json", compact({"type": "test_result", "exit": "expanded"})])
    assert run(["--project-root", str(add_root), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])["next_required_action"] == "create_exit_review_target"

    repair_root = tmp_path / "repair"
    repair_root.mkdir()
    repair_target = create_checkpoint_passed_objective(repair_root)
    run(
        [
            "--project-root",
            str(repair_root),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            repair_target["review_target_id"],
            "--verdict",
            "fail",
            "--checked-criteria-json",
            compact([]),
            "--feedback-action",
            "repair_route",
        ]
    )
    repair = run(["--project-root", str(repair_root), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])
    assert repair["next_required_action"] == "repair_route"
    assert repair["loop"]["review_feedback_action"] == "repair_route"

    retry_root = tmp_path / "retry"
    retry_root.mkdir()
    retry_target = create_checkpoint_passed_objective(retry_root)
    run(
        [
            "--project-root",
            str(retry_root),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            retry_target["review_target_id"],
            "--verdict",
            "unknown",
            "--checked-criteria-json",
            compact([]),
            "--feedback-action",
            "retry_review",
        ]
    )
    retry = run(["--project-root", str(retry_root), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])
    assert retry["next_required_action"] == "create_exit_review_target"
    assert retry["loop"]["review_feedback_action"] == "retry_review"

    route_root = tmp_path / "route"
    route_root.mkdir()
    route_target = create_checkpoint_passed_objective(route_root)
    run(
        [
            "--project-root",
            str(route_root),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            route_target["review_target_id"],
            "--verdict",
            "fail",
            "--checked-criteria-json",
            compact([]),
            "--feedback-action",
            "select_alternate_route",
        ]
    )
    route = run(["--project-root", str(route_root), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])
    assert route["next_required_action"] == "select_alternate_route"
    assert route["loop"]["next_work_item_id"] == "W1"
    run(["--project-root", str(route_root), *route["loop"]["next_argv"]])
    assert run(["--project-root", str(route_root), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])["next_required_action"] == "create_exit_review_target"

    contract_root = tmp_path / "contract"
    contract_root.mkdir()
    contract_target = create_checkpoint_passed_objective(contract_root)
    run(
        [
            "--project-root",
            str(contract_root),
            "review-judgment-record",
            "--session-id",
            "s1",
            "--objective-slug",
            "2026-07-09-demo",
            "--review-target-id",
            contract_target["review_target_id"],
            "--verdict",
            "blocked",
            "--checked-criteria-json",
            compact([]),
            "--feedback-action",
            "contract_change_required",
        ]
    )
    terminal = run(["--project-root", str(contract_root), "continue", "--session-id", "s1", "--objective-slug", "2026-07-09-demo"])
    assert terminal["loop"]["terminal_verdict"] == "blocked"


def test_hook_blocks_direct_protected_ledger_write(tmp_path: Path) -> None:
    protected = ledger(tmp_path, "objective.csv", slug="x")
    payload = {"command": f"python -c \"open('{protected}','w').write('bad')\""}
    result = subprocess.run(
        [sys.executable, str(MOBIUS), "--project-root", str(tmp_path), "hook", "pre-tool-use"],
        input=json.dumps(payload),
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 2
    assert "protected-ledger" in result.stderr


def test_hook_allows_read_only_protected_ledger_inspection(tmp_path: Path) -> None:
    create_locked_objective(tmp_path, slug="read")
    protected = ledger(tmp_path, "objective.csv", slug="read")
    payload = {"command": f"cat '{protected}'"}
    result = subprocess.run(
        [sys.executable, str(MOBIUS), "--project-root", str(tmp_path), "hook", "pre-tool-use"],
        input=json.dumps(payload),
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0
    assert "protected-ledger" not in result.stderr
