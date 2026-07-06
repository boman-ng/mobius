#!/usr/bin/env python3
"""Focused regression tests for Mobius' current gate contract."""

from __future__ import annotations

import csv
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import types
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
PLUGIN_ROOT = REPO_ROOT / "plugins" / "mobius"
sys.path.insert(0, str(PLUGIN_ROOT / "scripts"))

import mobius


ROOT = PLUGIN_ROOT
MOBIUS = ROOT / "scripts" / "mobius.py"


def load_mobius_cv_mcp_module():
    class FakeFastMCP:
        def __init__(self, *args, **kwargs) -> None:
            pass

        def tool(self):
            def decorator(func):
                return func

            return decorator

        def run(self, *args, **kwargs) -> None:
            pass

    previous = {
        name: sys.modules.get(name)
        for name in ("mcp", "mcp.server", "mcp.server.fastmcp")
    }
    previous_child = os.environ.get("MOBIUS_CV_KIMI_CHILD")
    fake_mcp = types.ModuleType("mcp")
    fake_server = types.ModuleType("mcp.server")
    fake_fastmcp = types.ModuleType("mcp.server.fastmcp")
    fake_fastmcp.FastMCP = FakeFastMCP
    sys.modules["mcp"] = fake_mcp
    sys.modules["mcp.server"] = fake_server
    sys.modules["mcp.server.fastmcp"] = fake_fastmcp
    os.environ["MOBIUS_CV_KIMI_CHILD"] = "1"
    try:
        spec = importlib.util.spec_from_file_location(
            "mobius_cv_mcp_under_test",
            ROOT / "scripts" / "mobius_cv_mcp.py",
        )
        if spec is None or spec.loader is None:
            raise AssertionError("could not load mobius_cv_mcp.py")
        module = importlib.util.module_from_spec(spec)
        sys.modules["mobius_cv_mcp_under_test"] = module
        spec.loader.exec_module(module)
        return module
    finally:
        if previous_child is None:
            os.environ.pop("MOBIUS_CV_KIMI_CHILD", None)
        else:
            os.environ["MOBIUS_CV_KIMI_CHILD"] = previous_child
        for name, value in previous.items():
            if value is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = value


def run(args: list[str], *, input_text: str | None = None, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        [sys.executable, str(MOBIUS), *args],
        input=input_text,
        text=True,
        capture_output=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise AssertionError(f"command failed {args}\nstdout={result.stdout}\nstderr={result.stderr}")
    return result


def compact_json(value: object) -> str:
    return json.dumps(value, separators=(",", ":"))


def assert_loop_action_is_mirrored(result: dict[str, object]) -> None:
    loop = result.get("loop")
    if isinstance(loop, dict):
        assert result.get("next_required_action") == loop.get("next_required_action")


def assert_required_loop_action_is_mirrored(result: dict[str, object]) -> dict[str, object]:
    loop = result.get("loop")
    assert isinstance(loop, dict)
    assert result.get("next_required_action") == loop.get("next_required_action")
    assert result.get("packet_id", "") == loop.get("packet_id", "")
    assert result.get("review_mode", "") == loop.get("review_mode", "")
    return loop


def goal_dir(root: Path, session_id: str, suffix: str) -> tuple[str, Path]:
    run_dir = root / ".mobius" / "runs" / f"codex-session-{session_id}"
    matches = sorted(path for path in run_dir.iterdir() if path.is_dir() and path.name.endswith(suffix))
    if not matches:
        raise AssertionError(f"missing goal ending with {suffix}")
    return matches[-1].name, matches[-1]


def replace_arg(args: list[str], option: str, value: str) -> list[str]:
    updated = list(args)
    updated[updated.index(option) + 1] = value
    return updated


def remove_options(args: list[str], options: set[str]) -> list[str]:
    updated: list[str] = []
    skip_next = False
    for item in args:
        if skip_next:
            skip_next = False
            continue
        if item in options:
            skip_next = True
            continue
        updated.append(item)
    return updated


def stage_contract_args(
    root: Path,
    session_id: str,
    slug: str,
    plan_id: str = "P1",
    acceptance_id: str = "A1",
    depends: list[str] | None = None,
) -> list[str]:
    return [
        "--project-root",
        str(root),
        "contract-add-stage",
        "--session-id",
        session_id,
        "--goal-slug",
        slug,
        "--id",
        plan_id,
        "--title",
        "Implement",
        "--description",
        "Implement verifiable behavior",
        "--depends-on-json",
        compact_json(depends or []),
        "--scope-json",
        compact_json(
            {
                "allowed_paths": ["scripts/**", "tests/**"],
                "forbidden_paths": [".mobius/**"],
                "non_goals": ["Do not change unrelated behavior"],
                "invariants": ["contract validation passes"],
                "side_effect_level": "local",
            }
        ),
        "--work-json",
        compact_json(
            {
                "target_refs": ["scripts/mobius.py"],
                "deliverables": ["verifiable behavior"],
                "deleted_paths": [],
            }
        ),
        "--gate-json",
        compact_json(
            {
                "entry": ["contract locked"],
                "exit": ["test evidence command exits 0"],
                "verifiers": ["command_result", "mobiuscv_delta"],
                "review_focus": ["proof obligations"],
            }
        ),
        "--recovery-json",
        compact_json(
            {
                "rollback_boundary": "revert stage files",
                "restart_rule": "restart from pending stage",
                "escalation_rule": "surface blocker",
            }
        ),
        "--budget-json",
        compact_json(
            {
                "retry_limit": 2,
                "max_stage_attempts": 3,
                "stop_condition": "recorded review blocks or passes",
            }
        ),
        "--acceptance-json",
        compact_json(
            [
                {
                    "id": acceptance_id,
                    "requirement": "Command proof is recorded",
                    "observable_outcome": "evidence.csv contains command_result proof row with exit code 0",
                    "evidence_required": [{"type": "command_result", "name": "test evidence", "exit_code": 0}],
                    "verifier": [{"type": "command_result", "name": "test evidence"}, {"type": "mobiuscv_delta"}],
                    "review_focus": ["evidence_required_json is satisfied"],
                    "required": True,
                }
            ]
        ),
    ]


def add_stage(
    root: Path,
    session_id: str,
    slug: str,
    plan_id: str = "P1",
    acceptance_id: str = "A1",
    depends: list[str] | None = None,
) -> dict[str, object]:
    return json.loads(run(stage_contract_args(root, session_id, slug, plan_id, acceptance_id, depends)).stdout)


def human_assertion_stage_args(root: Path, session_id: str, slug: str) -> list[str]:
    command = stage_contract_args(root, session_id, slug)
    command[command.index("--gate-json") + 1] = compact_json(
        {
            "entry": ["contract locked"],
            "exit": ["human assertion evidence is recorded"],
            "verifiers": ["human_assertion", "mobiuscv_delta"],
            "review_focus": ["human assertion exists"],
        }
    )
    command[command.index("--acceptance-json") + 1] = compact_json(
        [
            {
                "id": "A1",
                "requirement": "Human assertion is recorded",
                "observable_outcome": "evidence.csv contains human_assertion proof row",
                "evidence_required": [{"type": "human_assertion"}],
                "verifier": [{"type": "human_assertion"}, {"type": "mobiuscv_delta"}],
                "review_focus": ["human assertion proof is present"],
                "required": True,
            }
        ]
    )
    return command


def read_goal_id(goal: Path) -> str:
    rows = list(csv.DictReader((goal / "goal.csv").open(encoding="utf-8")))
    return rows[0]["goal_id"]


def prepare_goal(root: Path, session_id: str, slug_suffix: str) -> tuple[str, Path, str]:
    run(["--project-root", str(root), "init", "--session-id", session_id])
    run(
        [
            "--project-root",
            str(root),
            "goal-start",
            "--session-id",
            session_id,
            "--slug",
            slug_suffix,
            "--title",
            slug_suffix,
            "--user-goal",
            slug_suffix,
        ]
    )
    slug, goal = goal_dir(root, session_id, slug_suffix)
    add_stage(root, session_id, slug)
    run(["--project-root", str(root), "contract-lock", "--session-id", session_id, "--goal-slug", slug])
    evidence = json.loads(
        run(
            [
                "--project-root",
                str(root),
                "evidence-add",
                "--session-id",
                session_id,
                "--goal-slug",
                slug,
                "--type",
                "command_result",
                "--summary",
                "test evidence",
                "--supports",
                "A1",
                "--artifact-json",
                compact_json({"type": "command_result", "name": "test evidence", "command": "pytest", "exit_code": 0}),
            ]
        ).stdout
    )
    return slug, goal, evidence["evidence_id"]


def prepare_unlocked_goal(root: Path, session_id: str, slug_suffix: str) -> tuple[str, Path]:
    run(["--project-root", str(root), "init", "--session-id", session_id])
    run(
        [
            "--project-root",
            str(root),
            "goal-start",
            "--session-id",
            session_id,
            "--slug",
            slug_suffix,
            "--title",
            slug_suffix,
            "--user-goal",
            slug_suffix,
        ]
    )
    slug, goal = goal_dir(root, session_id, slug_suffix)
    add_stage(root, session_id, slug)
    return slug, goal


def create_packet(root: Path, session_id: str, slug: str, review_mode: str, acceptance_ids: list[str] | None = None) -> dict[str, object]:
    command = [
        "--project-root",
        str(root),
        "packet-create",
        "--session-id",
        session_id,
        "--goal-slug",
        slug,
        "--review-mode",
        review_mode,
    ]
    for acceptance_id in acceptance_ids or []:
        command.extend(["--acceptance-id", acceptance_id])
    return json.loads(run(command).stdout)["packet"]


def start_stage(root: Path, session_id: str, slug: str, plan_item_id: str = "P1") -> None:
    result = json.loads(
        run(
            [
                "--project-root",
                str(root),
                "loop-start-stage",
                "--session-id",
                session_id,
                "--goal-slug",
                slug,
                "--plan-item-id",
                plan_item_id,
            ]
        ).stdout
    )
    assert result["row"]["status"] == "running"


def write_delta_packet_row(root: Path, goal: Path, slug: str, packet_id: str = "packet_delta_001") -> dict[str, object]:
    packet = mobius.packet_envelope(root, goal, packet_id, slug, "delta_review", "P1", ["A1"])
    row = {
        "schema": "mobius.packet",
        "packet_id": packet_id,
        "goal_id": read_goal_id(goal),
        "goal_slug": slug,
        "review_mode": "delta_review",
        "stateless": mobius.as_bool_cell(True),
        "scope": "P1",
        "created_at": "2026-07-01T00:00:00+00:00",
        "packet_json": mobius.as_json_cell(packet),
        "packet_sha256": "",
    }
    row["packet_sha256"] = mobius.packet_hash(packet)
    mobius.write_csv_rows(goal / "packets.csv", mobius.PACKET_FIELDS, [row])
    return packet


def reviewer_result(reviewer_id: str, review_mode: str, checked_ids: list[str], verdict: str = "pass") -> dict[str, object]:
    return {
        "schema": "mobius.cv_reviewer_result",
        "reviewer_id": reviewer_id,
        "review_mode": review_mode,
        "status": "completed",
        "verdict": verdict,
        "checked_acceptance_ids": checked_ids,
        "unchecked_acceptance_ids": [],
        "blocking_findings": [],
        "required_revisions": [],
        "evidence_checked": ["plan.csv", "acceptance.csv", "evidence.csv"],
    }


def cv_envelope(
    cv_id: str,
    goal_id: str,
    overall: str = "pass",
    degraded: list[str] | None = None,
    packet_id: str = "packet_exit_001",
    reviewer_verdict: str | None = None,
) -> dict[str, object]:
    degraded = degraded or []
    reviewer_verdict = reviewer_verdict or overall
    policy = mobius.review_gate_policy("exit_review", {"level": 2})
    reviewers = [
        {
            **reviewer_result("codex-subagent", "exit_review", ["A1"], reviewer_verdict),
            "status": "completed" if "codex-subagent" not in degraded else "timeout",
        },
        {
            **reviewer_result("kimi-code", "exit_review", ["A1"], reviewer_verdict),
            "status": "completed" if "kimi-code" not in degraded else "timeout",
        },
    ]
    derived = mobius.derive_cv_aggregate(reviewers, ["A1"], "exit_review", policy)
    return {
        "schema": "mobius.cv_result",
        "cv_id": cv_id,
        "goal_id": goal_id,
        "packet_id": packet_id,
        "review_mode": "exit_review",
        "level": 2,
        "stateless": True,
        "reviewers": reviewers,
        "comparison": {
            "agreement": derived["agreement"],
            "reviewer_verdicts": derived["reviewer_verdicts"],
            "degraded_reviewers": derived["degraded_reviewers"],
        },
        "result": {
            "overall": derived["overall"] if reviewer_verdict == overall else overall,
            "checked_acceptance_ids": derived["checked_acceptance_ids"],
            "unchecked_acceptance_ids": derived["unchecked_acceptance_ids"],
            "blocking_findings": derived["blocking_findings"],
            "required_revisions": derived["required_revisions"],
        },
        "input_refs": {"review_policy": policy},
        "returned_at": "2026-07-01T00:00:00+00:00",
    }


def exit_cv_from_reviewers(cv_id: str, goal_id: str, reviewers: list[dict[str, object]], packet_id: str) -> dict[str, object]:
    policy = mobius.review_gate_policy("exit_review", {"level": 2})
    derived = mobius.derive_cv_aggregate(reviewers, ["A1"], "exit_review", policy)
    return {
        "schema": "mobius.cv_result",
        "cv_id": cv_id,
        "goal_id": goal_id,
        "packet_id": packet_id,
        "review_mode": "exit_review",
        "level": 2,
        "stateless": True,
        "reviewers": reviewers,
        "comparison": {
            "agreement": derived["agreement"],
            "reviewer_verdicts": derived["reviewer_verdicts"],
            "degraded_reviewers": derived["degraded_reviewers"],
        },
        "result": {
            "overall": derived["overall"],
            "checked_acceptance_ids": derived["checked_acceptance_ids"],
            "unchecked_acceptance_ids": derived["unchecked_acceptance_ids"],
            "blocking_findings": derived["blocking_findings"],
            "required_revisions": derived["required_revisions"],
        },
        "input_refs": {"review_policy": policy},
        "returned_at": "2026-07-01T00:00:00+00:00",
    }


def delta_cv_envelope(
    cv_id: str,
    goal_id: str,
    overall: str = "pass",
    packet_id: str = "packet_delta_001",
    reviewer_verdict: str | None = None,
    level: int = 2,
    policy_name: str = "delta_kimi",
) -> dict[str, object]:
    reviewer_verdict = reviewer_verdict or overall
    policy = mobius.review_gate_policy("delta_review", {"name": policy_name})
    reviewers = [reviewer_result("codex-subagent", "delta_review", ["A1"], reviewer_verdict)]
    if "kimi-code" in policy["required_reviewers"]:
        reviewers.append(reviewer_result("kimi-code", "delta_review", ["A1"], reviewer_verdict))
    derived = mobius.derive_cv_aggregate(reviewers, ["A1"], "delta_review", policy)
    return {
        "schema": "mobius.cv_result",
        "cv_id": cv_id,
        "goal_id": goal_id,
        "packet_id": packet_id,
        "review_mode": "delta_review",
        "level": max(level, int(policy["minimum_level"])),
        "stateless": True,
        "reviewers": reviewers,
        "comparison": {
            "agreement": derived["agreement"],
            "reviewer_verdicts": derived["reviewer_verdicts"],
            "degraded_reviewers": derived["degraded_reviewers"],
        },
        "result": {
            "overall": derived["overall"] if reviewer_verdict == overall else overall,
            "checked_acceptance_ids": derived["checked_acceptance_ids"],
            "unchecked_acceptance_ids": derived["unchecked_acceptance_ids"],
            "blocking_findings": derived["blocking_findings"],
            "required_revisions": derived["required_revisions"],
        },
        "input_refs": {"review_policy": policy},
        "returned_at": "2026-07-01T00:00:00+00:00",
    }


def delta_cv_with_revisions(
    cv_id: str,
    goal_id: str,
    revisions: list[str],
    packet_id: str = "packet_delta_001",
    policy_name: str = "delta_kimi",
) -> dict[str, object]:
    cv = delta_cv_envelope(cv_id, goal_id, overall="fail", packet_id=packet_id, reviewer_verdict="fail", policy_name=policy_name)
    policy = mobius.review_gate_policy("delta_review", {"name": policy_name})
    reviewers = cv["reviewers"]
    assert isinstance(reviewers, list)
    for reviewer in reviewers:
        reviewer["required_revisions"] = revisions
    derived = mobius.derive_cv_aggregate(reviewers, ["A1"], "delta_review", policy)
    cv["comparison"] = {
        "agreement": derived["agreement"],
        "reviewer_verdicts": derived["reviewer_verdicts"],
        "degraded_reviewers": derived["degraded_reviewers"],
    }
    cv["result"] = {
        "overall": derived["overall"],
        "checked_acceptance_ids": derived["checked_acceptance_ids"],
        "unchecked_acceptance_ids": derived["unchecked_acceptance_ids"],
        "blocking_findings": derived["blocking_findings"],
        "required_revisions": derived["required_revisions"],
    }
    return cv


def terminal_goal(root: Path, session_id: str, slug_suffix: str, overall: str = "pass") -> tuple[str, Path, dict[str, object]]:
    slug, goal, _evidence = prepare_goal(root, session_id, slug_suffix)
    packet = create_packet(root, session_id, slug, "exit_review")
    recorded = mobius.record_cv_result(
        root,
        session_id,
        slug,
        cv_envelope("cv_exit_001", read_goal_id(goal), overall=overall, packet_id=str(packet["packet"])),
        "exit_review",
    )
    assert recorded["gate"] == {"pass": "accepted", "blocked": "blocked"}[overall]
    return slug, goal, packet


def test_plan_loop_packet_smoke_path() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        result = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "goal-start",
                    "--session-id",
                    "s",
                    "--slug",
                    "smoke",
                    "--title",
                    "smoke",
                    "--user-goal",
                    "smoke",
                ]
            ).stdout
        )
        slug = result["goal_slug"]
        goal = Path(result["goal_dir"])
        assert list(csv.reader((goal / "goal.csv").open(encoding="utf-8")))[0] == mobius.GOAL_FIELDS
        goal_state = list(csv.DictReader((goal / "goal.csv").open(encoding="utf-8")))[0]
        assert goal_state["contract_path"] == "goal.md"
        assert len(goal_state["contract_sha256_tail"]) == 7
        goal_contract = (goal / "goal.md").read_text(encoding="utf-8")
        assert 'schema = "mobius.goal_contract"' in goal_contract
        assert "## User Goal" in goal_contract
        assert list(csv.reader((goal / "plan.csv").open(encoding="utf-8")))[0] == mobius.PLAN_FIELDS
        assert list(csv.reader((goal / "acceptance.csv").open(encoding="utf-8")))[0] == mobius.ACCEPTANCE_FIELDS
        for command in (
            stage_contract_args(root, "s", slug),
            ["--project-root", str(root), "contract-validate", "--session-id", "s", "--goal-slug", slug],
            ["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug],
        ):
            envelope = json.loads(run(command).stdout)
            assert envelope["schema"] == "mobius.command_result"
            assert envelope["ok"] is True
        locked_contract = (goal / "goal.md").read_text(encoding="utf-8")
        assert "locked_at = \"\"" not in locked_contract
        start_stage(root, "s", slug)
        evidence = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "evidence-add",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--type",
                    "command_result",
                    "--summary",
                    "test evidence",
                    "--supports",
                    "A1",
                    "--artifact-json",
                    compact_json({"type": "command_result", "name": "test evidence", "command": "pytest", "exit_code": 0}),
                ]
            ).stdout
        )
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        assert evidence["evidence_id"] == "E1"
        assert packet["coverage"]["A1"] == ["E1"]
        assert packet["refs"]["E1"][0] == "command_result"
        assert packet["refs"]["E1"][2].startswith("h:")
        assert "content" not in json.dumps(packet)


