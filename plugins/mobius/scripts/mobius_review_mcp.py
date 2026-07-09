#!/usr/bin/env python3
"""Mobius Review MCP server."""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from mcp.server.fastmcp import FastMCP

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))
import mobius


SERVER_VERSION = "0.5.0"
RESULT_SCHEMA = "mobius.review_result"
REVIEWER_SCHEMA = "mobius.reviewer_result"
VALID_REVIEW_MODES = {"checkpoint_review", "exit_review"}
VALID_LEVELS = {1, 2}
RESULT_START = "MOBIUS_REVIEW_RESULT"
RESULT_END = "END_MOBIUS_REVIEW_RESULT"
VALID_REVIEWER_VERDICTS = {"pass", "fail", "unknown", "blocked"}

INSTRUCTIONS = (
    "Mobius Review provides stateless recorded review gates for explicitly targeted Mobius "
    "objectives. Pass a frozen Review Target every time. Do not pass prior review chat as scope. "
    "Checkpoint review checks one work item; exit review checks the full criterion matrix. "
    "Reviewers audit evidence quality, assumptions, blind spots, disconfirmation, Goodhart risk, "
    "contract drift, staleness, and pruning concerns. Missing, unchecked, invalid, ambiguous, or "
    "degraded reviewer output is not a pass."
)

mcp = FastMCP(name="mobius-review", instructions=INSTRUCTIONS)
STARTED_AT = datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def now_iso() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def compact_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def run_command(args: list[str], timeout: int = 30, input_text: str | None = None) -> dict[str, Any]:
    try:
        completed = subprocess.run(args, input=input_text, text=True, capture_output=True, timeout=timeout, check=False)
    except FileNotFoundError:
        return {"status": "missing_cli", "exit_code": None, "stdout": "", "stderr": "", "args": args}
    except subprocess.TimeoutExpired as exc:
        return {"status": "timeout", "exit_code": None, "stdout": exc.stdout or "", "stderr": exc.stderr or "", "args": args}
    return {
        "status": "ok" if completed.returncode == 0 else "error",
        "exit_code": completed.returncode,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
        "args": args,
    }


def local_tool_schema(name: str, description: str) -> dict[str, str]:
    return {"name": name, "description": description}


def parse_result_block(text: str, expected_mode: str) -> dict[str, Any]:
    if not text:
        raise ValueError("codex_subagent_result is required")
    pattern = re.compile(rf"{RESULT_START}\s*(.*?)\s*{RESULT_END}", re.DOTALL)
    matches = pattern.findall(text)
    if len(matches) != 1:
        raise ValueError(f"expected exactly one {RESULT_START} block")
    fields: dict[str, str] = {}
    for raw_line in matches[0].splitlines():
        line = raw_line.strip()
        if not line or ":" not in line:
            continue
        key, value = line.split(":", 1)
        fields[key.strip().upper()] = value.strip()
    reviewer = fields.get("REVIEWER", "")
    review_mode = fields.get("REVIEW_MODE", "")
    verdict = fields.get("VERDICT", "")
    if review_mode != expected_mode:
        raise ValueError(f"review mode mismatch: expected {expected_mode}, got {review_mode}")
    if verdict not in VALID_REVIEWER_VERDICTS:
        raise ValueError(f"invalid verdict: {verdict}")

    def array_field(name: str) -> list[str]:
        value = fields.get(name, "[]")
        parsed = json.loads(value)
        if not isinstance(parsed, list):
            raise ValueError(f"{name} must be a JSON array")
        return [str(item) for item in parsed]

    feedback_action = fields.get("FEEDBACK_ACTION", "none") or "none"
    if feedback_action not in mobius.FEEDBACK_ACTIONS:
        raise ValueError(f"invalid feedback action: {feedback_action}")
    checked_criterion_ids = array_field("CHECKED_CRITERION_IDS")
    unchecked_criterion_ids = array_field("UNCHECKED_CRITERION_IDS")
    blocking_findings = array_field("BLOCKING_FINDINGS")
    required_revisions = array_field("REQUIRED_REVISIONS")
    if verdict == "pass":
        pass_errors: list[str] = []
        if unchecked_criterion_ids:
            pass_errors.append("pass reviewer result cannot include unchecked criteria")
        if blocking_findings:
            pass_errors.append("pass reviewer result cannot include blocking findings")
        if required_revisions:
            pass_errors.append("pass reviewer result cannot include required revisions")
        if feedback_action != "none":
            pass_errors.append("pass reviewer result requires FEEDBACK_ACTION: none")
        if pass_errors:
            raise ValueError("; ".join(pass_errors))
    return {
        "schema": REVIEWER_SCHEMA,
        "reviewer": reviewer or "codex-subagent",
        "review_mode": review_mode,
        "verdict": verdict,
        "checked_criterion_ids": checked_criterion_ids,
        "unchecked_criterion_ids": unchecked_criterion_ids,
        "blocking_findings": blocking_findings,
        "required_revisions": required_revisions,
        "evidence_checked": array_field("EVIDENCE_CHECKED"),
        "feedback_action": feedback_action,
        "notes": fields.get("NOTES", ""),
    }