def test_contract_defaults_are_explicit_and_lockable() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "defaults", "--title", "defaults", "--user-goal", "defaults"])
        slug, goal = goal_dir(root, "s", "defaults")
        missing = run(remove_options(stage_contract_args(root, "s", slug), {"--scope-json"}), check=False)
        assert missing.returncode == 2
        assert "missing required JSON arguments: scope-json" in json.loads(missing.stdout)["errors"]

        command = remove_options(
            stage_contract_args(root, "s", slug),
            {"--depends-on-json", "--scope-json", "--gate-json", "--recovery-json", "--budget-json"},
        )
        command.extend(["--contract-defaults", "local"])
        assert json.loads(run(command).stdout)["ok"] is True
        row = list(csv.DictReader((goal / "plan.csv").open(encoding="utf-8")))[0]
        assert json.loads(row["scope_json"])["forbidden_paths"] == [".mobius/**"]
        assert json.loads(run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug]).stdout)["ok"] is True


def test_contract_lock_rejects_zero_stage_goal() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "empty", "--title", "empty", "--user-goal", "empty"])
        result = run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", "empty"], check=False)
        assert result.returncode == 2
        payload = json.loads(result.stdout)
        assert payload["next_required_action"] == "fix_contract"
        assert any("at least one active required plan item" in error for error in payload["errors"])


def test_contract_add_stage_rejects_required_stage_without_required_acceptance() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "optional-only", "--title", "optional-only", "--user-goal", "optional-only"])
        slug, _goal = goal_dir(root, "s", "optional-only")
        args = stage_contract_args(root, "s", slug)
        acceptance = json.loads(args[args.index("--acceptance-json") + 1])
        acceptance[0]["required"] = False
        args[args.index("--acceptance-json") + 1] = compact_json(acceptance)
        result = run(args, check=False)
        assert result.returncode == 2
        payload = json.loads(result.stdout)
        assert payload["next_required_action"] == "fix_contract"
        assert any("required plan item must link at least one required acceptance id" in error for error in payload["errors"])


def test_locked_goal_contract_is_frozen() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "frozen-goal")
        before = (goal / "goal.md").read_text(encoding="utf-8")
        result = run(
            [
                "--project-root",
                str(root),
                "goal-start",
                "--session-id",
                "s",
                "--slug",
                "frozen-goal",
                "--title",
                "mutated",
                "--user-goal",
                "mutated",
            ],
            check=False,
        )
        assert result.returncode == 2
        assert "cannot modify an active or locked goal contract" in json.loads(result.stdout)["errors"][0]
        assert (goal / "goal.md").read_text(encoding="utf-8") == before
        second_lock = json.loads(run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug]).stdout)
        assert second_lock["ok"] is True
        assert "goal.md" not in second_lock["updated_files"]
        assert (goal / "goal.md").read_text(encoding="utf-8") == before

        tampered = before.replace('locked_at = "', 'locked_at = "" # ')
        (goal / "goal.md").write_text(tampered, encoding="utf-8")
        result = json.loads(run(["--project-root", str(root), "contract-validate", "--session-id", "s", "--goal-slug", slug], check=False).stdout)
        assert any("locked_at is required" in error or "contract_sha256_tail mismatch" in error for error in result["errors"])


def test_contract_validation_rejects_bad_acceptance_and_verifiers() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "bad-contract", "--title", "bad", "--user-goal", "bad"])
        slug, _goal = goal_dir(root, "s", "bad-contract")

        bad_acceptance = [
            {
                "id": "A1",
                "requirement": "Complete implementation is robust",
                "observable_outcome": "looks appropriate",
                "evidence_required": [],
                "verifier": [],
                "review_focus": [],
            }
        ]
        result = run(replace_arg(stage_contract_args(root, "s", slug), "--acceptance-json", compact_json(bad_acceptance)), check=False)
        errors = json.loads(result.stdout)["errors"]
        assert any("evidence_required_json" in error for error in errors)
        assert any("verifier_json" in error for error in errors)
        assert any("vague requirement" in error for error in errors)

        mobiuscv_evidence = [
            {
                "id": "A1",
                "requirement": "No hidden behavior is introduced",
                "observable_outcome": "change-set scope evidence and delta review show no hidden behavior",
                "evidence_required": [{"type": "mobiuscv_delta", "name": "delta review"}],
                "verifier": [{"type": "mobiuscv_delta"}],
                "review_focus": ["hidden behavior"],
                "required": True,
            }
        ]
        result = run(replace_arg(stage_contract_args(root, "s", slug), "--acceptance-json", compact_json(mobiuscv_evidence)), check=False)
        assert any("unsupported evidence type" in error and "mobiuscv_delta" in error for error in json.loads(result.stdout)["errors"])

        noncanonical_acceptance = [
            {
                "id": "A1",
                "requirement": "Tests pass",
                "observable_outcome": "test command exits 0",
                "evidence_required_json": [{"type": "command_result", "name": "test evidence", "exit_code": 0}],
                "verifier_json": [{"type": "command_result", "name": "test evidence"}],
                "review_focus_json": ["proof obligation is satisfied"],
                "required": True,
            }
        ]
        result = run(replace_arg(stage_contract_args(root, "s", slug), "--acceptance-json", compact_json(noncanonical_acceptance)), check=False)
        assert any("noncanonical keys are not allowed" in error for error in json.loads(result.stdout)["errors"])

        bad_gate = compact_json({"entry": ["contract locked"], "exit": ["tests pass"], "verifiers": ["command_result", "mystery_review"], "review_focus": ["proof obligations"]})
        result = run(replace_arg(stage_contract_args(root, "s", slug), "--gate-json", bad_gate), check=False)
        assert any("unsupported verifier type" in error and "mystery_review" in error for error in json.loads(result.stdout)["errors"])

        alias_recovery = replace_arg(
            stage_contract_args(root, "s", slug),
            "--recovery-json",
            compact_json({"rollback": "old", "restart": "old", "escalation": "old"}),
        )
        result = run(alias_recovery, check=False)
        assert any("recovery_json requires rollback_boundary" in error for error in json.loads(result.stdout)["errors"])

        mixed_alias_recovery = replace_arg(
            stage_contract_args(root, "s", slug),
            "--recovery-json",
            compact_json(
                {
                    "rollback_boundary": "revert",
                    "restart_rule": "restart",
                    "escalation_rule": "surface blocker",
                    "rollback": "old",
                }
            ),
        )
        result = run(mixed_alias_recovery, check=False)
        assert any("recovery_json contains noncanonical keys" in error for error in json.loads(result.stdout)["errors"])

        alias_budget = replace_arg(
            stage_contract_args(root, "s", slug),
            "--budget-json",
            compact_json({"retries": 2, "stop": "old"}),
        )
        result = run(alias_budget, check=False)
        assert any("budget_json requires retry_limit" in error for error in json.loads(result.stdout)["errors"])

        mixed_alias_budget = replace_arg(
            stage_contract_args(root, "s", slug),
            "--budget-json",
            compact_json({"retry_limit": 2, "max_stage_attempts": 3, "stop_condition": "recorded review blocks or passes", "retries": 2}),
        )
        result = run(mixed_alias_budget, check=False)
        assert any("budget_json contains noncanonical keys" in error for error in json.loads(result.stdout)["errors"])


def test_contract_add_stage_rejects_without_half_written_rows() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "stage-atomic", "--title", "atomic", "--user-goal", "atomic"])
        slug, goal = goal_dir(root, "s", "stage-atomic")
        before_plan = list(csv.DictReader((goal / "plan.csv").open(encoding="utf-8")))
        before_acceptance = list(csv.DictReader((goal / "acceptance.csv").open(encoding="utf-8")))
        bad = replace_arg(stage_contract_args(root, "s", slug), "--acceptance-json", "[]")
        result = run(bad, check=False)
        assert result.returncode == 2
        assert list(csv.DictReader((goal / "plan.csv").open(encoding="utf-8"))) == before_plan
        assert list(csv.DictReader((goal / "acceptance.csv").open(encoding="utf-8"))) == before_acceptance


def test_contract_lock_hash_covers_structural_fields() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "lock-hash")
        rows = list(csv.DictReader((goal / "plan.csv").open(encoding="utf-8")))
        rows[0]["scope_json"] = compact_json({"allowed_paths": ["other/**"], "forbidden_paths": [".mobius/**"]})
        mobius.write_csv_rows(goal / "plan.csv", mobius.PLAN_FIELDS, rows)
        result = json.loads(run(["--project-root", str(root), "contract-validate", "--session-id", "s", "--goal-slug", slug], check=False).stdout)
        assert any("locked structural fields changed after lock" in error for error in result["errors"])


def test_contract_supersede_stage_is_transactional_and_explicit() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "supersede", "--title", "supersede", "--user-goal", "supersede"])
        slug, goal = goal_dir(root, "s", "supersede")
        add_stage(root, "s", slug, "P1", "A1")
        run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])

        replacement_acceptance = [
            {
                "id": "A2",
                "supersedes_id": "A1",
                "requirement": "Replacement command proof is recorded",
                "observable_outcome": "evidence.csv contains replacement command proof row with exit code 0",
                "evidence_required": [{"type": "command_result", "name": "replacement evidence", "exit_code": 0}],
                "verifier": [{"type": "command_result", "name": "replacement evidence"}, {"type": "mobiuscv_delta"}],
                "review_focus": ["replacement proof obligation is satisfied"],
                "required": True,
            }
        ]
        command = stage_contract_args(root, "s", slug, plan_id="P2", acceptance_id="A2")
        command[command.index("contract-add-stage")] = "contract-supersede-stage"
        command = replace_arg(command, "--acceptance-json", compact_json(replacement_acceptance))
        command.extend(["--supersedes-id", "P1", "--change-reason", "narrow scope after review"])
        result = json.loads(run(command).stdout)
        assert result["ok"] is True

        plan_rows = {row["id"]: row for row in csv.DictReader((goal / "plan.csv").open(encoding="utf-8"))}
        acceptance_rows = {row["id"]: row for row in csv.DictReader((goal / "acceptance.csv").open(encoding="utf-8"))}
        assert plan_rows["P1"]["contract_status"] == "superseded"
        assert plan_rows["P1"]["change_reason"] == "narrow scope after review"
        assert plan_rows["P2"]["supersedes_id"] == "P1"
        assert acceptance_rows["A1"]["status"] == "superseded"
        assert acceptance_rows["A2"]["supersedes_id"] == "A1"
        assert acceptance_rows["A2"]["change_reason"] == "narrow scope after review"
        assert json.loads(run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug]).stdout)["ok"] is True
        assert json.loads(run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", slug]).stdout)["next_plan_item_id"] == "P2"


def test_contract_supersede_stage_blocks_active_dependents() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "supersede-deps", "--title", "supersede", "--user-goal", "supersede"])
        slug, goal = goal_dir(root, "s", "supersede-deps")
        add_stage(root, "s", slug, "P1", "A1")
        add_stage(root, "s", slug, "P2", "A2", ["P1"])
        run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])

        replacement_acceptance = [
            {
                "id": "A3",
                "supersedes_id": "A1",
                "requirement": "Replacement command proof is recorded",
                "observable_outcome": "evidence.csv contains replacement command proof row with exit code 0",
                "evidence_required": [{"type": "command_result", "name": "replacement evidence", "exit_code": 0}],
                "verifier": [{"type": "command_result", "name": "replacement evidence"}, {"type": "mobiuscv_delta"}],
                "review_focus": ["replacement proof obligation is satisfied"],
                "required": True,
            }
        ]
        command = stage_contract_args(root, "s", slug, plan_id="P3", acceptance_id="A3")
        command[command.index("contract-add-stage")] = "contract-supersede-stage"
        command = replace_arg(command, "--acceptance-json", compact_json(replacement_acceptance))
        command.extend(["--supersedes-id", "P1", "--change-reason", "narrow scope after review"])
        result = run(command, check=False)
        assert result.returncode == 2
        assert "cannot supersede plan item with active dependents: P2" in json.loads(result.stdout)["errors"]
        assert "P3" not in (goal / "plan.csv").read_text(encoding="utf-8")


def test_continue_respects_dependencies() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "deps", "--title", "deps", "--user-goal", "deps"])
        slug, goal = goal_dir(root, "s", "deps")
        add_stage(root, "s", slug, "P1", "A1")
        add_stage(root, "s", slug, "P2", "A2", ["P1"])
        run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])
        first = json.loads(run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", slug]).stdout)
        assert first["next_plan_item_id"] == "P1"
        assert first["loop"]["schema"] == "mobius.loop"
        assert first["loop"]["mode"] == "full_plan"
        assert first["loop"]["agent_must_continue"] is True
        assert first["loop"]["agent_must_stop"] is False
        assert first["loop"]["next_command"] == "loop-start-stage --plan-item-id P1"
        start_stage(root, "s", slug, "P1")
        run(
            [
                "--project-root",
                str(root),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "command_result",
                "--summary",
                "test evidence",
                "--supports",
                "A1",
                "--artifact-json",
                compact_json({"type": "command_result", "name": "test evidence", "command": "pytest", "exit_code": 0}),
            ]
        )
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        second = json.loads(run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", slug]).stdout)
        assert second["next_plan_item_id"] == "P2"
        assert second["loop"]["agent_must_continue"] is True
        assert second["loop"]["next_command"] == "loop-start-stage --plan-item-id P2"


def test_packet_read_recovers_existing_packet_without_csv_transport() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "packet-read")
        start_stage(root, "s", slug)
        delta_packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(delta_packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        exit_packet = create_packet(root, "s", slug, "exit_review")
        audit = json.loads(run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", slug]).stdout)
        assert audit["loop"]["next_required_action"] == "record_exit_review"
        assert audit["loop"]["next_command"] == f"packet-read --review-mode exit_review --packet-id {exit_packet['packet']}"

        read = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "packet-read",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--review-mode",
                    "exit_review",
                    "--packet-id",
                    str(exit_packet["packet"]),
                ]
            ).stdout
        )
        assert read["next_required_action"] == "record_exit_review"
        assert read["review_allowed"] is True
        assert read["packet"] == exit_packet
        assert read["packet_sha256"].startswith("sha256:")
        assert "packet_json" not in read
        assert "packet_id" not in read["packet"]

        mobius.record_cv_result(root, "s", slug, cv_envelope("cv_exit_001", read_goal_id(goal), packet_id=str(exit_packet["packet"])), "exit_review")
        reviewed = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "packet-read",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--review-mode",
                    "exit_review",
                    "--packet-id",
                    str(exit_packet["packet"]),
                ]
            ).stdout
        )
        assert reviewed["next_required_action"] == "completion_allowed"
        assert reviewed["loop"]["agent_must_stop"] is True
        assert reviewed["review_allowed"] is False