def require_target_mode(target: dict[str, Any], expected_mode: str) -> dict[str, Any]:
    mode = str(target.get("mode") or target.get("review_mode") or "")
    if mode != expected_mode:
        raise ValueError(f"review target mode mismatch: expected {expected_mode}, got {mode or '<missing>'}")
    return target


def target_from_input(project_root: str, session_id: str, objective_slug: str, review_mode: str, review_target: Any = None, review_target_id: str | None = None) -> dict[str, Any]:
    if isinstance(review_target, dict) and review_target.get("schema") == "mobius.review_target":
        return require_target_mode(review_target, review_mode)
    if isinstance(review_target, dict) and isinstance(review_target.get("review_target"), dict):
        nested = review_target["review_target"]
        if nested.get("schema") == "mobius.review_target":
            return require_target_mode(nested, review_mode)
    if not review_target_id:
        raise ValueError("review_target or review_target_id is required")
    command = [
        sys.executable,
        str(SCRIPT_DIR / "mobius.py"),
        "--project-root",
        project_root,
        "review-target-read",
        "--session-id",
        session_id,
        "--objective-slug",
        objective_slug,
        "--review-mode",
        review_mode,
        "--review-target-id",
        review_target_id,
    ]
    loaded = run_command(command)
    if loaded["exit_code"] != 0:
        raise ValueError(loaded["stderr"] or loaded["stdout"] or "review target load failed")
    payload = json.loads(loaded["stdout"])
    return require_target_mode(payload["review_target"], review_mode)


def record_judgment(project_root: str, session_id: str, objective_slug: str, review_mode: str, review_target: dict[str, Any], reviewer_result: dict[str, Any], level: int) -> dict[str, Any]:
    review_target_id = str(review_target.get("review_target", ""))
    command = [
        sys.executable,
        str(SCRIPT_DIR / "mobius.py"),
        "--project-root",
        project_root,
        "review-judgment-record",
        "--session-id",
        session_id,
        "--objective-slug",
        objective_slug,
        "--review-target-id",
        review_target_id,
        "--reviewer",
        reviewer_result["reviewer"],
        "--verdict",
        reviewer_result["verdict"],
        "--checked-criteria-json",
        compact_json(reviewer_result["checked_criterion_ids"]),
        "--blocking-findings-json",
        compact_json(reviewer_result["blocking_findings"]),
        "--required-revisions-json",
        compact_json(reviewer_result["required_revisions"]),
        "--feedback-action",
        reviewer_result["feedback_action"],
        "--level",
        str(level),
    ]
    recorded = run_command(command)
    if recorded["exit_code"] != 0:
        return {
            "schema": RESULT_SCHEMA,
            "ok": False,
            "persisted": False,
            "review_mode": review_mode,
            "review_target_id": review_target_id,
            "errors": [recorded["stderr"] or recorded["stdout"] or "recording failed"],
        }
    payload = json.loads(recorded["stdout"])
    return {
        "schema": RESULT_SCHEMA,
        "ok": True,
        "persisted": True,
        "review_mode": review_mode,
        "review_target_id": review_target_id,
        "review_judgment_id": payload.get("review_judgment_id", ""),
        "gate": payload.get("gate", ""),
        "reviewer_result": reviewer_result,
        "cli": payload,
    }


@mcp.tool()
def mobius_review_health(deep: bool = False, include_commands: bool = False) -> dict[str, Any]:
    """Return local Mobius Review readiness."""
    uv = os.environ.get("MOBIUS_REVIEW_UV") or ""
    payload: dict[str, Any] = {
        "schema": "mobius.review_health",
        "ok": True,
        "server_version": SERVER_VERSION,
        "started_at": STARTED_AT,
        "review_block": RESULT_START,
        "uv_configured": bool(uv),
    }
    if include_commands:
        payload["commands"] = {"python": sys.executable, "mobius": str(SCRIPT_DIR / "mobius.py")}
    if deep:
        with tempfile.TemporaryDirectory(prefix="mobius-review-health-") as tmp:
            payload["workspace_probe"] = Path(tmp).is_dir()
    return payload