def test_loop_lifecycle_commands_return_mirrored_loop_actions() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "mirrored-loop", "--title", "mirrored-loop", "--user-goal", "mirrored-loop"])
        slug, _goal = goal_dir(root, "s", "mirrored-loop")
        add_stage(root, "s", slug)
        run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])

        started = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "loop-start-stage",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--plan-item-id",
                    "P1",
                ]
            ).stdout
        )
        loop = assert_required_loop_action_is_mirrored(started)
        assert loop["next_required_action"] == "run_missing_command_evidence"

        evidence = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "evidence-add",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--type",
                    "command_result",
                    "--summary",
                    "test evidence",
                    "--supports",
                    "A1",
                    "--artifact-json",
                    compact_json({"type": "command_result", "name": "test evidence", "command": "pytest", "exit_code": 0}),
                ]
            ).stdout
        )
        loop = assert_required_loop_action_is_mirrored(evidence)
        assert loop["next_required_action"] == "create_delta_packet"

        packet = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "packet-create",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--review-mode",
                    "delta_review",
                    "--acceptance-id",
                    "A1",
                ]
            ).stdout
        )
        loop = assert_required_loop_action_is_mirrored(packet)
        assert loop["next_required_action"] == "record_delta_review"
        assert loop["packet_id"] == packet["packet"]["packet"]
        assert loop["review_mode"] == "delta_review"

        read = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "packet-read",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--review-mode",
                    "delta_review",
                    "--packet-id",
                    str(packet["packet"]["packet"]),
                ]
            ).stdout
        )
        loop = assert_required_loop_action_is_mirrored(read)
        assert loop["next_required_action"] == "record_delta_review"
        assert loop["packet_id"] == packet["packet"]["packet"]
        assert read["review_allowed"] is True


def test_record_missing_evidence_loop_action_for_non_command_proof() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "missing-human", "--title", "missing-human", "--user-goal", "missing-human"])
        slug, _goal = goal_dir(root, "s", "missing-human")
        run(human_assertion_stage_args(root, "s", slug))
        run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])
        started = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "loop-start-stage",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--plan-item-id",
                    "P1",
                ]
            ).stdout
        )
        loop = assert_required_loop_action_is_mirrored(started)
        assert loop["next_required_action"] == "record_missing_evidence"
        assert loop["next_command"] == "evidence-add"


def test_unlocked_contract_blocks_loop_packet_and_recorded_review() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal = prepare_unlocked_goal(root, "s", "unlocked")
        loop = run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", slug], check=False)
        assert loop.returncode == 2
        assert json.loads(loop.stdout)["next_required_action"] == "needs_contract_change"
        packet_create = run(
            ["--project-root", str(root), "packet-create", "--session-id", "s", "--goal-slug", slug, "--review-mode", "delta_review", "--acceptance-id", "A1"],
            check=False,
        )
        assert packet_create.returncode == 2
        packet = write_delta_packet_row(root, goal, slug)
        try:
            mobius.record_cv_result(
                root,
                "s",
                slug,
                delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(packet["packet"])),
                "delta_review",
                target_plan_item_id="P1",
                target_acceptance_ids=["A1"],
            )
        except mobius.MobiusError as exc:
            assert "unlocked contract rows: P1,A1" in str(exc)
        else:
            raise AssertionError("record_cv_result accepted unlocked contract")


def test_evidence_requires_known_acceptance_and_structured_required_proof() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, _goal = prepare_unlocked_goal(root, "s", "evidence-contract")
        run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])
        unknown = run(["--project-root", str(root), "evidence-add", "--session-id", "s", "--goal-slug", slug, "--type", "command_result", "--summary", "x", "--supports", "A404"], check=False)
        assert unknown.returncode == 2
        unstructured = run(["--project-root", str(root), "evidence-add", "--session-id", "s", "--goal-slug", slug, "--type", "command_result", "--summary", "test evidence", "--supports", "A1"], check=False)
        assert unstructured.returncode == 2
        structured = json.loads(
            run(
                [
                    "--project-root",
                    str(root),
                    "evidence-add",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--type",
                    "command_result",
                    "--summary",
                    "test evidence",
                    "--supports",
                    "A1",
                    "--artifact-json",
                    compact_json({"type": "command_result", "name": "test evidence", "command": "pytest", "exit_code": 0}),
                ]
            ).stdout
        )
        assert structured["ok"] is True


def test_packet_is_lightweight_index_and_hash_checked() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "packet")
        packet = create_packet(root, "s", slug, "exit_review")
        row = list(csv.DictReader((goal / "packets.csv").open(encoding="utf-8")))[0]
        assert packet["schema"] == "mobius.packet"
        assert packet["ledger"]["root"].startswith(".mobius/runs/")
        assert len(packet["ledger"]["hash"]) == 7
        assert packet["coverage"] == {"A1": ["E1"]}
        assert packet["refs"]["E1"][2].startswith("h:")
        assert list(row) == mobius.PACKET_FIELDS
        assert "inputs" not in packet
        assert "content" not in json.dumps(packet)
        assert row["packet_sha256"].startswith("sha256:")

        run(
            [
                "--project-root",
                str(root),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "command_result",
                "--summary",
                "new evidence changes the hash",
                "--supports",
                "A1",
                "--artifact-json",
                compact_json({"type": "command_result", "name": "test evidence", "command": "pytest", "exit_code": 0}),
            ]
        )
        _normalized, errors = mobius.validate_packet_for_goal(goal, packet, "exit_review")
        assert any("coverage.A1 mismatch" in error or "refs mismatch" in error for error in errors)


def test_file_ref_and_change_set_scope_evidence_are_compact_refs() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        artifact = root / "result.txt"
        artifact.write_text("proof\n", encoding="utf-8")
        slug, goal, _evidence = prepare_goal(root, "s", "evidence-index")
        run(
            [
                "--project-root",
                str(root),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "file_ref",
                "--summary",
                "file proves A1",
                "--supports",
                "A1",
                "--artifact",
                str(artifact),
            ]
        )
        run(
            [
                "--project-root",
                str(root),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "change_set_scope",
                "--summary",
                "change scope ref",
                "--supports",
                "A1",
                "--artifact-json",
                compact_json(
                    {
                        "type": "change_set_scope",
                        "name": "source scope",
                        "paths": ["scripts/mobius.py"],
                        "allowed_change_classes": ["source"],
                        "forbidden_paths": [".mobius/**"],
                        "coverage": {"tracked": True, "staged": True, "untracked": True, "intent_to_add": True},
                    }
                ),
            ]
        )
        packet = create_packet(root, "s", slug, "exit_review")
        refs = packet["refs"]
        assert refs["E2"][0] == "file_ref"
        assert refs["E2"][1] == "result.txt"
        assert refs["E2"][2].startswith("h:")
        assert refs["E3"][0] == "change_set_scope"
        assert refs["E3"][1] == "scripts/mobius.py"
        assert refs["E3"][2].startswith("h:")
        assert "sha256:" not in json.dumps(packet)
        assert mobius.validate_packet_for_goal(goal, packet, "exit_review")[1] == []


def test_evidence_path_boundaries_are_enforced() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        project = Path(tmp) / "project"
        project.mkdir()
        outside = Path(tmp) / "outside.txt"
        inside = project / "inside.txt"
        outside.write_text("outside\n", encoding="utf-8")
        inside.write_text("inside\n", encoding="utf-8")
        slug, _goal, _evidence = prepare_goal(project, "s", "path-boundary")

        outside_result = run(
            [
                "--project-root",
                str(project),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "file_ref",
                "--summary",
                "outside",
                "--supports",
                "A1",
                "--artifact",
                str(outside),
            ],
            check=False,
        )
        assert outside_result.returncode == 2
        assert "inside project root" in json.loads(outside_result.stdout)["errors"][0]

        wrong_type = run(
            [
                "--project-root",
                str(project),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "command_result",
                "--summary",
                "command path ref",
                "--supports",
                "A1",
                "--artifact",
                str(inside),
            ],
            check=False,
        )
        assert wrong_type.returncode == 2
        assert "--artifact path refs are only allowed" in json.loads(wrong_type.stdout)["errors"][0]

        change_set_path = run(
            [
                "--project-root",
                str(project),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "change_set_scope",
                "--summary",
                "change scope path ref",
                "--supports",
                "A1",
                "--artifact-json",
                compact_json({"type": "change_set_scope", "name": "source scope", "path": str(inside)}),
            ],
            check=False,
        )
        assert change_set_path.returncode == 2
        assert "path refs are only allowed" in json.loads(change_set_path.stdout)["errors"][0]

        absolute_scope_path = run(
            [
                "--project-root",
                str(project),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "change_set_scope",
                "--summary",
                "absolute scope path",
                "--supports",
                "A1",
                "--artifact-json",
                compact_json(
                    {
                        "type": "change_set_scope",
                        "paths": ["src", str(outside)],
                        "allowed_change_classes": ["source"],
                        "forbidden_paths": [".mobius/**"],
                        "coverage": {"tracked": True, "staged": True, "untracked": True, "intent_to_add": True},
                    }
                ),
            ],
            check=False,
        )
        assert absolute_scope_path.returncode == 2
        assert "change_set_scope.paths: path must be root-relative" in json.loads(absolute_scope_path.stdout)["errors"][0]

        malformed_scope_cases = [
            (
                {
                    "type": "change_set_scope",
                    "paths": ["src"],
                    "forbidden_paths": [".mobius/**"],
                    "coverage": {"tracked": True, "staged": True, "untracked": True, "intent_to_add": True},
                },
                "allowed_change_classes",
            ),
            (
                {
                    "type": "change_set_scope",
                    "paths": ["src"],
                    "allowed_change_classes": ["source"],
                    "coverage": {"tracked": True, "staged": True, "untracked": True, "intent_to_add": True},
                },
                "forbidden_paths",
            ),
            (
                {
                    "type": "change_set_scope",
                    "paths": ["src"],
                    "allowed_change_classes": ["source"],
                    "forbidden_paths": [".mobius/**"],
                    "coverage": {"tracked": False, "staged": True, "untracked": True, "intent_to_add": True},
                },
                "coverage flags must be true booleans",
            ),
            (
                {
                    "type": "change_set_scope",
                    "paths": ["src"],
                    "allowed_change_classes": ["source"],
                    "forbidden_paths": [".mobius/**"],
                    "coverage": {"tracked": "true", "staged": True, "untracked": True, "intent_to_add": True},
                },
                "coverage flags must be true booleans",
            ),
            (
                {
                    "type": "change_set_scope",
                    "paths": ["../src"],
                    "allowed_change_classes": ["source"],
                    "forbidden_paths": [".mobius/**"],
                    "coverage": {"tracked": True, "staged": True, "untracked": True, "intent_to_add": True},
                },
                "path must not contain '..'",
            ),
        ]
        for payload, expected in malformed_scope_cases:
            result = run(
                [
                    "--project-root",
                    str(project),
                    "evidence-add",
                    "--session-id",
                    "s",
                    "--goal-slug",
                    slug,
                    "--type",
                    "change_set_scope",
                    "--summary",
                    "malformed scope",
                    "--supports",
                    "A1",
                    "--artifact-json",
                    compact_json(payload),
                ],
                check=False,
            )
            assert result.returncode == 2
            assert expected in json.loads(result.stdout)["errors"][0]


def test_csv_row_shaped_packet_is_not_transport() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "packet-csv-row")
        create_packet(root, "s", slug, "exit_review")
        row = list(csv.DictReader((goal / "packets.csv").open(encoding="utf-8")))[0]
        _normalized, errors = mobius.validate_packet_for_goal(goal, row, "exit_review")
        assert any("packet is required" in error for error in errors)


def test_recorded_delta_pass_updates_loop_and_records_policy() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "delta-pass")
        start_stage(root, "s", slug)
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        recorded = mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "awaiting_exit_review"
        assert recorded["next_required_action"] == "create_exit_packet"
        assert recorded["loop"]["next_required_action"] == "create_exit_packet"
        loop_row = list(csv.DictReader((goal / "loop.csv").open(encoding="utf-8")))[0]
        assert loop_row["status"] == "passed"
        assert list(csv.DictReader((goal / "loop.csv").open(encoding="utf-8")))[0]["last_cv_id"] == "cv_delta_001"
        input_refs = json.loads(list(csv.DictReader((goal / "cv.csv").open(encoding="utf-8")))[0]["input_refs_json"])
        assert input_refs["review_policy"]["name"] == "delta_kimi"