@mcp.tool()
def mobius_review_registry() -> dict[str, Any]:
    """Return Mobius Review tool and result contracts."""
    return {
        "schema": "mobius.review_registry",
        "server_version": SERVER_VERSION,
        "tools": [
            local_tool_schema("mobius_review_health", "local readiness diagnostics"),
            local_tool_schema("mobius_review_build_subagent_prompt", "build a stateless host reviewer prompt"),
            local_tool_schema("mobius_review_record_checkpoint_judgment", "persist a checkpoint review judgment"),
            local_tool_schema("mobius_review_record_exit_judgment", "persist an exit review judgment"),
        ],
        "reviewer_result_block": RESULT_START,
        "review_modes": sorted(VALID_REVIEW_MODES),
        "feedback_actions": sorted(mobius.FEEDBACK_ACTIONS),
    }


@mcp.tool()
def mobius_review_build_subagent_prompt(review_target: dict[str, Any], review_mode: str) -> dict[str, Any]:
    """Build the stateless host reviewer prompt for one Review Target."""
    if review_mode not in VALID_REVIEW_MODES:
        return {"schema": "mobius.review_prompt", "ok": False, "errors": [f"invalid review_mode: {review_mode}"]}
    criteria = review_target.get("criteria", [])
    prompt = f"""You are a stateless Mobius Review reviewer.

Review mode: {review_mode}
Review target id: {review_target.get('review_target', '')}
Objective: {review_target.get('objective', '')}
Criteria to check: {compact_json(criteria)}

Use local read-only inspection to verify the Review Target refs and evidence. Do not rely on prior
review chat. Review failure is feedback, not objective failure. Classify findings into one of:
repair_route, add_evidence, select_alternate_route, retry_review, contract_change_required, none.

Return exactly one block:

{RESULT_START}
REVIEWER: codex-subagent
REVIEW_MODE: {review_mode}
VERDICT: pass|fail|unknown|blocked
CHECKED_CRITERION_IDS: <json array>
UNCHECKED_CRITERION_IDS: <json array>
BLOCKING_FINDINGS: <json array>
REQUIRED_REVISIONS: <json array>
EVIDENCE_CHECKED: <json array>
FEEDBACK_ACTION: none|repair_route|add_evidence|select_alternate_route|retry_review|contract_change_required
NOTES: <short text>
{RESULT_END}
"""
    return {
        "schema": "mobius.review_prompt",
        "ok": True,
        "review_mode": review_mode,
        "review_target_id": review_target.get("review_target", ""),
        "prompt": prompt,
    }


@mcp.tool()
def mobius_review_record_checkpoint_judgment(
    project_root: str,
    session_id: str,
    objective_slug: str,
    work_item_id: str,
    review_target: dict[str, Any] | None = None,
    review_target_id: str | None = None,
    codex_subagent_result: str | None = None,
    level: int = 1,
) -> dict[str, Any]:
    """Persist one checkpoint Review Judgment."""
    try:
        if level not in VALID_LEVELS:
            raise ValueError("level must be 1 or 2")
        target = target_from_input(project_root, session_id, objective_slug, "checkpoint_review", review_target, review_target_id)
        if target.get("work_item_id") != work_item_id:
            raise ValueError("work item mismatch")
        reviewer_result = parse_result_block(codex_subagent_result or "", "checkpoint_review")
        return record_judgment(project_root, session_id, objective_slug, "checkpoint_review", target, reviewer_result, level)
    except Exception as exc:
        return {"schema": RESULT_SCHEMA, "ok": False, "persisted": False, "review_mode": "checkpoint_review", "errors": [str(exc)]}


@mcp.tool()
def mobius_review_record_exit_judgment(
    project_root: str,
    session_id: str,
    objective_slug: str,
    review_target: dict[str, Any] | None = None,
    review_target_id: str | None = None,
    codex_subagent_result: str | None = None,
    level: int = 2,
) -> dict[str, Any]:
    """Persist one exit Review Judgment."""
    try:
        if level not in VALID_LEVELS:
            raise ValueError("level must be 1 or 2")
        target = target_from_input(project_root, session_id, objective_slug, "exit_review", review_target, review_target_id)
        reviewer_result = parse_result_block(codex_subagent_result or "", "exit_review")
        return record_judgment(project_root, session_id, objective_slug, "exit_review", target, reviewer_result, level)
    except Exception as exc:
        return {"schema": RESULT_SCHEMA, "ok": False, "persisted": False, "review_mode": "exit_review", "errors": [str(exc)]}


if __name__ == "__main__":
    mcp.run()