def test_ledger_audit_reports_missing_exit_cv() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "audit-missing-exit")
        start_stage(root, "s", slug)
        delta_packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(delta_packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        exit_packet = create_packet(root, "s", slug, "exit_review")
        audit = json.loads(run(["--project-root", str(root), "ledger-audit", "--session-id", "s", "--goal-slug", slug]).stdout)["audit"]
        assert audit["loop_gate"] == "awaiting_exit_review"
        assert audit["packet_id"] == exit_packet["packet"]
        assert audit["review_mode"] == "exit_review"
        assert audit["loop"]["packet_id"] == exit_packet["packet"]
        assert audit["loop"]["review_mode"] == "exit_review"
        assert audit["exit_cv_id"] == ""
        assert audit["unverified_acceptance_ids"] == ["A1"]
        assert audit["next_required_action"] == "record_exit_review"


def test_exit_review_interruption_is_visible() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "interrupted-exit")
        start_stage(root, "s", slug)
        delta_packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(delta_packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        exit_packet = create_packet(root, "s", slug, "exit_review")
        mobius.review_attempt_started(goal, str(exit_packet["packet"]), "exit_review")
        audit = json.loads(run(["--project-root", str(root), "ledger-audit", "--session-id", "s", "--goal-slug", slug]).stdout)["audit"]
        assert audit["packet_id"] == exit_packet["packet"]
        assert audit["review_mode"] == "exit_review"
        assert audit["exit_cv_id"] == ""
        assert audit["open_review_attempts"][0]["status"] == "started"
        assert audit["interrupted_review_attempts"][0]["status"] == "interrupted"
        assert audit["next_required_action"] == "retry_review"
        assert audit["loop"]["next_command"] == f"packet-read --review-mode exit_review --packet-id {exit_packet['packet']}"


def test_status_lists_active_goals_and_loop_next_is_not_public() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, _goal, _evidence = prepare_goal(root, "s", "status-active")
        status = json.loads(run(["--project-root", str(root), "status"]).stdout)
        assert status["next_required_action"] == "continue_active_goal"
        assert status["active_goals"][0]["goal_slug"] == slug

        removed = run(["--project-root", str(root), "loop-next", "--session-id", "s", "--goal-slug", slug], check=False)
        assert removed.returncode == 2
        assert "invalid choice" in removed.stderr


def test_delta_light_policy_and_mcp_default_skip_kimi() -> None:
    module = load_mobius_cv_mcp_module()
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "delta-light")
        start_stage(root, "s", slug)
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        reviewer_called = {"value": False}

        def fail_if_called(*args, **kwargs):
            reviewer_called["value"] = True
            raise AssertionError("delta-light should not run Kimi")

        module.run_kimi_review = fail_if_called
        recorded = module.mobius_cv_record_delta_review(
            project_root=str(root),
            session_id="s",
            goal_slug=slug,
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
            packet=packet,
            codex_subagent_result=reviewer_result("codex-subagent", "delta_review", ["A1"]),
            cv_id="cv_delta_001",
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "awaiting_exit_review"
        assert recorded["next_required_action"] == "create_exit_packet"
        assert reviewer_called["value"] is False
        input_refs = json.loads(list(csv.DictReader((goal / "cv.csv").open(encoding="utf-8")))[0]["input_refs_json"])
        assert input_refs["review_policy"]["name"] == "delta_light"
        attempts = list(csv.DictReader((goal / "review_attempts.csv").open(encoding="utf-8")))
        assert attempts[0]["packet_id"] == packet["packet"]
        assert attempts[0]["status"] == "recorded"


def test_raw_reviewer_result_is_ref_not_inline_blob() -> None:
    module = load_mobius_cv_mcp_module()
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "raw-ref")
        start_stage(root, "s", slug)
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        recorded = module.mobius_cv_record_delta_review(
            project_root=str(root),
            session_id="s",
            goal_slug=slug,
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
            packet=packet,
            codex_subagent_result=valid_reviewer_block(reviewer="codex-subagent", mode="delta_review"),
            cv_id="cv_delta_raw",
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "awaiting_exit_review"
        assert recorded["next_required_action"] == "create_exit_packet"
        cv_row = list(csv.DictReader((goal / "cv.csv").open(encoding="utf-8")))[0]
        assert cv_row["raw_ref"].endswith("raw_reviews/cv_delta_raw.json")
        assert len(cv_row["raw_hash_tail"]) == 7
        assert "MOBIUS_CV_REVIEWER_RESULT" not in json.dumps(cv_row)
        assert "_raw_text" not in cv_row["reviewers_json"]
        raw_path = root / cv_row["raw_ref"]
        assert raw_path.exists()
        assert "MOBIUS_CV_REVIEWER_RESULT" in raw_path.read_text(encoding="utf-8")


def test_pass_cv_requires_recorded_review_policy() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "missing-policy")
        start_stage(root, "s", slug)
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        cv = delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(packet["packet"]))
        cv["input_refs"] = {}
        try:
            mobius.record_cv_result(root, "s", slug, cv, "delta_review", target_plan_item_id="P1", target_acceptance_ids=["A1"])
        except mobius.MobiusError as exc:
            assert "pass result requires input_refs.review_policy" in str(exc)
        else:
            raise AssertionError("record_cv_result accepted a pass without recorded review policy")


def test_recorded_review_rejects_aggregate_mismatch_and_degraded_pass() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "review-invalid")
        start_stage(root, "s", slug)
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        cv = delta_cv_envelope("cv_delta_001", read_goal_id(goal), overall="pass", packet_id=str(packet["packet"]), reviewer_verdict="fail")
        cv["result"]["overall"] = "pass"
        try:
            mobius.record_cv_result(root, "s", slug, cv, "delta_review", target_plan_item_id="P1", target_acceptance_ids=["A1"])
        except mobius.MobiusError as exc:
            assert "result.overall does not match reviewer rows" in str(exc)
        else:
            raise AssertionError("record_cv_result accepted top-level pass over reviewer fail")

        exit_packet = create_packet(root, "s", slug, "exit_review")
        cv = cv_envelope("cv_exit_001", read_goal_id(goal), overall="pass", degraded=["kimi-code"], packet_id=str(exit_packet["packet"]))
        cv["result"]["overall"] = "pass"
        try:
            mobius.record_cv_result(root, "s", slug, cv, "exit_review")
        except mobius.MobiusError as exc:
            assert "degraded_reviewers" in str(exc)
        else:
            raise AssertionError("record_cv_result accepted degraded pass")


def test_delta_pass_requires_satisfied_proof_obligations() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal = prepare_unlocked_goal(root, "s", "unsatisfied-proof")
        run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])
        start_stage(root, "s", slug)
        mobius.write_csv_rows(
            goal / "evidence.csv",
            mobius.EVIDENCE_FIELDS,
            [
                {
                    "schema": "mobius.evidence",
                    "id": "E1",
                    "goal_id": read_goal_id(goal),
                    "type": "human",
                    "summary": "does not satisfy command proof",
                    "supports_json": mobius.as_json_cell(["A1"]),
                    "artifact_json": mobius.as_json_cell({}),
                    "created_by": "test",
                    "created_at": "2026-07-01T00:00:00+00:00",
                }
            ],
        )
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        recorded = mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "running"
        assert recorded["next_required_action"] == "run_missing_command_evidence"
        assert recorded["loop"]["agent_must_continue"] is True
        assert recorded["loop"]["next_required_action"] == "run_missing_command_evidence"
        assert any("unsatisfied evidence_required_json" in finding for finding in recorded["blocking_findings"])


def test_repairable_delta_fail_returns_repair_stage() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "delta-repair")
        start_stage(root, "s", slug)
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        recorded = mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_with_revisions("cv_delta_001", read_goal_id(goal), ["tighten architecture boundary"], packet_id=str(packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "running"
        assert recorded["next_required_action"] == "repair_stage"
        assert recorded["loop"]["agent_must_continue"] is True
        assert recorded["loop"]["next_command"] == "loop-start-stage --plan-item-id P1"

        start_stage(root, "s", slug)
        row = list(csv.DictReader((goal / "loop.csv").open(encoding="utf-8")))[0]
        assert row["status"] == "running"
        assert row["attempt"] == "2"
        assert row["last_packet_id"] == ""
        assert row["last_cv_id"] == ""
        new_packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        assert new_packet["packet"] == "packet_delta_002"


def test_repair_budget_exhaustion_stops_loop() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "budget", "--title", "budget", "--user-goal", "budget"])
        slug, goal = goal_dir(root, "s", "budget")
        command = replace_arg(
            stage_contract_args(root, "s", slug),
            "--budget-json",
            compact_json({"retry_limit": 0, "max_stage_attempts": 1, "stop_condition": "recorded review blocks or passes"}),
        )
        run(command)
        run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])
        start_stage(root, "s", slug)
        run(
            [
                "--project-root",
                str(root),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "command_result",
                "--summary",
                "test evidence",
                "--supports",
                "A1",
                "--artifact-json",
                compact_json({"type": "command_result", "name": "test evidence", "command": "pytest", "exit_code": 0}),
            ]
        )
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        recorded = mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_with_revisions("cv_delta_001", read_goal_id(goal), ["repair needed"], packet_id=str(packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "blocked"
        assert recorded["next_required_action"] == "repair_budget_exhausted"
        audit = json.loads(run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", slug]).stdout)
        assert audit["loop"]["agent_must_stop"] is True
        assert audit["loop"]["stop_reason"] == "repair_budget_exhausted"


def test_delta_unknown_review_quality_routes_to_new_packet_before_budget_or_evidence() -> None:
    scenarios = {
        "delta-unknown-budget": {"budget": {"retry_limit": 0, "max_stage_attempts": 1, "stop_condition": "recorded review blocks or passes"}, "evidence": True},
        "delta-unknown-evidence": {"budget": {"retry_limit": 1, "max_stage_attempts": 2, "stop_condition": "recorded review blocks or passes"}, "evidence": False},
    }
    for slug_suffix, config in scenarios.items():
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", slug_suffix, "--title", slug_suffix, "--user-goal", slug_suffix])
            slug, goal = goal_dir(root, "s", slug_suffix)
            command = replace_arg(stage_contract_args(root, "s", slug), "--budget-json", compact_json(config["budget"]))
            run(command)
            run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])
            start_stage(root, "s", slug)
            if config["evidence"]:
                run(
                    [
                        "--project-root",
                        str(root),
                        "evidence-add",
                        "--session-id",
                        "s",
                        "--goal-slug",
                        slug,
                        "--type",
                        "command_result",
                        "--summary",
                        "test evidence",
                        "--supports",
                        "A1",
                        "--artifact-json",
                        compact_json({"type": "command_result", "name": "test evidence", "command": "pytest", "exit_code": 0}),
                    ]
                )
            packet = create_packet(root, "s", slug, "delta_review", ["A1"])
            recorded = mobius.record_cv_result(
                root,
                "s",
                slug,
                delta_cv_envelope(
                    "cv_delta_unknown",
                    read_goal_id(goal),
                    overall="unknown",
                    packet_id=str(packet["packet"]),
                    reviewer_verdict="unknown",
                ),
                "delta_review",
                target_plan_item_id="P1",
                target_acceptance_ids=["A1"],
            )
            assert_loop_action_is_mirrored(recorded)
            assert recorded["gate"] == "running"
            assert recorded["next_required_action"] == "create_new_packet"
            assert recorded["loop"]["next_command"] == "packet-create --review-mode delta_review --acceptance-id A1"
            row = list(csv.DictReader((goal / "loop.csv").open(encoding="utf-8")))[0]
            assert row["status"] == "running"


def test_blocked_delta_returns_blocked_loop() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "delta-blocked")
        start_stage(root, "s", slug)
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        recorded = mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), overall="blocked", packet_id=str(packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "blocked"
        assert recorded["next_required_action"] == "goal_blocked"
        audit = json.loads(run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", slug]).stdout)
        assert audit["loop"]["agent_must_stop"] is True
        assert audit["loop"]["stop_reason"] == "review_blocked"


def test_public_loop_stop_reasons_are_closed_set() -> None:
    allowed = {"review_blocked", "repair_budget_exhausted", "contract_change_required", "no_runnable_action"}
    assert mobius.LOOP_STOP_REASONS == allowed

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        accepted_slug, accepted_goal, packet = terminal_goal(root, "s", "accepted-stop")
        accepted = json.loads(run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", accepted_slug]).stdout)
        assert accepted["loop"]["agent_must_stop"] is True
        assert accepted["loop"]["stop_reason"] in allowed
        assert accepted["loop"]["stop_reason"] == "no_runnable_action"

        blocked_slug, _blocked_goal, _packet = terminal_goal(root, "s", "blocked-stop", "blocked")
        blocked = json.loads(run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", blocked_slug]).stdout)
        assert blocked["loop"]["agent_must_stop"] is True
        assert blocked["loop"]["stop_reason"] in allowed
        assert blocked["loop"]["stop_reason"] == "review_blocked"

        mutate = run(
            [
                "--project-root",
                str(root),
                "packet-read",
                "--session-id",
                "s",
                "--goal-slug",
                accepted_slug,
                "--review-mode",
                "exit_review",
                "--packet-id",
                str(packet["packet"]),
            ]
        )
        read = json.loads(mutate.stdout)
        assert read["loop"]["stop_reason"] in allowed


def test_recorded_exit_pass_updates_acceptance_verdict_and_run() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "exit-pass")
        packet = create_packet(root, "s", slug, "exit_review")
        recorded = mobius.record_cv_result(root, "s", slug, cv_envelope("cv_exit_001", read_goal_id(goal), packet_id=str(packet["packet"])), "exit_review")
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "accepted"
        assert list(csv.DictReader((goal / "acceptance.csv").open(encoding="utf-8")))[0]["status"] == "pass"
        assert list(csv.DictReader((goal / "verdict.csv").open(encoding="utf-8")))[0]["overall"] == "accepted"
        run_row = list(csv.DictReader((root / ".mobius" / "runs" / "codex-session-s" / "run.csv").open(encoding="utf-8")))[0]
        assert json.loads(run_row["goals_json"])[0]["status"] == "accepted"


def test_recorded_exit_failure_does_not_half_write_state() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "exit-atomic")
        packet = create_packet(root, "s", slug, "exit_review")
        before_cv = list(csv.DictReader((goal / "cv.csv").open(encoding="utf-8")))
        before_acceptance = list(csv.DictReader((goal / "acceptance.csv").open(encoding="utf-8")))
        before_verdict = list(csv.DictReader((goal / "verdict.csv").open(encoding="utf-8")))
        previous = os.environ.get("MOBIUS_TEST_FAIL_BEFORE_CSV_COMMIT")
        os.environ["MOBIUS_TEST_FAIL_BEFORE_CSV_COMMIT"] = "1"
        try:
            try:
                mobius.record_cv_result(root, "s", slug, cv_envelope("cv_exit_001", read_goal_id(goal), packet_id=str(packet["packet"])), "exit_review")
            except mobius.MobiusError as exc:
                assert "injected failure before CSV commit" in str(exc)
            else:
                raise AssertionError("record_cv_result did not surface injected storage failure")
        finally:
            if previous is None:
                os.environ.pop("MOBIUS_TEST_FAIL_BEFORE_CSV_COMMIT", None)
            else:
                os.environ["MOBIUS_TEST_FAIL_BEFORE_CSV_COMMIT"] = previous
        assert list(csv.DictReader((goal / "cv.csv").open(encoding="utf-8"))) == before_cv
        assert list(csv.DictReader((goal / "acceptance.csv").open(encoding="utf-8"))) == before_acceptance
        assert list(csv.DictReader((goal / "verdict.csv").open(encoding="utf-8"))) == before_verdict


def test_recorded_exit_failure_repairs_earliest_stage_and_blocked_is_terminal() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "exit-repair")
        start_stage(root, "s", slug)
        delta_packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(delta_packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        exit_packet = create_packet(root, "s", slug, "exit_review")
        recorded = mobius.record_cv_result(
            root,
            "s",
            slug,
            cv_envelope("cv_exit_fail", read_goal_id(goal), overall="fail", packet_id=str(exit_packet["packet"])),
            "exit_review",
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "running"
        assert recorded["next_required_action"] == "repair_stage"
        assert recorded["loop"]["agent_must_continue"] is True
        assert recorded["loop"]["next_command"] == "loop-start-stage --plan-item-id P1"
        assert list(csv.DictReader((goal / "goal.csv").open(encoding="utf-8")))[0]["status"] == "active"
        loop_row = list(csv.DictReader((goal / "loop.csv").open(encoding="utf-8")))[0]
        assert loop_row["status"] == "running"
        assert loop_row["last_cv_id"] == "cv_exit_fail"
        start_stage(root, "s", slug)
        repair_delta_packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_repair", read_goal_id(goal), packet_id=str(repair_delta_packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        after_repair = json.loads(run(["--project-root", str(root), "continue", "--session-id", "s", "--goal-slug", slug]).stdout)
        assert after_repair["loop"]["next_required_action"] == "create_new_packet"
        assert after_repair["loop"]["agent_must_continue"] is True
        assert after_repair["loop"]["next_command"] == "packet-create --review-mode exit_review"

        slug, goal, _evidence = prepare_goal(root, "s", "exit-blocked")
        packet = create_packet(root, "s", slug, "exit_review")
        recorded = mobius.record_cv_result(
            root,
            "s",
            slug,
            cv_envelope("cv_exit_blocked", read_goal_id(goal), overall="blocked", packet_id=str(packet["packet"])),
            "exit_review",
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "blocked"
        assert list(csv.DictReader((goal / "goal.csv").open(encoding="utf-8")))[0]["status"] == "blocked"


def test_recorded_exit_unknown_creates_new_exit_packet_not_evidence_action() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "exit-unknown")
        start_stage(root, "s", slug)
        delta_packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(delta_packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        exit_packet = create_packet(root, "s", slug, "exit_review")
        recorded = mobius.record_cv_result(
            root,
            "s",
            slug,
            cv_envelope(
                "cv_exit_unknown",
                read_goal_id(goal),
                overall="unknown",
                reviewer_verdict="unknown",
                packet_id=str(exit_packet["packet"]),
            ),
            "exit_review",
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "awaiting_exit_review"
        assert recorded["next_required_action"] == "create_new_packet"
        assert recorded["loop"]["review_mode"] == "exit_review"
        assert recorded["loop"]["packet_id"] == ""
        assert recorded["loop"]["next_command"] == "packet-create --review-mode exit_review"


def test_recorded_exit_fail_with_degraded_or_unchecked_retries_exit_review() -> None:
    scenarios = {
        "exit-degraded-fail": [
            reviewer_result("codex-subagent", "exit_review", ["A1"], "fail"),
            {**reviewer_result("kimi-code", "exit_review", ["A1"], "fail"), "status": "timeout"},
        ],
        "exit-unchecked-fail": [
            reviewer_result("codex-subagent", "exit_review", [], "fail"),
            reviewer_result("kimi-code", "exit_review", [], "fail"),
        ],
    }
    for slug_suffix, reviewers in scenarios.items():
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            slug, goal, _evidence = prepare_goal(root, "s", slug_suffix)
            start_stage(root, "s", slug)
            delta_packet = create_packet(root, "s", slug, "delta_review", ["A1"])
            mobius.record_cv_result(
                root,
                "s",
                slug,
                delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(delta_packet["packet"])),
                "delta_review",
                target_plan_item_id="P1",
                target_acceptance_ids=["A1"],
            )
            exit_packet = create_packet(root, "s", slug, "exit_review")
            recorded = mobius.record_cv_result(
                root,
                "s",
                slug,
                exit_cv_from_reviewers("cv_exit_retry", read_goal_id(goal), reviewers, str(exit_packet["packet"])),
                "exit_review",
            )
            assert_loop_action_is_mirrored(recorded)
            assert recorded["gate"] == "awaiting_exit_review"
            assert recorded["next_required_action"] == "create_new_packet"
            assert recorded["loop"]["review_mode"] == "exit_review"
            assert recorded["loop"]["next_command"] == "packet-create --review-mode exit_review"
            loop_row = list(csv.DictReader((goal / "loop.csv").open(encoding="utf-8")))[0]
            assert loop_row["status"] == "passed"
            assert loop_row["last_cv_id"] == "cv_delta_001"


def test_recorded_exit_fail_budget_exhaustion_persists_blocked_loop() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        run(["--project-root", str(root), "goal-start", "--session-id", "s", "--slug", "exit-budget", "--title", "exit-budget", "--user-goal", "exit-budget"])
        slug, goal = goal_dir(root, "s", "exit-budget")
        command = replace_arg(
            stage_contract_args(root, "s", slug),
            "--budget-json",
            compact_json({"retry_limit": 0, "max_stage_attempts": 1, "stop_condition": "recorded review blocks or passes"}),
        )
        run(command)
        run(["--project-root", str(root), "contract-lock", "--session-id", "s", "--goal-slug", slug])
        run(
            [
                "--project-root",
                str(root),
                "evidence-add",
                "--session-id",
                "s",
                "--goal-slug",
                slug,
                "--type",
                "command_result",
                "--summary",
                "test evidence",
                "--supports",
                "A1",
                "--artifact-json",
                compact_json({"type": "command_result", "name": "test evidence", "command": "pytest", "exit_code": 0}),
            ]
        )
        start_stage(root, "s", slug)
        delta_packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        mobius.record_cv_result(
            root,
            "s",
            slug,
            delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(delta_packet["packet"])),
            "delta_review",
            target_plan_item_id="P1",
            target_acceptance_ids=["A1"],
        )
        exit_packet = create_packet(root, "s", slug, "exit_review")
        recorded = mobius.record_cv_result(
            root,
            "s",
            slug,
            cv_envelope("cv_exit_budget", read_goal_id(goal), overall="fail", packet_id=str(exit_packet["packet"])),
            "exit_review",
        )
        assert_loop_action_is_mirrored(recorded)
        assert recorded["gate"] == "blocked"
        assert recorded["next_required_action"] == "repair_budget_exhausted"
        assert recorded["loop"]["agent_must_stop"] is True
        assert recorded["loop"]["stop_reason"] == "repair_budget_exhausted"
        loop_row = list(csv.DictReader((goal / "loop.csv").open(encoding="utf-8")))[0]
        assert loop_row["status"] == "blocked"
        assert loop_row["last_cv_id"] == "cv_exit_budget"
        assert any(str(item).startswith("repair_budget_exhausted:") for item in json.loads(loop_row["blocking_findings_json"]))


def test_packet_id_is_one_shot_for_delta_review() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "one-shot")
        start_stage(root, "s", slug)
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        mobius.record_cv_result(root, "s", slug, delta_cv_envelope("cv_delta_001", read_goal_id(goal), packet_id=str(packet["packet"])), "delta_review", target_plan_item_id="P1", target_acceptance_ids=["A1"])
        try:
            mobius.record_cv_result(root, "s", slug, delta_cv_envelope("cv_delta_002", read_goal_id(goal), packet_id=str(packet["packet"])), "delta_review", target_plan_item_id="P1", target_acceptance_ids=["A1"])
        except mobius.MobiusError as exc:
            assert f"packet_id already has a recorded review: {packet['packet']}" in str(exc)
        else:
            raise AssertionError("record_cv_result reused a packet_id")


def test_terminal_goal_blocks_mutations_and_recorded_reviews() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, packet = terminal_goal(root, "s", "terminal")
        for command, expected in (
            (
                ["--project-root", str(root), "packet-create", "--session-id", "s", "--goal-slug", slug, "--review-mode", "exit_review"],
                "packet-create is not allowed for terminal goal: accepted",
            ),
            (
                ["--project-root", str(root), "evidence-add", "--session-id", "s", "--goal-slug", slug, "--type", "command_result", "--summary", "late", "--supports", "A1"],
                "evidence-add is not allowed for terminal goal: accepted",
            ),
        ):
            result = run(command, check=False)
            assert result.returncode == 2
            assert expected in json.loads(result.stdout)["errors"]
        try:
            mobius.record_cv_result(root, "s", slug, cv_envelope("cv_exit_002", read_goal_id(goal), packet_id=str(packet["packet"])), "exit_review")
        except mobius.MobiusError as exc:
            assert "record_cv_result is not allowed for terminal goal: accepted" in str(exc)
        else:
            raise AssertionError("record_cv_result accepted terminal goal")


def test_mobius_cv_prompt_allows_autonomous_review_tools() -> None:
    module = load_mobius_cv_mcp_module()
    prompt = module.review_prompt('{"schema":"mobius.packet","coverage":{"A1":["E1"]},"refs":{"E1":["command_result","pytest","h:123abcd"]}}', "exit_review", ["A1"], "kimi-code")
    assert "frozen local index" in prompt
    assert "packet refs as starting points" in prompt
    assert "not exclusive evidence" in prompt
    assert "packet.refs" in prompt
    assert "MOBIUS_CV_REVIEWER_RESULT" in prompt
    assert prompt.rstrip().endswith("END_MOBIUS_CV_REVIEWER_RESULT")
    context_prompt = module.review_prompt(
        '{"schema":"mobius.packet","coverage":{"A1":["E1"]},"refs":{"E1":["file_ref","README.md","h:123abcd"]}}',
        "exit_review",
        ["A1"],
        "kimi-code",
        {"project_root": "/tmp/mobius-project", "ledger_abs_root": "/tmp/mobius-project/.mobius/run"},
    )
    assert "Reviewer local path context" in context_prompt
    assert '"/tmp/mobius-project"' in context_prompt
    assert "The frozen Mobius packet JSON above remains the" in context_prompt


def valid_reviewer_block(reviewer: str = "kimi-code", mode: str = "exit_review") -> str:
    return f"""MOBIUS_CV_REVIEWER_RESULT
REVIEWER: {reviewer}
REVIEW_MODE: {mode}
VERDICT: pass
CHECKED_ACCEPTANCE_IDS: ["A1"]
UNCHECKED_ACCEPTANCE_IDS: []
BLOCKING_FINDINGS: []
REQUIRED_REVISIONS: []
EVIDENCE_CHECKED: ["plan.csv","acceptance.csv","evidence.csv"]
NOTES: ok
END_MOBIUS_CV_REVIEWER_RESULT"""


def test_mobius_cv_parser_accepts_and_rejects_contract_blocks() -> None:
    module = load_mobius_cv_mcp_module()
    parsed, errors = module.parse_reviewer_result_block(valid_reviewer_block(), "kimi-code", "exit_review")
    assert errors == []
    assert parsed["checked_acceptance_ids"] == ["A1"]

    malformed = [
        valid_reviewer_block().replace("\nEND_MOBIUS_CV_REVIEWER_RESULT", ""),
        "prefix\n" + valid_reviewer_block(),
        valid_reviewer_block(reviewer="codex-subagent"),
        valid_reviewer_block().replace('CHECKED_ACCEPTANCE_IDS: ["A1"]', "CHECKED_ACCEPTANCE_IDS: A1"),
    ]
    for text in malformed:
        parsed, errors = module.parse_reviewer_result_block(text, "kimi-code", "exit_review")
        assert parsed is None
        assert errors


def test_kimi_adapter_handles_auth_timeout_and_invalid_output() -> None:
    module = load_mobius_cv_mcp_module()
    previous_which = module.shutil.which
    try:
        module.shutil.which = lambda _name: "/usr/bin/kimi"
        module.discover_kimi = lambda deep=False: {"status": "ready", "supports": {"prompt": True}}

        module.run_kimi_review_command = lambda *args, **kwargs: {
            "status": "error",
            "exit_code": 1,
            "stdout": "",
            "stderr": "OAuth provider managed:kimi-code failed to fetch an access token from auth.kimi.com",
            "duration_seconds": 0.1,
        }
        auth = module.run_kimi_review("{}", "delta_review", ["A1"], 900)
        assert auth["status"] == "auth_unavailable"
        assert auth["retryable"] is False

        module.run_kimi_review_command = lambda *args, **kwargs: {
            "status": "hard_timeout",
            "exit_code": None,
            "stdout": "",
            "stderr": "",
            "duration_seconds": 1.0,
        }
        timeout = module.run_kimi_review("{}", "delta_review", ["A1"], 900)
        assert timeout["status"] == "hard_timeout"
        assert timeout["retryable"] is True

        module.run_kimi_review_command = lambda *args, **kwargs: {
            "status": "ok",
            "exit_code": 0,
            "stdout": '{"message":{"content":"not the required block"}}\n',
            "stderr": "",
            "duration_seconds": 0.1,
        }
        invalid = module.run_kimi_review("{}", "exit_review", ["A1"], 900)
        assert invalid["status"] == "invalid_output"
        assert invalid["verdict"] == "unknown"
    finally:
        module.shutil.which = previous_which


def test_kimi_stream_extractor_requires_assistant_role() -> None:
    module = load_mobius_cv_mcp_module()
    block = valid_reviewer_block(mode="exit_review")
    assert module.assistant_text_from_stream_json(json.dumps({"type": "tool_result", "content": block})) == ""
    assert module.assistant_text_from_stream_json(block) == ""
    assert module.assistant_text_from_stream_json(json.dumps({"role": "tool", "content": block})) == ""
    assert module.assistant_text_from_stream_json(json.dumps({"role": "assistant", "content": block})) == block


def test_kimi_startup_health_is_lazy_by_default() -> None:
    module = load_mobius_cv_mcp_module()
    calls: list[bool] = []

    def fake_discover(deep: bool = False) -> dict[str, object]:
        calls.append(deep)
        return {"id": "kimi-code", "status": "ready", "commands": ["chat"], "checks": [], "supports": {"prompt": True}}

    module.discover_kimi = fake_discover
    previous_child = os.environ.get("MOBIUS_CV_KIMI_CHILD")
    try:
        os.environ.pop("MOBIUS_CV_KIMI_CHILD", None)
        startup = module.startup_health()
        assert startup["status"] == "ready"
        assert calls[-1] is False

        health = module.mobius_cv_health(deep=True, include_commands=False)
        assert health["reviewers"][1]["status"] == "ready"
        assert calls[-1] is True
        assert "commands" not in health["reviewers"][1]
    finally:
        if previous_child is None:
            os.environ.pop("MOBIUS_CV_KIMI_CHILD", None)
        else:
            os.environ["MOBIUS_CV_KIMI_CHILD"] = previous_child


def test_mcp_missing_subagent_fails_before_reviewers_and_packet_consumption() -> None:
    module = load_mobius_cv_mcp_module()
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        slug, goal, _evidence = prepare_goal(root, "s", "mcp-missing-subagent")
        start_stage(root, "s", slug)
        packet = create_packet(root, "s", slug, "delta_review", ["A1"])
        reviewer_called = {"value": False}

        def fail_if_called(*args, **kwargs):
            reviewer_called["value"] = True
            raise AssertionError("reviewer should not run when codex_subagent_result is missing")

        module.run_kimi_review = fail_if_called
        recorded = module.review_and_record("delta_review", str(root), "s", slug, packet, 2, None, None, "P1", ["A1"], 900, "cv_delta_001", None)
        assert recorded["ok"] is False
        assert "codex_subagent_result is required" in recorded["errors"][0]
        assert reviewer_called["value"] is False
        assert mobius.packet_has_recorded_review(goal, str(packet["packet"])) is False


def test_mcp_registry_and_recorded_surface() -> None:
    module = load_mobius_cv_mcp_module()
    policies = {item["name"]: item for item in module.mobius_cv_registry()["review_policies"]}
    assert sorted(policies) == ["delta_kimi", "delta_light", "exit_strict"]
    assert policies["delta_light"]["required_reviewers"] == ["codex-subagent"]
    assert policies["delta_kimi"]["required_reviewers"] == ["codex-subagent", "kimi-code"]

    source = (ROOT / "scripts" / "mobius_cv_mcp.py").read_text(encoding="utf-8")
    assert "def mobius_cv_record_delta_review" in source
    assert "def mobius_cv_record_exit_review" in source
    assert "def mobius_cv_review_" + "delta(" not in source
    assert "packet_" + "path" not in source
    assert "packet_" + "text" not in source


def test_stop_hook_blocks_only_explicit_pending_goal() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        accepted_slug, accepted_goal, _evidence = prepare_goal(root, "s", "accepted")
        packet = create_packet(root, "s", accepted_slug, "exit_review")
        mobius.record_cv_result(root, "s", accepted_slug, cv_envelope("cv_exit_001", read_goal_id(accepted_goal), packet_id=str(packet["packet"])), "exit_review")
        pending_slug, _pending_goal, _pending_evidence = prepare_goal(root, "s", "pending")

        pending_payload = json.dumps({"session_id": "s", "goal_slug": pending_slug, "message": f"Mobius goal {pending_slug} completed"})
        assert run(["--project-root", str(root), "hook", "stop"], input_text=pending_payload, check=False).returncode == 2
        accepted_payload = json.dumps({"session_id": "s", "goal_slug": accepted_slug, "message": f"Mobius goal {accepted_slug} completed"})
        assert run(["--project-root", str(root), "hook", "stop"], input_text=accepted_payload, check=False).returncode == 0
        ordinary_payload = json.dumps({"session_id": "s", "message": "completed"})
        assert run(["--project-root", str(root), "hook", "stop"], input_text=ordinary_payload, check=False).returncode == 0


def test_pre_tool_hook_blocks_state_writes_and_allows_reads() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        _slug, goal, _evidence = prepare_goal(root, "s", "hook-state")
        ordinary = json.dumps({"tool_input": {"command": "echo complete"}})
        assert run(["--project-root", str(root), "hook", "pre-tool-use"], input_text=ordinary, check=False).returncode == 0

        read = json.dumps({"tool_input": {"command": f"cat {goal / 'plan.csv'}"}})
        assert run(["--project-root", str(root), "hook", "pre-tool-use"], input_text=read, check=False).returncode == 0

        sed_read = json.dumps({"tool_input": {"command": f"sed -n 1p {goal / 'plan.csv'}"}})
        assert run(["--project-root", str(root), "hook", "pre-tool-use"], input_text=sed_read, check=False).returncode == 0

        ignore_read = json.dumps({"tool_input": {"command": f"git check-ignore {goal / 'plan.csv'}"}})
        assert run(["--project-root", str(root), "hook", "pre-tool-use"], input_text=ignore_read, check=False).returncode == 0

        sed_write = json.dumps({"tool_input": {"command": f"sed -i s/schema/schema/ {goal / 'plan.csv'}"}})
        result = run(["--project-root", str(root), "hook", "pre-tool-use"], input_text=sed_write, check=False)
        assert result.returncode == 2
        assert "plan.csv is protected state" in result.stderr

        for target, expected in (
            (goal / "plan.csv", "plan.csv is protected state"),
            (goal / "review_attempts.csv", "review_attempts.csv is protected state"),
            (goal / "verdict.csv", "verdict.csv is derived state"),
        ):
            payload = json.dumps({"tool_input": {"command": f"cat /dev/null > {target}"}})
            result = run(["--project-root", str(root), "hook", "pre-tool-use"], input_text=payload, check=False)
            assert result.returncode == 2
            assert expected in result.stderr

        structured = json.dumps({"tool_name": "apply_patch", "tool_input": {"path": str(goal / "acceptance.csv")}})
        result = run(["--project-root", str(root), "hook", "pre-tool-use"], input_text=structured, check=False)
        assert result.returncode == 2
        assert "acceptance.csv is protected state" in result.stderr


def test_pre_tool_hook_does_not_scan_packet_create_commands() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        command = f"python3 scripts/mobius.py --project-root {root} packet-create --session-id s --goal-slug missing --review-mode exit_review"
        payload = json.dumps({"tool_input": {"command": command}})
        assert run(["--project-root", str(root), "hook", "pre-tool-use"], input_text=payload, check=False).returncode == 0
        narrative = json.dumps({"message": "please do not run packet-create --review-mode exit_review here"})
        assert run(["--project-root", str(root), "hook", "pre-tool-use"], input_text=narrative, check=False).returncode == 0


def test_hook_config_dispatches_current_events() -> None:
    data = json.loads((ROOT / "hooks" / "hooks.json").read_text(encoding="utf-8"))
    assert sorted(data["hooks"].keys()) == ["PreToolUse", "Stop"]
    for hook_name, expected_action in (("PreToolUse", "pre-tool-use"), ("Stop", "stop")):
        command = data["hooks"][hook_name][0]["hooks"][0]["command"]
        assert f'mobius-hook "${{PLUGIN_ROOT}}" hook {expected_action}' in command
        assert "mobius_hook_launcher.sh" in command
        env = os.environ.copy()
        env["PLUGIN_ROOT"] = str(ROOT)
        result = subprocess.run(command, input="{}", text=True, shell=True, executable="/bin/bash", capture_output=True, env=env, check=False)
        assert result.returncode == 0

        missing_root_env = dict(env)
        missing_root_env.pop("PLUGIN_ROOT", None)
        missing_root = subprocess.run(command, input="{}", text=True, shell=True, executable="/bin/bash", capture_output=True, env=missing_root_env, check=False)
        assert missing_root.returncode == 2
        assert "PLUGIN_ROOT missing" in missing_root.stderr

    with tempfile.TemporaryDirectory() as tmp:
        corrupt_root = Path(tmp)
        (corrupt_root / "scripts").mkdir()
        corrupt = subprocess.run(
            ["/bin/sh", str(ROOT / "scripts" / "mobius_hook_launcher.sh"), str(corrupt_root), "hook", "pre-tool-use"],
            input="{}",
            text=True,
            capture_output=True,
            check=False,
        )
        assert corrupt.returncode == 2
        assert "hook-corrupt-install" in corrupt.stderr

        env = os.environ.copy()
        env["PATH"] = str(corrupt_root)
        missing_python = subprocess.run(
            ["/bin/sh", str(ROOT / "scripts" / "mobius_hook_launcher.sh"), str(ROOT), "hook", "pre-tool-use"],
            input="{}",
            text=True,
            capture_output=True,
            env=env,
            check=False,
        )
        assert missing_python.returncode == 2
        assert "hook-runtime-missing: python3" in missing_python.stderr


def test_public_docs_skills_and_verify_surface() -> None:
    assert sorted(path.parent.name for path in (ROOT / "skills").glob("*/SKILL.md")) == ["mobius-loop", "mobius-plan"]
    docs_text = "\n".join(path.read_text(encoding="utf-8") for path in [*ROOT.glob("skills/*/SKILL.md"), *ROOT.glob("references/*.md")])
    source_and_docs = "\n".join(
        path.read_text(encoding="utf-8")
        for path in [
            ROOT / "scripts" / "mobius.py",
            REPO_ROOT / "tests" / "mobius_regression_tests.py",
            *ROOT.glob("skills/*/SKILL.md"),
            *ROOT.glob("references/*.md"),
        ]
    )
    removed_terms = [
        "goal_" + "re" + "ject" + "ed",
        "re" + "ject" + "ed",
        "failed_" + "acceptance_ids_json",
        "needs_" + "human",
        "needs_user_" + "authorization",
        "needs_external_" + "auth",
        "needs_missing_" + "tool",
        "delta_" + "packet_id",
        "exit_" + "packet_id",
    ]
    for term in removed_terms:
        assert term not in source_and_docs
    stale_public_action = "advance_to_next_" + "plan_item"
    assert stale_public_action not in docs_text
    mobius_source = (ROOT / "scripts" / "mobius.py").read_text(encoding="utf-8")
    assert stale_public_action not in mobius_source
    assert "recompute_" + "verdict" not in source_and_docs
    assert "stale_" + "verdict" not in mobius_source
    assert "stored_" + "verdict_is_stale" not in mobius_source
    assert "Skill/MCP/CLI/Hook Responsibility Boundary" in docs_text
    assert "mobius.plan" in docs_text
    assert "mobius.acceptance" in docs_text

    plan_skill = (ROOT / "skills" / "mobius-plan" / "SKILL.md").read_text(encoding="utf-8")
    assert "MobiusCV is a verifier, not objective evidence" in plan_skill
    assert '"evidence_required":[{"type":"mobiuscv_delta"' not in plan_skill

    loop_skill = (ROOT / "skills" / "mobius-loop" / "SKILL.md").read_text(encoding="utf-8")
    assert "Full Plan Loop" in loop_skill
    assert "Default to full-plan loop execution" in loop_skill
    assert "Do not stop after a passed delta gate" in loop_skill
    assert "mobius.loop" in docs_text
    assert "packet-read" in docs_text

    plan_yaml = (ROOT / "skills" / "mobius-plan" / "agents" / "openai.yaml").read_text(encoding="utf-8")
    loop_yaml = (ROOT / "skills" / "mobius-loop" / "agents" / "openai.yaml").read_text(encoding="utf-8")
    assert 'value: "mobius-cv"' not in plan_yaml
    assert 'type: "mcp"' in loop_yaml
    assert 'value: "mobius-cv"' in loop_yaml

    verify = (REPO_ROOT / "scripts" / "verify.sh").read_text(encoding="utf-8")
    assert re.search(r"/home/[A-Za-z0-9_.-]+", verify) is None
    assert re.search(r"/Users/[A-Za-z0-9_.-]+", verify) is None
    assert re.search(r"C:\\Users\\[A-Za-z0-9_.-]+", verify) is None
    assert 'expected_owner = "boman-ng"' in verify
    assert 'expected_slug = f"{expected_owner}/mobius"' in verify
    assert "PYTHONPYCACHEPREFIX" in verify
    assert "validate_bundle" in verify
    assert ".agents/plugins/marketplace.json" in verify
    assert "forbidden_plugin_source_paths" in verify
    assert "release_text_paths" in verify
    assert "mobius_cv_mcp_server.sh\" --self-check" in verify
    assert "hook-health" in verify
    assert "git -C \"$REPO_ROOT\" check-ignore -q .mobius" in verify


def test_mcp_launcher_self_check() -> None:
    config = json.loads((ROOT / ".mcp.json").read_text(encoding="utf-8"))
    server = config["mcpServers"]["mobius-cv"]
    assert server["command"] == "/bin/bash"
    assert server["args"] == ["./scripts/mobius_cv_mcp_server.sh"]

    uv_path = shutil.which("uv")
    assert uv_path, "uv is required for the MCP launcher self-check"
    with tempfile.TemporaryDirectory() as tmp:
        env = os.environ.copy()
        env.pop("PYTHONDONTWRITEBYTECODE", None)
        env["PATH"] = "/usr/bin:/bin"
        env["MOBIUS_CV_UV"] = uv_path
        result = subprocess.run(
            ["/bin/bash", str(ROOT / "scripts" / "mobius_cv_mcp_server.sh"), "--self-check"],
            cwd=tmp,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip().endswith("mobius-cv-launcher-ok")


TESTS = [
    test_plan_loop_packet_smoke_path,
    test_contract_defaults_are_explicit_and_lockable,
    test_contract_lock_rejects_zero_stage_goal,
    test_contract_add_stage_rejects_required_stage_without_required_acceptance,
    test_locked_goal_contract_is_frozen,
    test_contract_validation_rejects_bad_acceptance_and_verifiers,
    test_contract_add_stage_rejects_without_half_written_rows,
    test_contract_lock_hash_covers_structural_fields,
    test_contract_supersede_stage_is_transactional_and_explicit,
    test_contract_supersede_stage_blocks_active_dependents,
    test_continue_respects_dependencies,
    test_packet_read_recovers_existing_packet_without_csv_transport,
    test_loop_lifecycle_commands_return_mirrored_loop_actions,
    test_record_missing_evidence_loop_action_for_non_command_proof,
    test_unlocked_contract_blocks_loop_packet_and_recorded_review,
    test_evidence_requires_known_acceptance_and_structured_required_proof,
    test_packet_is_lightweight_index_and_hash_checked,
    test_file_ref_and_change_set_scope_evidence_are_compact_refs,
    test_evidence_path_boundaries_are_enforced,
    test_csv_row_shaped_packet_is_not_transport,
    test_recorded_delta_pass_updates_loop_and_records_policy,
    test_ledger_audit_reports_missing_exit_cv,
    test_exit_review_interruption_is_visible,
    test_status_lists_active_goals_and_loop_next_is_not_public,
    test_delta_light_policy_and_mcp_default_skip_kimi,
    test_raw_reviewer_result_is_ref_not_inline_blob,
    test_pass_cv_requires_recorded_review_policy,
    test_recorded_review_rejects_aggregate_mismatch_and_degraded_pass,
    test_delta_pass_requires_satisfied_proof_obligations,
    test_repairable_delta_fail_returns_repair_stage,
    test_repair_budget_exhaustion_stops_loop,
    test_delta_unknown_review_quality_routes_to_new_packet_before_budget_or_evidence,
    test_blocked_delta_returns_blocked_loop,
    test_public_loop_stop_reasons_are_closed_set,
    test_recorded_exit_pass_updates_acceptance_verdict_and_run,
    test_recorded_exit_failure_does_not_half_write_state,
    test_recorded_exit_failure_repairs_earliest_stage_and_blocked_is_terminal,
    test_recorded_exit_unknown_creates_new_exit_packet_not_evidence_action,
    test_recorded_exit_fail_with_degraded_or_unchecked_retries_exit_review,
    test_recorded_exit_fail_budget_exhaustion_persists_blocked_loop,
    test_packet_id_is_one_shot_for_delta_review,
    test_terminal_goal_blocks_mutations_and_recorded_reviews,
    test_mobius_cv_prompt_allows_autonomous_review_tools,
    test_mobius_cv_parser_accepts_and_rejects_contract_blocks,
    test_kimi_adapter_handles_auth_timeout_and_invalid_output,
    test_kimi_stream_extractor_requires_assistant_role,
    test_kimi_startup_health_is_lazy_by_default,
    test_mcp_missing_subagent_fails_before_reviewers_and_packet_consumption,
    test_mcp_registry_and_recorded_surface,
    test_stop_hook_blocks_only_explicit_pending_goal,
    test_pre_tool_hook_blocks_state_writes_and_allows_reads,
    test_pre_tool_hook_does_not_scan_packet_create_commands,
    test_hook_config_dispatches_current_events,
    test_public_docs_skills_and_verify_surface,
    test_mcp_launcher_self_check,
]


def main() -> int:
    for test in TESTS:
        test()
        print(f"ok {test.__name__}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
