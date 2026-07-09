#!/usr/bin/env python3
"""Mobius v0.5 local ledger engine."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MOBIUS_VERSION = "0.5.0"
VERDICT_RULE = (
    "accepted iff every required work item and criterion is locked, every required criterion "
    "is supported by objective evidence, and a non-degraded exit review judgment passes"
)

RUN_FIELDS = ["schema", "run_id", "codex_session_id", "project_root", "created_at", "mobius_version", "objectives_json"]
OBJECTIVE_FIELDS = [
    "schema",
    "objective_id",
    "run_id",
    "objective_slug",
    "status",
    "created_at",
    "updated_at",
    "contract_path",
    "contract_sha256_tail",
    "locked",
    "locked_at",
    "locked_by",
]
WORK_ITEM_FIELDS = [
    "schema",
    "objective_id",
    "revision",
    "id",
    "title",
    "description",
    "contract_status",
    "required",
    "depends_on_json",
    "scope_json",
    "work_json",
    "gate_json",
    "recovery_json",
    "timebox_json",
    "criteria_ids_json",
    "locked",
    "locked_at",
    "locked_by",
    "lock_hash",
]
CRITERION_FIELDS = [
    "schema",
    "objective_id",
    "id",
    "work_item_id",
    "requirement",
    "observable_outcome",
    "evidence_required_json",
    "verifier_json",
    "review_focus_json",
    "required",
    "status",
    "evidence_ids_json",
    "review_judgment_id",
    "verified_at",
    "locked",
    "locked_at",
    "locked_by",
    "lock_hash",
]
ROUTE_FIELDS = [
    "schema",
    "objective_id",
    "id",
    "work_item_id",
    "criterion_ids_json",
    "rationale",
    "status",
    "created_at",
]
ROUTE_RUN_FIELDS = [
    "schema",
    "objective_id",
    "id",
    "route_id",
    "work_item_id",
    "status",
    "started_at",
    "finished_at",
    "timebox_ms",
    "failure_kind",
    "review_judgment_id",
]
BUDGET_FIELDS = [
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
EVIDENCE_FIELDS = ["schema", "id", "objective_id", "type", "summary", "supports_json", "artifact_json", "created_by", "created_at"]
REVIEW_TARGET_FIELDS = [
    "schema",
    "review_target_id",
    "objective_id",
    "objective_slug",
    "review_mode",
    "stateless",
    "work_item_id",
    "route_run_id",
    "created_at",
    "target_json",
    "target_sha256",
]
REVIEW_JUDGMENT_FIELDS = [
    "schema",
    "review_judgment_id",
    "objective_id",
    "review_target_id",
    "review_mode",
    "level",
    "stateless",
    "reviewers_json",
    "result_json",
    "feedback_action",
    "raw_ref",
    "raw_hash_tail",
    "returned_at",
]
REVIEW_RUN_FIELDS = [
    "schema",
    "review_run_id",
    "review_target_id",
    "review_mode",
    "status",
    "started_at",
    "finished_at",
    "reviewer_summary_ref",
    "failure_kind",
    "retryable",
    "diagnostic_ref",
]
VERDICT_FIELDS = [
    "schema",
    "objective_id",
    "overall",
    "adjudicated_by",
    "adjudicated_at",
    "rule",
    "derived_from_json",
    "unverified_work_item_ids_json",
    "unverified_criterion_ids_json",
    "blocked_criterion_ids_json",
]

LEDGER_FIELDS = {
    "run.csv": RUN_FIELDS,
    "objective.csv": OBJECTIVE_FIELDS,
    "work_items.csv": WORK_ITEM_FIELDS,
    "criteria.csv": CRITERION_FIELDS,
    "routes.csv": ROUTE_FIELDS,
    "route_runs.csv": ROUTE_RUN_FIELDS,
    "budget.csv": BUDGET_FIELDS,
    "evidence.csv": EVIDENCE_FIELDS,
    "review_targets.csv": REVIEW_TARGET_FIELDS,
    "review_judgments.csv": REVIEW_JUDGMENT_FIELDS,
    "review_runs.csv": REVIEW_RUN_FIELDS,
    "verdict.csv": VERDICT_FIELDS,
}
PROTECTED_LEDGER_FILENAMES = tuple(LEDGER_FIELDS)

EVIDENCE_TYPES = {"change_set_scope", "file_ref", "command_result", "test_result", "human_assertion"}
REVIEW_TYPES = {"checkpoint_review", "exit_review"}
JUDGMENT_VERDICTS = {"pass", "fail", "unknown", "blocked"}
FEEDBACK_ACTIONS = {"none", "repair_route", "add_evidence", "select_alternate_route", "retry_review", "contract_change_required"}
CLOCK_DOMAINS = {"harness_internal", "external_blocking", "external_detached", "mixed", "unknown"}
OBJECTIVE_STATUSES = {"planning", "active", "accepted", "blocked"}


def now_iso() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def parse_iso_ms(value: str) -> int | None:
    if not value:
        return None
    try:
        normalized = value.replace("Z", "+00:00")
        parsed = datetime.fromisoformat(normalized)
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        return int(parsed.timestamp() * 1000)
    except ValueError:
        return None


def json_dumps(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def parse_json_cell(value: str | None, default: Any) -> Any:
    if value is None or str(value).strip() == "":
        return default
    try:
        return json.loads(value)
    except json.JSONDecodeError:
        return default


def sha256_text(text: str) -> str:
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return "sha256:" + digest.hexdigest()


def slug_id(prefix: str, value: str) -> str:
    normalized = re.sub(r"[^A-Za-z0-9_]+", "_", value).strip("_").lower()
    return f"{prefix}_{normalized or 'objective'}"


def project_root(args: argparse.Namespace) -> Path:
    return Path(getattr(args, "project_root", "") or os.getcwd()).resolve()


def mobius_root(root: Path) -> Path:
    path = root / ".mobius"
    path.mkdir(parents=True, exist_ok=True)
    gitignore = path / ".gitignore"
    if not gitignore.exists():
        gitignore.write_text("*\n!.gitignore\n", encoding="utf-8")
    return path


def run_dir(root: Path, session_id: str) -> Path:
    path = mobius_root(root) / "runs" / f"codex-session-{session_id}"
    path.mkdir(parents=True, exist_ok=True)
    return path


def objective_dir(root: Path, session_id: str, objective_slug: str) -> Path:
    return run_dir(root, session_id) / objective_slug


def read_rows(path: Path) -> list[dict[str, str]]:
    if not path.exists():
        return []
    with path.open(newline="", encoding="utf-8") as handle:
        return [dict(row) for row in csv.DictReader(handle)]


def write_rows(path: Path, fields: list[str], rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        for row in rows:
            writer.writerow({field: str(row.get(field, "")) for field in fields})


def append_row(path: Path, fields: list[str], row: dict[str, Any]) -> None:
    rows = read_rows(path)
    rows.append({field: str(row.get(field, "")) for field in fields})
    write_rows(path, fields, rows)


def single_row(path: Path) -> dict[str, str]:
    rows = read_rows(path)
    return rows[0] if rows else {}


def json_print(payload: dict[str, Any], exit_code: int = 0) -> None:
    print(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
    raise SystemExit(exit_code)


def result(command: str, root: Path, objective_slug: str = "", objective_id: str = "", ok: bool = True, errors: list[str] | None = None, **extra: Any) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "schema": "mobius.command_result",
        "ok": ok,
        "command": command,
        "objective_id": objective_id,
        "objective_slug": objective_slug,
        "updated_files": extra.pop("updated_files", []),
        "gate": extra.pop("gate", ""),
        "next_required_action": extra.pop("next_required_action", ""),
        "errors": errors or [],
    }
    payload.update(extra)
    return payload


def require_objective(root: Path, session_id: str, objective_slug: str) -> tuple[Path, dict[str, str]]:
    path = objective_dir(root, session_id, objective_slug)
    row = single_row(path / "objective.csv")
    if not row:
        json_print(result("load-objective", root, objective_slug, ok=False, errors=[f"objective not found: {objective_slug}"]), 2)
    return path, row


def initialize_objective_ledgers(path: Path) -> None:
    for filename, fields in LEDGER_FIELDS.items():
        if filename == "run.csv":
            continue
        ledger = path / filename
        if not ledger.exists():
            write_rows(ledger, fields, [])


def next_id(rows: list[dict[str, str]], prefix: str, field: str = "id") -> str:
    return f"{prefix}_{len(rows) + 1:03d}"


def active_work_items(path: Path) -> list[dict[str, str]]:
    return [row for row in read_rows(path / "work_items.csv") if row.get("contract_status") != "superseded"]


def active_criteria(path: Path) -> list[dict[str, str]]:
    return [row for row in read_rows(path / "criteria.csv") if row.get("status") != "superseded"]


def criteria_by_id(path: Path) -> dict[str, dict[str, str]]:
    return {row.get("id", ""): row for row in active_criteria(path)}


def criteria_for_work_item(path: Path, work_item_id: str) -> list[dict[str, str]]:
    return [row for row in active_criteria(path) if row.get("work_item_id") == work_item_id]


def evidence_for_criterion(path: Path, criterion_id: str) -> list[dict[str, str]]:
    matches: list[dict[str, str]] = []
    for row in read_rows(path / "evidence.csv"):
        if criterion_id in [str(item) for item in parse_json_cell(row.get("supports_json"), [])]:
            matches.append(row)
    return matches


def unverified_criteria(path: Path) -> list[str]:
    return [row["id"] for row in active_criteria(path) if row.get("required", "true") == "true" and row.get("status") != "pass"]


def work_item_is_verified(path: Path, work_item_id: str) -> bool:
    required = [row for row in criteria_for_work_item(path, work_item_id) if row.get("required", "true") == "true"]
    return bool(required) and all(row.get("status") == "pass" for row in required)


def terminal_verdict(path: Path) -> str:
    row = single_row(path / "verdict.csv")
    overall = row.get("overall", "")
    return overall if overall in {"accepted", "blocked"} else ""


def write_verdict(path: Path, objective_id: str, overall: str) -> None:
    criteria = active_criteria(path)
    work_items = active_work_items(path)
    unverified = [row["id"] for row in criteria if row.get("required", "true") == "true" and row.get("status") != "pass"]
    blocked = [row["id"] for row in criteria if row.get("status") == "blocked"]
    work_unverified = [row["id"] for row in work_items if row.get("required", "true") == "true" and not work_item_is_verified(path, row["id"])]
    row = {
        "schema": "mobius.verdict",
        "objective_id": objective_id,
        "overall": overall,
        "adjudicated_by": "mobius_gate",
        "adjudicated_at": now_iso(),
        "rule": VERDICT_RULE,
        "derived_from_json": json_dumps(
            {
                "work_items_sha256": sha256_text(json_dumps(work_items)),
                "criteria_sha256": sha256_text(json_dumps(criteria)),
                "evidence_sha256": sha256_text(json_dumps(read_rows(path / "evidence.csv"))),
                "review_judgments_sha256": sha256_text(json_dumps(read_rows(path / "review_judgments.csv"))),
            }
        ),
        "unverified_work_item_ids_json": json_dumps(work_unverified),
        "unverified_criterion_ids_json": json_dumps(unverified),
        "blocked_criterion_ids_json": json_dumps(blocked),
    }
    write_rows(path / "verdict.csv", VERDICT_FIELDS, [row])


def validate_contract(path: Path) -> list[str]:
    errors: list[str] = []
    objective = single_row(path / "objective.csv")
    if not objective:
        errors.append("objective.csv is missing an objective row")
    work_items = active_work_items(path)
    criteria = active_criteria(path)
    if not work_items:
        errors.append("at least one work item is required")
    if not criteria:
        errors.append("at least one criterion is required")
    work_item_ids = {row.get("id", "") for row in work_items}
    criterion_ids = {row.get("id", "") for row in criteria}
    for index, row in enumerate(work_items):
        if row.get("schema") != "mobius.work_item":
            errors.append(f"work item {row.get('id', '')} has invalid schema")
        if index > 0 and not parse_json_cell(row.get("depends_on_json"), []):
            errors.append(f"work item {row.get('id', '')} must depend on a predecessor")
        for dep in parse_json_cell(row.get("depends_on_json"), []):
            if dep not in work_item_ids:
                errors.append(f"work item {row.get('id', '')} depends on unknown work item {dep}")
        linked = parse_json_cell(row.get("criteria_ids_json"), [])
        if not linked:
            errors.append(f"work item {row.get('id', '')} must link criteria")
        for criterion_id in linked:
            if criterion_id not in criterion_ids:
                errors.append(f"work item {row.get('id', '')} links unknown criterion {criterion_id}")
        timebox = parse_json_cell(row.get("timebox_json"), {})
        if int(timebox.get("route_run_timebox_ms", 0) or 0) <= 0:
            errors.append(f"work item {row.get('id', '')} requires positive route_run_timebox_ms")
        if "max_stage_attempts" in timebox or "attempt_limit" in timebox:
            errors.append(f"work item {row.get('id', '')} uses retry-count budgeting instead of a timebox")
    for row in criteria:
        if row.get("schema") != "mobius.criterion":
            errors.append(f"criterion {row.get('id', '')} has invalid schema")
        if row.get("work_item_id") not in work_item_ids:
            errors.append(f"criterion {row.get('id', '')} references unknown work item")
        evidence_required = parse_json_cell(row.get("evidence_required_json"), [])
        verifiers = parse_json_cell(row.get("verifier_json"), [])
        if not evidence_required:
            errors.append(f"criterion {row.get('id', '')} requires evidence")
        for item in evidence_required:
            kind = item.get("type") if isinstance(item, dict) else item
            if kind not in EVIDENCE_TYPES:
                errors.append(f"criterion {row.get('id', '')} has invalid evidence type {kind}")
        for item in verifiers:
            kind = item.get("type") if isinstance(item, dict) else item
            if kind not in EVIDENCE_TYPES | {"checkpoint_review", "exit_review"}:
                errors.append(f"criterion {row.get('id', '')} has invalid verifier type {kind}")
    return errors


def cmd_objective_start(args: argparse.Namespace) -> None:
    root = project_root(args)
    session_id = args.session_id
    objective_slug = args.slug
    objective_id = slug_id("objective", objective_slug)
    run = run_dir(root, session_id)
    path = run / objective_slug
    path.mkdir(parents=True, exist_ok=True)
    created_at = now_iso()
    run_row = {
        "schema": "mobius.run",
        "run_id": f"codex-session-{session_id}",
        "codex_session_id": session_id,
        "project_root": str(root),
        "created_at": created_at,
        "mobius_version": MOBIUS_VERSION,
        "objectives_json": json_dumps([objective_slug]),
    }
    write_rows(run / "run.csv", RUN_FIELDS, [run_row])
    non_claims = args.non_claim or []
    objective_md = (
        "+++\n"
        'schema = "mobius.objective_contract"\n'
        f'objective_id = "{objective_id}"\n'
        f'run_id = "codex-session-{session_id}"\n'
        f'objective_slug = "{objective_slug}"\n'
        f'title = "{args.title}"\n'
        f'created_at = "{created_at}"\n'
        f"non_claims = {json.dumps(non_claims, ensure_ascii=False)}\n"
        "+++\n\n"
        "## User Request\n\n"
        f"{args.user_request.strip()}\n"
    )
    (path / "objective.md").write_text(objective_md, encoding="utf-8")
    initialize_objective_ledgers(path)
    objective_row = {
        "schema": "mobius.objective_state",
        "objective_id": objective_id,
        "run_id": f"codex-session-{session_id}",
        "objective_slug": objective_slug,
        "status": "planning",
        "created_at": created_at,
        "updated_at": created_at,
        "contract_path": "objective.md",
        "contract_sha256_tail": sha256_text(objective_md)[-16:],
        "locked": "false",
    }
    write_rows(path / "objective.csv", OBJECTIVE_FIELDS, [objective_row])
    write_verdict(path, objective_id, "pending")
    json_print(result("objective-start", root, objective_slug, objective_id, gate="planning", updated_files=["run.csv", "objective.md", "objective.csv", "verdict.csv"], objective_dir=str(path)))


def normalize_criteria_json(raw: str, work_item_id: str) -> list[dict[str, Any]]:
    parsed = json.loads(raw)
    if not isinstance(parsed, list):
        raise ValueError("criteria JSON must be a list")
    rows: list[dict[str, Any]] = []
    for item in parsed:
        if not isinstance(item, dict):
            raise ValueError("each criterion must be an object")
        rows.append(
            {
                "id": str(item["id"]),
                "work_item_id": work_item_id,
                "requirement": str(item.get("requirement", "")),
                "observable_outcome": str(item.get("observable_outcome", "")),
                "evidence_required": item.get("evidence_required", []),
                "verifier": item.get("verifier", []),
                "review_focus": item.get("review_focus", []),
                "required": bool(item.get("required", True)),
            }
        )
    return rows


def cmd_contract_add_work_item(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    objective_id = objective["objective_id"]
    errors: list[str] = []
    try:
        criteria_specs = normalize_criteria_json(args.criteria_json, args.id)
    except Exception as exc:
        json_print(result("contract-add-work-item", root, args.objective_slug, objective_id, ok=False, errors=[str(exc)]), 2)
    existing = active_work_items(path)
    if args.id in {row.get("id") for row in existing}:
        errors.append(f"work item already exists: {args.id}")
    existing_criteria = criteria_by_id(path)
    for criterion in criteria_specs:
        if criterion["id"] in existing_criteria:
            errors.append(f"criterion already exists: {criterion['id']}")
    if errors:
        json_print(result("contract-add-work-item", root, args.objective_slug, objective_id, ok=False, errors=errors), 2)
    depends_on = json.loads(args.depends_on_json or "[]")
    timebox = json.loads(args.timebox_json)
    work_item = {
        "schema": "mobius.work_item",
        "objective_id": objective_id,
        "revision": args.revision,
        "id": args.id,
        "title": args.title,
        "description": args.description,
        "contract_status": "pending",
        "required": str(not args.optional).lower(),
        "depends_on_json": json_dumps(depends_on),
        "scope_json": json_dumps(json.loads(args.scope_json)),
        "work_json": json_dumps(json.loads(args.work_json)),
        "gate_json": json_dumps(json.loads(args.gate_json)),
        "recovery_json": json_dumps(json.loads(args.recovery_json)),
        "timebox_json": json_dumps(timebox),
        "criteria_ids_json": json_dumps([item["id"] for item in criteria_specs]),
        "locked": "false",
    }
    append_row(path / "work_items.csv", WORK_ITEM_FIELDS, work_item)
    for spec in criteria_specs:
        append_row(
            path / "criteria.csv",
            CRITERION_FIELDS,
            {
                "schema": "mobius.criterion",
                "objective_id": objective_id,
                "id": spec["id"],
                "work_item_id": args.id,
                "requirement": spec["requirement"],
                "observable_outcome": spec["observable_outcome"],
                "evidence_required_json": json_dumps(spec["evidence_required"]),
                "verifier_json": json_dumps(spec["verifier"]),
                "review_focus_json": json_dumps(spec["review_focus"]),
                "required": str(spec["required"]).lower(),
                "status": "unknown",
                "evidence_ids_json": "[]",
                "locked": "false",
            },
        )
    json_print(result("contract-add-work-item", root, args.objective_slug, objective_id, gate="planning", updated_files=["work_items.csv", "criteria.csv"], criteria_ids=[item["id"] for item in criteria_specs]))


def cmd_contract_validate(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    errors = validate_contract(path)
    json_print(result("contract-validate", root, args.objective_slug, objective["objective_id"], ok=not errors, errors=errors, gate="valid" if not errors else "invalid"))


def lock_hash(row: dict[str, str]) -> str:
    material = {key: value for key, value in row.items() if key not in {"locked", "locked_at", "locked_by", "lock_hash"}}
    return sha256_text(json_dumps(material))


def cmd_contract_lock(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    errors = validate_contract(path)
    if errors:
        json_print(result("contract-lock", root, args.objective_slug, objective["objective_id"], ok=False, errors=errors, gate="invalid"), 2)
    locked_at = now_iso()
    objective["status"] = "active"
    objective["updated_at"] = locked_at
    objective["locked"] = "true"
    objective["locked_at"] = locked_at
    objective["locked_by"] = args.locked_by
    write_rows(path / "objective.csv", OBJECTIVE_FIELDS, [objective])
    work_items = read_rows(path / "work_items.csv")
    for row in work_items:
        if row.get("contract_status") != "superseded":
            row["contract_status"] = "locked"
            row["locked"] = "true"
            row["locked_at"] = locked_at
            row["locked_by"] = args.locked_by
            row["lock_hash"] = lock_hash(row)
    write_rows(path / "work_items.csv", WORK_ITEM_FIELDS, work_items)
    criteria = read_rows(path / "criteria.csv")
    for row in criteria:
        if row.get("status") != "superseded":
            row["locked"] = "true"
            row["locked_at"] = locked_at
            row["locked_by"] = args.locked_by
            row["lock_hash"] = lock_hash(row)
    write_rows(path / "criteria.csv", CRITERION_FIELDS, criteria)
    write_verdict(path, objective["objective_id"], "pending")
    json_print(result("contract-lock", root, args.objective_slug, objective["objective_id"], gate="ready", updated_files=["objective.csv", "work_items.csv", "criteria.csv", "verdict.csv"]))


def latest_review_judgment_for_work_item(path: Path, work_item_id: str) -> tuple[dict[str, str], dict[str, str], dict[str, Any]]:
    targets = [row for row in read_rows(path / "review_targets.csv") if row.get("work_item_id") == work_item_id and row.get("review_mode") == "checkpoint_review"]
    target_ids = {row.get("review_target_id") for row in targets}
    judgments = [row for row in read_rows(path / "review_judgments.csv") if row.get("review_target_id") in target_ids]
    if not judgments:
        return {}, {}, {}
    judgment = judgments[-1]
    target = next((row for row in reversed(targets) if row.get("review_target_id") == judgment.get("review_target_id")), {})
    return target, judgment, parse_json_cell(judgment.get("result_json"), {})


def latest_exit_review_judgment(path: Path) -> tuple[dict[str, str], dict[str, str], dict[str, Any]]:
    targets = [row for row in read_rows(path / "review_targets.csv") if row.get("review_mode") == "exit_review"]
    target_ids = {row.get("review_target_id") for row in targets}
    judgments = [row for row in read_rows(path / "review_judgments.csv") if row.get("review_target_id") in target_ids]
    if not judgments:
        return {}, {}, {}
    judgment = judgments[-1]
    target = next((row for row in reversed(targets) if row.get("review_target_id") == judgment.get("review_target_id")), {})
    return target, judgment, parse_json_cell(judgment.get("result_json"), {})


def has_new_evidence_after_review(path: Path, criterion_ids: list[str], target_row: dict[str, str], timestamp: str) -> bool:
    target = parse_json_cell(target_row.get("target_json"), {})
    prior_coverage = target.get("coverage", {})
    covered_ids = {
        str(evidence_id)
        for criterion_id in criterion_ids
        for evidence_id in (prior_coverage.get(criterion_id, []) if isinstance(prior_coverage, dict) else [])
    }
    for row in read_rows(path / "evidence.csv"):
        if row.get("created_at", "") <= timestamp:
            if row.get("id") in covered_ids:
                continue
        supports = [str(item) for item in parse_json_cell(row.get("supports_json"), [])]
        if any(criterion_id in supports for criterion_id in criterion_ids) and row.get("id") not in covered_ids:
            return True
    return False


def has_route_run_after(path: Path, work_item_id: str, timestamp: str) -> bool:
    for row in read_rows(path / "route_runs.csv"):
        if row.get("work_item_id") != work_item_id:
            continue
        if row.get("status") == "running":
            return True
        if timestamp and row.get("started_at", "") > timestamp:
            return True
    return False


def work_item_for_criteria(path: Path, criterion_ids: list[str]) -> str:
    criteria = criteria_by_id(path)
    for criterion_id in criterion_ids:
        work_item_id = criteria.get(criterion_id, {}).get("work_item_id", "")
        if work_item_id:
            return work_item_id
    items = active_work_items(path)
    return items[0]["id"] if items else ""


def next_route_id(path: Path, work_item_id: str) -> str:
    prefix = f"route_{work_item_id.lower()}_"
    count = sum(1 for row in read_rows(path / "routes.csv") if row.get("id", "").startswith(prefix))
    return f"{prefix}{count + 1:03d}"


def impacted_criteria_from_review(path: Path, target_row: dict[str, str], result_json: dict[str, Any]) -> list[str]:
    target = parse_json_cell(target_row.get("target_json"), {})
    checked = [str(item) for item in result_json.get("checked_criterion_ids", [])]
    unchecked = [str(item) for item in result_json.get("unchecked_criterion_ids", [])]
    target_criteria = [str(item) for item in target.get("criteria", [])]
    impacted = [item for item in [*unchecked, *unverified_criteria(path)] if item in target_criteria]
    if impacted:
        return sorted(set(impacted))
    remaining = [item for item in target_criteria if item not in checked]
    return sorted(set(remaining or target_criteria))


def review_feedback_loop(
    path: Path,
    objective_slug: str,
    session_id: str,
    item: dict[str, str],
    latest_run: dict[str, str],
    criteria: list[dict[str, str]],
) -> dict[str, Any]:
    target, judgment, result_json = latest_review_judgment_for_work_item(path, item["id"])
    if not judgment or result_json.get("verdict") == "pass":
        return {}
    if target.get("route_run_id") and latest_run.get("id") and target.get("route_run_id") != latest_run.get("id"):
        return {}
    feedback_action = judgment.get("feedback_action") or "none"
    criterion_ids = [row["id"] for row in criteria if row.get("required", "true") == "true"]
    if feedback_action in {"repair_route", "add_evidence"} and has_new_evidence_after_review(path, criterion_ids, target, judgment.get("returned_at", "")):
        return {}
    if feedback_action == "retry_review":
        return {}
    if feedback_action == "select_alternate_route":
        route_id = next_route_id(path, item["id"])
        argv = [
            "route-run-start",
            "--session-id",
            session_id,
            "--objective-slug",
            objective_slug,
            "--work-item-id",
            item["id"],
            "--route-id",
            route_id,
            "--rationale",
            "selected after Review Feedback",
        ]
        return {
            "schema": "mobius.loop",
            "mode": "full_plan",
            "agent_must_continue": True,
            "agent_must_stop": False,
            "next_required_action": "select_alternate_route",
            "next_command": shlex.join(argv),
            "next_argv": argv,
            "next_actions": [{"type": "cli", "name": "start_alternate_route_run", "argv": argv}],
            "next_work_item_id": item["id"],
            "route_run_id": latest_run.get("id", ""),
            "review_target_id": target.get("review_target_id", ""),
            "review_judgment_id": judgment.get("review_judgment_id", ""),
            "review_feedback_action": feedback_action,
            "terminal_verdict": "",
            "stop_reason": "",
        }
    if feedback_action == "contract_change_required":
        return {
            "schema": "mobius.loop",
            "mode": "full_plan",
            "agent_must_continue": False,
            "agent_must_stop": True,
            "next_required_action": "contract_change_required",
            "next_command": "",
            "next_argv": [],
            "next_actions": [{"type": "host", "name": "revise_objective_contract"}],
            "next_work_item_id": item["id"],
            "route_run_id": latest_run.get("id", ""),
            "review_target_id": target.get("review_target_id", ""),
            "review_judgment_id": judgment.get("review_judgment_id", ""),
            "review_feedback_action": feedback_action,
            "terminal_verdict": terminal_verdict(path),
            "stop_reason": "contract_change_required",
        }
    if feedback_action not in FEEDBACK_ACTIONS or feedback_action == "none":
        feedback_action = "classify_review_feedback"
    return {
        "schema": "mobius.loop",
        "mode": "full_plan",
        "agent_must_continue": True,
        "agent_must_stop": False,
        "next_required_action": feedback_action,
        "next_command": "",
        "next_argv": [],
        "next_actions": [{"type": "host", "name": feedback_action, "review_judgment_id": judgment.get("review_judgment_id", "")}],
        "next_work_item_id": item["id"],
        "route_run_id": latest_run.get("id", ""),
        "review_target_id": target.get("review_target_id", ""),
        "review_judgment_id": judgment.get("review_judgment_id", ""),
        "review_feedback_action": judgment.get("feedback_action") or "none",
        "terminal_verdict": "",
        "stop_reason": "",
    }


def exit_review_feedback_loop(path: Path, objective_slug: str, session_id: str) -> dict[str, Any]:
    target, judgment, result_json = latest_exit_review_judgment(path)
    if not judgment or result_json.get("verdict") == "pass":
        return {}
    feedback_action = judgment.get("feedback_action") or "none"
    impacted = impacted_criteria_from_review(path, target, result_json)
    work_item_id = work_item_for_criteria(path, impacted)
    if feedback_action in {"repair_route", "add_evidence"} and has_new_evidence_after_review(path, impacted, target, judgment.get("returned_at", "")):
        return {}
    if feedback_action == "select_alternate_route" and work_item_id and has_route_run_after(path, work_item_id, judgment.get("returned_at", "")):
        return {}
    if feedback_action == "retry_review":
        argv = ["review-target-create", "--session-id", session_id, "--objective-slug", objective_slug, "--review-mode", "exit_review"]
        return {
            "schema": "mobius.loop",
            "mode": "full_plan",
            "agent_must_continue": True,
            "agent_must_stop": False,
            "next_required_action": "create_exit_review_target",
            "next_command": shlex.join(argv),
            "next_argv": argv,
            "next_actions": [{"type": "cli", "name": "retry_exit_review", "argv": argv}],
            "review_target_id": target.get("review_target_id", ""),
            "review_judgment_id": judgment.get("review_judgment_id", ""),
            "review_feedback_action": feedback_action,
            "impacted_criterion_ids": impacted,
            "terminal_verdict": "",
            "stop_reason": "",
        }
    if feedback_action == "select_alternate_route":
        route_id = next_route_id(path, work_item_id) if work_item_id else ""
        argv = [
            "route-run-start",
            "--session-id",
            session_id,
            "--objective-slug",
            objective_slug,
            "--work-item-id",
            work_item_id,
            "--route-id",
            route_id,
            "--rationale",
            "selected after Exit Review Feedback",
        ]
        return {
            "schema": "mobius.loop",
            "mode": "full_plan",
            "agent_must_continue": True,
            "agent_must_stop": False,
            "next_required_action": "select_alternate_route",
            "next_command": shlex.join(argv),
            "next_argv": argv if work_item_id else [],
            "next_actions": [{"type": "cli", "name": "start_alternate_route_run", "argv": argv}] if work_item_id else [{"type": "host", "name": "select_alternate_route"}],
            "next_work_item_id": work_item_id,
            "review_target_id": target.get("review_target_id", ""),
            "review_judgment_id": judgment.get("review_judgment_id", ""),
            "review_feedback_action": feedback_action,
            "impacted_criterion_ids": impacted,
            "terminal_verdict": "",
            "stop_reason": "",
        }
    if feedback_action == "contract_change_required":
        return {
            "schema": "mobius.loop",
            "mode": "full_plan",
            "agent_must_continue": False,
            "agent_must_stop": True,
            "next_required_action": "contract_change_required",
            "next_command": "",
            "next_argv": [],
            "next_actions": [{"type": "host", "name": "revise_objective_contract"}],
            "next_work_item_id": work_item_id,
            "review_target_id": target.get("review_target_id", ""),
            "review_judgment_id": judgment.get("review_judgment_id", ""),
            "review_feedback_action": feedback_action,
            "impacted_criterion_ids": impacted,
            "terminal_verdict": terminal_verdict(path),
            "stop_reason": "contract_change_required",
        }
    if feedback_action not in FEEDBACK_ACTIONS or feedback_action == "none":
        feedback_action = "classify_review_feedback"
    return {
        "schema": "mobius.loop",
        "mode": "full_plan",
        "agent_must_continue": True,
        "agent_must_stop": False,
        "next_required_action": feedback_action,
        "next_command": "",
        "next_argv": [],
        "next_actions": [{"type": "host", "name": feedback_action, "review_judgment_id": judgment.get("review_judgment_id", "")}],
        "next_work_item_id": work_item_id,
        "review_target_id": target.get("review_target_id", ""),
        "review_judgment_id": judgment.get("review_judgment_id", ""),
        "review_feedback_action": judgment.get("feedback_action") or "none",
        "impacted_criterion_ids": impacted,
        "terminal_verdict": "",
        "stop_reason": "",
    }


def current_loop(path: Path, objective_slug: str, objective_id: str, session_id: str) -> dict[str, Any]:
    terminal = terminal_verdict(path)
    if terminal:
        return {
            "schema": "mobius.loop",
            "mode": "full_plan",
            "agent_must_continue": False,
            "agent_must_stop": True,
            "next_required_action": "objective_terminal",
            "next_command": "",
            "next_argv": [],
            "next_actions": [],
            "terminal_verdict": terminal,
            "stop_reason": terminal,
        }
    for item in active_work_items(path):
        criteria = criteria_for_work_item(path, item["id"])
        if criteria and all(row.get("status") == "pass" for row in criteria if row.get("required", "true") == "true"):
            continue
        route_runs = [row for row in read_rows(path / "route_runs.csv") if row.get("work_item_id") == item["id"]]
        latest_run = route_runs[-1] if route_runs else {}
        if not latest_run or latest_run.get("status") in {"failed", "expired"}:
            argv = [
                "route-run-start",
                "--session-id",
                session_id,
                "--objective-slug",
                objective_slug,
                "--work-item-id",
                item["id"],
            ]
            return {
                "schema": "mobius.loop",
                "mode": "full_plan",
                "agent_must_continue": True,
                "agent_must_stop": False,
                "next_required_action": "start_route_run",
                "next_command": shlex.join(argv),
                "next_argv": argv,
                "next_actions": [{"type": "cli", "name": "start_route_run", "argv": argv}],
                "next_work_item_id": item["id"],
                "route_run_id": "",
                "review_target_id": "",
                "terminal_verdict": "",
                "stop_reason": "",
            }
        missing = [row["id"] for row in criteria if not evidence_for_criterion(path, row["id"])]
        if missing:
            return {
                "schema": "mobius.loop",
                "mode": "full_plan",
                "agent_must_continue": True,
                "agent_must_stop": False,
                "next_required_action": "record_missing_evidence",
                "next_command": "",
                "next_argv": [],
                "next_actions": [{"type": "host", "name": "record_evidence", "criterion_ids": missing}],
                "next_work_item_id": item["id"],
                "route_run_id": latest_run["id"],
                "missing_criterion_ids": missing,
                "review_target_id": "",
                "terminal_verdict": "",
                "stop_reason": "",
            }
        targets = [
            row
            for row in read_rows(path / "review_targets.csv")
            if row.get("work_item_id") == item["id"] and row.get("review_mode") == "checkpoint_review"
        ]
        judged_targets = {row.get("review_target_id") for row in read_rows(path / "review_judgments.csv")}
        open_targets = [row for row in targets if row.get("review_target_id") not in judged_targets]
        if open_targets:
            target = open_targets[-1]
            return {
                "schema": "mobius.loop",
                "mode": "full_plan",
                "agent_must_continue": True,
                "agent_must_stop": False,
                "next_required_action": "record_review_judgment",
                "next_command": "",
                "next_argv": [],
                "next_actions": [{"type": "mcp", "name": "record_checkpoint_judgment", "review_target_id": target["review_target_id"]}],
                "next_work_item_id": item["id"],
                "route_run_id": latest_run["id"],
                "review_target_id": target["review_target_id"],
                "terminal_verdict": "",
                "stop_reason": "",
            }
        feedback = review_feedback_loop(path, objective_slug, session_id, item, latest_run, criteria)
        if feedback:
            return feedback
        argv = [
            "review-target-create",
            "--session-id",
            session_id,
            "--objective-slug",
            objective_slug,
            "--review-mode",
            "checkpoint_review",
            "--work-item-id",
            item["id"],
        ]
        return {
            "schema": "mobius.loop",
            "mode": "full_plan",
            "agent_must_continue": True,
            "agent_must_stop": False,
            "next_required_action": "create_review_target",
            "next_command": shlex.join(argv),
            "next_argv": argv,
            "next_actions": [{"type": "cli", "name": "create_review_target", "argv": argv}],
            "next_work_item_id": item["id"],
            "route_run_id": latest_run["id"],
            "review_target_id": "",
            "terminal_verdict": "",
            "stop_reason": "",
        }
    exit_targets = [row for row in read_rows(path / "review_targets.csv") if row.get("review_mode") == "exit_review"]
    judged_targets = {row.get("review_target_id") for row in read_rows(path / "review_judgments.csv")}
    open_exit = [row for row in exit_targets if row.get("review_target_id") not in judged_targets]
    if open_exit:
        target = open_exit[-1]
        return {
            "schema": "mobius.loop",
            "mode": "full_plan",
            "agent_must_continue": True,
            "agent_must_stop": False,
            "next_required_action": "record_exit_judgment",
            "next_command": "",
            "next_argv": [],
            "next_actions": [{"type": "mcp", "name": "record_exit_judgment", "review_target_id": target["review_target_id"]}],
            "review_target_id": target["review_target_id"],
            "terminal_verdict": "",
            "stop_reason": "",
        }
    feedback = exit_review_feedback_loop(path, objective_slug, session_id)
    if feedback:
        return feedback
    argv = ["review-target-create", "--session-id", session_id, "--objective-slug", objective_slug, "--review-mode", "exit_review"]
    return {
        "schema": "mobius.loop",
        "mode": "full_plan",
        "agent_must_continue": True,
        "agent_must_stop": False,
        "next_required_action": "create_exit_review_target",
        "next_command": shlex.join(argv),
        "next_argv": argv,
        "next_actions": [{"type": "cli", "name": "create_exit_review_target", "argv": argv}],
        "review_target_id": "",
        "terminal_verdict": "",
        "stop_reason": "",
    }


def audit_payload(path: Path, session_id: str, objective_slug: str, objective_id: str) -> dict[str, Any]:
    loop = current_loop(path, objective_slug, objective_id, session_id)
    return {
        "schema": "mobius.ledger_audit",
        "objective_dir": str(path),
        "session_id": session_id,
        "objective_slug": objective_slug,
        "loop_gate": "terminal" if loop.get("terminal_verdict") else "ready",
        "terminal_verdict": loop.get("terminal_verdict", ""),
        "next_required_action": loop.get("next_required_action", ""),
        "next_work_item_id": loop.get("next_work_item_id", ""),
        "review_target_id": loop.get("review_target_id", ""),
        "unverified_criterion_ids": unverified_criteria(path),
        "final_unverified_criterion_ids": unverified_criteria(path),
        "route_runs": read_rows(path / "route_runs.csv"),
        "budget_events": len(read_rows(path / "budget.csv")),
        "loop": loop,
    }


def cmd_explain(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    audit = audit_payload(path, args.session_id, args.objective_slug, objective["objective_id"])
    loop = audit["loop"]
    json_print(
        result(
            "explain",
            root,
            args.objective_slug,
            objective["objective_id"],
            gate=audit["loop_gate"],
            next_required_action=loop.get("next_required_action", ""),
            audit=audit,
            loop=loop,
            unverified_criterion_ids=audit["unverified_criterion_ids"],
            next_argv=loop.get("next_argv", []),
            next_actions=loop.get("next_actions", []),
        )
    )


def cmd_loop_status(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    audit = audit_payload(path, args.session_id, args.objective_slug, objective["objective_id"])
    json_print(result("loop-status", root, args.objective_slug, objective["objective_id"], gate=audit["loop_gate"], next_required_action=audit["next_required_action"], audit=audit, loop=audit["loop"], rows=read_rows(path / "route_runs.csv")))


def cmd_ledger_audit(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    audit = audit_payload(path, args.session_id, args.objective_slug, objective["objective_id"])
    json_print(result("ledger-audit", root, args.objective_slug, objective["objective_id"], gate=audit["loop_gate"], next_required_action=audit["next_required_action"], audit=audit, loop=audit["loop"]))


def cmd_continue(args: argparse.Namespace) -> None:
    cmd_ledger_audit(args)


def cmd_route_run_start(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    work_items = {row["id"]: row for row in active_work_items(path)}
    if args.work_item_id not in work_items:
        json_print(result("route-run-start", root, args.objective_slug, objective["objective_id"], ok=False, errors=[f"unknown work item: {args.work_item_id}"]), 2)
    item = work_items[args.work_item_id]
    criteria_ids = parse_json_cell(item.get("criteria_ids_json"), [])
    route_id = args.route_id or f"route_{args.work_item_id.lower()}_001"
    routes = read_rows(path / "routes.csv")
    if route_id not in {row.get("id") for row in routes}:
        append_row(
            path / "routes.csv",
            ROUTE_FIELDS,
            {
                "schema": "mobius.route",
                "objective_id": objective["objective_id"],
                "id": route_id,
                "work_item_id": args.work_item_id,
                "criterion_ids_json": json_dumps(criteria_ids),
                "rationale": args.rationale or "selected route",
                "status": "active",
                "created_at": now_iso(),
            },
        )
    timebox = parse_json_cell(item.get("timebox_json"), {})
    timebox_ms = int(args.timebox_ms or timebox.get("route_run_timebox_ms", 0) or 0)
    route_run_id = next_id(read_rows(path / "route_runs.csv"), "route_run")
    started_at = now_iso()
    append_row(
        path / "route_runs.csv",
        ROUTE_RUN_FIELDS,
        {
            "schema": "mobius.route_run",
            "objective_id": objective["objective_id"],
            "id": route_run_id,
            "route_id": route_id,
            "work_item_id": args.work_item_id,
            "status": "running",
            "started_at": started_at,
            "timebox_ms": timebox_ms,
        },
    )
    append_budget(path, objective["objective_id"], "route_run_started", "harness_internal", True, started_at=started_at, work_item_id=args.work_item_id, route_id=route_id, route_run_id=route_run_id, remaining_ms=timebox_ms, source="mobius_cli")
    json_print(result("route-run-start", root, args.objective_slug, objective["objective_id"], gate="running", updated_files=["routes.csv", "route_runs.csv", "budget.csv"], route_run_id=route_run_id))


def safe_int(value: str | int | None, default: int = 0) -> int:
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return default


def parse_bool(value: Any) -> bool | None:
    if value is None:
        return None
    if isinstance(value, bool):
        return value
    text = str(value).strip().lower()
    if text in {"1", "true", "yes", "y", "on"}:
        return True
    if text in {"0", "false", "no", "n", "off"}:
        return False
    return None


def sync_route_run_timebox(path: Path, route_run_id: str, budget_id: str, finished_at: str = "") -> dict[str, Any]:
    if not route_run_id:
        return {"updated_files": [], "remaining_ms": ""}
    route_runs = read_rows(path / "route_runs.csv")
    route_run = next((row for row in route_runs if row.get("id") == route_run_id), None)
    if not route_run:
        return {"updated_files": [], "remaining_ms": ""}
    timebox_ms = safe_int(route_run.get("timebox_ms"))
    consumed_ms = sum(
        safe_int(row.get("consumed_ms"))
        for row in read_rows(path / "budget.csv")
        if row.get("route_run_id") == route_run_id
        and row.get("metered") == "true"
        and row.get("clock_domain") in {"harness_internal", "mixed"}
    )
    remaining_ms = max(timebox_ms - consumed_ms, 0) if timebox_ms else ""
    updated_files: list[str] = []
    budget_rows = read_rows(path / "budget.csv")
    for row in budget_rows:
        if row.get("id") == budget_id:
            row["remaining_ms"] = str(remaining_ms)
            updated_files.append("budget.csv")
            break
    if updated_files:
        write_rows(path / "budget.csv", BUDGET_FIELDS, budget_rows)
    if timebox_ms and consumed_ms >= timebox_ms and route_run.get("status") == "running":
        route_run["status"] = "expired"
        route_run["finished_at"] = finished_at or now_iso()
        route_run["failure_kind"] = "timebox_expired"
        write_rows(path / "route_runs.csv", ROUTE_RUN_FIELDS, route_runs)
        updated_files.append("route_runs.csv")
    return {"updated_files": sorted(set(updated_files)), "remaining_ms": remaining_ms, "consumed_ms": consumed_ms}


def classify_imported_timing(event: dict[str, Any], event_kind: str, duration_ms: int) -> dict[str, Any]:
    explicit_domain = str(event.get("clock_domain") or event.get("mobius_clock_domain") or "").strip()
    failure_kind = ""
    inferred_domain = ""
    if explicit_domain:
        if explicit_domain in CLOCK_DOMAINS:
            inferred_domain = explicit_domain
        else:
            inferred_domain = "unknown"
            failure_kind = "invalid_clock_domain"
    elif event_kind in {"model_generation", "model_request", "model_response", "assistant_generation", "llm_call"}:
        inferred_domain = "harness_internal"
    else:
        inferred_domain = "unknown"

    explicit_consumed = event.get("consumed_ms")
    consumed_ms = safe_int(explicit_consumed) if explicit_consumed is not None else None
    explicit_metered = parse_bool(event.get("metered"))
    if inferred_domain == "harness_internal":
        metered = explicit_metered if explicit_metered is not None else duration_ms > 0
        if consumed_ms is None and metered:
            consumed_ms = duration_ms
    elif inferred_domain == "mixed":
        metered = explicit_metered if explicit_metered is not None else consumed_ms is not None
        if metered and consumed_ms is None:
            metered = False
            consumed_ms = 0
            failure_kind = failure_kind or "missing_consumed_ms"
    else:
        metered = explicit_metered if explicit_metered is not None and consumed_ms is not None else False
        if consumed_ms is None:
            consumed_ms = 0
    return {
        "clock_domain": inferred_domain,
        "metered": bool(metered),
        "consumed_ms": consumed_ms,
        "failure_kind": failure_kind,
    }


def append_budget(
    path: Path,
    objective_id: str,
    event_kind: str,
    clock_domain: str,
    metered: bool,
    *,
    source: str,
    started_at: str = "",
    finished_at: str = "",
    duration_ms: int | str = 0,
    consumed_ms: int | str | None = None,
    remaining_ms: int | str = "",
    failure_kind: str = "",
    work_item_id: str = "",
    criterion_id: str = "",
    route_id: str = "",
    route_run_id: str = "",
    review_target_id: str = "",
    review_run_id: str = "",
    tool_call_id: str = "",
) -> str:
    budget_id = next_id(read_rows(path / "budget.csv"), "budget")
    if consumed_ms is None:
        consumed_ms = duration_ms if metered and clock_domain == "harness_internal" else 0
    append_row(
        path / "budget.csv",
        BUDGET_FIELDS,
        {
            "schema": "mobius.budget_event",
            "id": budget_id,
            "objective_id": objective_id,
            "work_item_id": work_item_id,
            "criterion_id": criterion_id,
            "route_id": route_id,
            "route_run_id": route_run_id,
            "review_target_id": review_target_id,
            "review_run_id": review_run_id,
            "tool_call_id": tool_call_id,
            "event_kind": event_kind,
            "clock_domain": clock_domain,
            "metered": str(metered).lower(),
            "source": source,
            "started_at": started_at,
            "finished_at": finished_at,
            "duration_ms": duration_ms,
            "consumed_ms": consumed_ms,
            "remaining_ms": remaining_ms,
            "failure_kind": failure_kind,
            "created_at": now_iso(),
        },
    )
    return budget_id


def cmd_budget_add(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    if args.clock_domain not in CLOCK_DOMAINS:
        json_print(result("budget-add", root, args.objective_slug, objective["objective_id"], ok=False, errors=[f"invalid clock domain: {args.clock_domain}"]), 2)
    metered = str(args.metered).lower() in {"1", "true", "yes"}
    if args.clock_domain == "mixed" and metered and args.consumed_ms is None:
        json_print(result("budget-add", root, args.objective_slug, objective["objective_id"], ok=False, errors=["mixed metered tool time requires explicit consumed_ms"]), 2)
    budget_id = append_budget(
        path,
        objective["objective_id"],
        args.event_kind,
        args.clock_domain,
        metered,
        source=args.source,
        started_at=args.started_at or "",
        finished_at=args.finished_at or "",
        duration_ms=args.duration_ms,
        consumed_ms=args.consumed_ms,
        failure_kind=args.failure_kind or "",
        tool_call_id=args.tool_call_id or "",
        work_item_id=args.work_item_id or "",
        criterion_id=args.criterion_id or "",
        route_id=args.route_id or "",
        route_run_id=args.route_run_id or "",
        review_target_id=args.review_target_id or "",
        review_run_id=args.review_run_id or "",
    )
    sync = sync_route_run_timebox(path, args.route_run_id or "", budget_id, args.finished_at or "")
    updated = sorted(set(["budget.csv", *sync.get("updated_files", [])]))
    json_print(
        result(
            "budget-add",
            root,
            args.objective_slug,
            objective["objective_id"],
            gate="recorded",
            updated_files=updated,
            budget_id=budget_id,
            route_run_consumed_ms=sync.get("consumed_ms", ""),
            route_run_remaining_ms=sync.get("remaining_ms", ""),
        )
    )


def supports_from_args(args: argparse.Namespace) -> list[str]:
    supports = list(args.supports or [])
    if args.supports_json:
        parsed = json.loads(args.supports_json)
        if isinstance(parsed, str):
            supports.append(parsed)
        elif isinstance(parsed, list):
            supports.extend(str(item) for item in parsed)
        else:
            raise ValueError("supports-json must be a JSON string or array")
    return sorted(set(supports))


def cmd_evidence_add(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    if args.type not in EVIDENCE_TYPES:
        json_print(result("evidence-add", root, args.objective_slug, objective["objective_id"], ok=False, errors=[f"invalid evidence type: {args.type}"]), 2)
    try:
        supports = supports_from_args(args)
    except Exception as exc:
        json_print(result("evidence-add", root, args.objective_slug, objective["objective_id"], ok=False, errors=[str(exc)]), 2)
    unknown = [item for item in supports if item not in criteria_by_id(path)]
    if unknown:
        json_print(result("evidence-add", root, args.objective_slug, objective["objective_id"], ok=False, errors=[f"unknown criterion ids: {', '.join(unknown)}"]), 2)
    artifact = json.loads(args.artifact_json or "{}")
    if args.artifact:
        rel = Path(args.artifact)
        abs_path = (root / rel).resolve()
        if not abs_path.is_file() or not str(abs_path).startswith(str(root)):
            json_print(result("evidence-add", root, args.objective_slug, objective["objective_id"], ok=False, errors=["file_ref artifact must be an existing project-root-relative file"]), 2)
        artifact.update({"type": "file_ref", "path": rel.as_posix(), "sha256": sha256_file(abs_path)})
    evidence_id = next_id(read_rows(path / "evidence.csv"), "evidence")
    append_row(
        path / "evidence.csv",
        EVIDENCE_FIELDS,
        {
            "schema": "mobius.evidence",
            "id": evidence_id,
            "objective_id": objective["objective_id"],
            "type": args.type,
            "summary": args.summary,
            "supports_json": json_dumps(supports),
            "artifact_json": json_dumps(artifact),
            "created_by": args.created_by,
            "created_at": now_iso(),
        },
    )
    json_print(result("evidence-add", root, args.objective_slug, objective["objective_id"], gate="evidence_recorded", updated_files=["evidence.csv"], evidence_id=evidence_id))


def cmd_evidence_list(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    rows = read_rows(path / "evidence.csv")
    if args.criterion_id:
        rows = [row for row in rows if args.criterion_id in parse_json_cell(row.get("supports_json"), [])]
    json_print(result("evidence-list", root, args.objective_slug, objective["objective_id"], gate="read_only", evidence=rows))


def latest_running_route_run(path: Path, work_item_id: str) -> dict[str, str]:
    rows = [row for row in read_rows(path / "route_runs.csv") if row.get("work_item_id") == work_item_id and row.get("status") == "running"]
    return rows[-1] if rows else {}


def cmd_review_target_create(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    if args.review_mode not in REVIEW_TYPES:
        json_print(result("review-target-create", root, args.objective_slug, objective["objective_id"], ok=False, errors=[f"invalid review mode: {args.review_mode}"]), 2)
    all_criteria = active_criteria(path)
    if args.review_mode == "checkpoint_review":
        if not args.work_item_id:
            json_print(result("review-target-create", root, args.objective_slug, objective["objective_id"], ok=False, errors=["checkpoint_review requires --work-item-id"]), 2)
        target_criteria = criteria_for_work_item(path, args.work_item_id)
        route_run = latest_running_route_run(path, args.work_item_id)
    else:
        target_criteria = all_criteria
        route_run = {}
    if not target_criteria:
        json_print(result("review-target-create", root, args.objective_slug, objective["objective_id"], ok=False, errors=["review target has no criteria"]), 2)
    coverage = {row["id"]: [e["id"] for e in evidence_for_criterion(path, row["id"])] for row in target_criteria}
    missing = [criterion_id for criterion_id, ids in coverage.items() if not ids]
    if missing:
        json_print(result("review-target-create", root, args.objective_slug, objective["objective_id"], ok=False, errors=[f"missing evidence for criteria: {', '.join(missing)}"], missing_criterion_ids=missing), 2)
    review_target_id = next_id(read_rows(path / "review_targets.csv"), "review_target")
    target = {
        "schema": "mobius.review_target",
        "review_target": review_target_id,
        "objective": args.objective_slug,
        "objective_id": objective["objective_id"],
        "mode": args.review_mode,
        "work_item_id": args.work_item_id or "",
        "route_run_id": route_run.get("id", ""),
        "coverage": coverage,
        "refs": {
            "objective": "objective.md",
            "work_items": "work_items.csv",
            "criteria": "criteria.csv",
            "evidence": "evidence.csv",
            "budget": "budget.csv",
        },
        "criteria": [row["id"] for row in target_criteria],
    }
    append_row(
        path / "review_targets.csv",
        REVIEW_TARGET_FIELDS,
        {
            "schema": "mobius.review_target",
            "review_target_id": review_target_id,
            "objective_id": objective["objective_id"],
            "objective_slug": args.objective_slug,
            "review_mode": args.review_mode,
            "stateless": "true",
            "work_item_id": args.work_item_id or "",
            "route_run_id": route_run.get("id", ""),
            "created_at": now_iso(),
            "target_json": json_dumps(target),
            "target_sha256": sha256_text(json_dumps(target)),
        },
    )
    json_print(result("review-target-create", root, args.objective_slug, objective["objective_id"], gate="review_target_ready", next_required_action="record_review_judgment", updated_files=["review_targets.csv"], review_target_id=review_target_id, review_target=target))


def cmd_review_target_read(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    rows = [row for row in read_rows(path / "review_targets.csv") if not args.review_target_id or row.get("review_target_id") == args.review_target_id]
    if args.review_mode:
        rows = [row for row in rows if row.get("review_mode") == args.review_mode]
    if not rows:
        json_print(result("review-target-read", root, args.objective_slug, objective["objective_id"], ok=False, errors=["review target not found"]), 2)
    row = rows[-1]
    target = parse_json_cell(row.get("target_json"), {})
    view = {
        "schema": "mobius.review_target_view",
        "id": f"{args.objective_slug}:{row['review_target_id']}:{row['review_mode']}",
        "relation": "checkpoint" if row["review_mode"] == "checkpoint_review" else "exit",
        "criteria": target.get("criteria", []),
        "coverage": target.get("coverage", {}),
        "feedback_actions": sorted(FEEDBACK_ACTIONS),
    }
    json_print(result("review-target-read", root, args.objective_slug, objective["objective_id"], gate="read_only", review_target=target, review_target_view=view))


def parse_string_list(raw: str | None) -> list[str]:
    if not raw:
        return []
    parsed = json.loads(raw)
    if not isinstance(parsed, list):
        raise ValueError("expected a JSON array")
    return [str(item) for item in parsed]


def cmd_review_judgment_record(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    targets = [row for row in read_rows(path / "review_targets.csv") if row.get("review_target_id") == args.review_target_id]
    if not targets:
        json_print(result("review-judgment-record", root, args.objective_slug, objective["objective_id"], ok=False, errors=["review target not found"]), 2)
    if args.verdict not in JUDGMENT_VERDICTS:
        json_print(result("review-judgment-record", root, args.objective_slug, objective["objective_id"], ok=False, errors=[f"invalid verdict: {args.verdict}"]), 2)
    if args.feedback_action not in FEEDBACK_ACTIONS:
        json_print(result("review-judgment-record", root, args.objective_slug, objective["objective_id"], ok=False, errors=[f"invalid feedback action: {args.feedback_action}"]), 2)
    prior = [row for row in read_rows(path / "review_judgments.csv") if row.get("review_target_id") == args.review_target_id]
    if prior:
        json_print(result("review-judgment-record", root, args.objective_slug, objective["objective_id"], ok=False, errors=["review target already has a judgment"]), 2)
    target_row = targets[-1]
    target = parse_json_cell(target_row.get("target_json"), {})
    checked = parse_string_list(args.checked_criteria_json)
    blocking = parse_string_list(args.blocking_findings_json)
    revisions = parse_string_list(args.required_revisions_json)
    missing = [item for item in target.get("criteria", []) if item not in checked]
    pass_errors: list[str] = []
    if args.verdict == "pass":
        if missing:
            pass_errors.append(f"pass judgment omitted criteria: {', '.join(missing)}")
        if blocking:
            pass_errors.append("pass judgment cannot include blocking findings")
        if revisions:
            pass_errors.append("pass judgment cannot include required revisions")
        if args.feedback_action != "none":
            pass_errors.append("pass judgment requires feedback_action=none")
    if pass_errors:
        json_print(result("review-judgment-record", root, args.objective_slug, objective["objective_id"], ok=False, errors=pass_errors), 2)
    judgment_id = next_id(read_rows(path / "review_judgments.csv"), "review_judgment")
    run_id = next_id(read_rows(path / "review_runs.csv"), "review_run")
    returned_at = now_iso()
    result_json = {
        "verdict": args.verdict,
        "checked_criterion_ids": checked,
        "unchecked_criterion_ids": missing,
        "blocking_findings": blocking,
        "required_revisions": revisions,
        "feedback_action": args.feedback_action,
    }
    review_failure_kind = ""
    review_retryable = "false"
    if args.verdict != "pass":
        if args.feedback_action == "retry_review":
            review_failure_kind = "review_infrastructure"
            review_retryable = "true"
        else:
            review_failure_kind = "review_feedback"
    append_row(
        path / "review_runs.csv",
        REVIEW_RUN_FIELDS,
        {
            "schema": "mobius.review_run",
            "review_run_id": run_id,
            "review_target_id": args.review_target_id,
            "review_mode": target_row["review_mode"],
            "status": "recorded",
            "started_at": args.started_at or returned_at,
            "finished_at": returned_at,
            "reviewer_summary_ref": args.raw_ref or "",
            "failure_kind": review_failure_kind,
            "retryable": review_retryable,
        },
    )
    append_row(
        path / "review_judgments.csv",
        REVIEW_JUDGMENT_FIELDS,
        {
            "schema": "mobius.review_judgment",
            "review_judgment_id": judgment_id,
            "objective_id": objective["objective_id"],
            "review_target_id": args.review_target_id,
            "review_mode": target_row["review_mode"],
            "level": args.level,
            "stateless": "true",
            "reviewers_json": json_dumps([args.reviewer]),
            "result_json": json_dumps(result_json),
            "feedback_action": args.feedback_action,
            "raw_ref": args.raw_ref or "",
            "raw_hash_tail": sha256_text(args.raw_result or "")[-16:] if args.raw_result else "",
            "returned_at": returned_at,
        },
    )
    append_budget(path, objective["objective_id"], "review_judgment_recorded", "harness_internal", False, source="mobius_review", review_target_id=args.review_target_id, review_run_id=run_id, finished_at=returned_at)
    updated = ["review_runs.csv", "review_judgments.csv", "budget.csv"]
    if args.verdict == "pass":
        criteria_rows = read_rows(path / "criteria.csv")
        for row in criteria_rows:
            if row.get("id") in checked:
                row["status"] = "pass"
                row["review_judgment_id"] = judgment_id
                row["verified_at"] = returned_at
                row["evidence_ids_json"] = json_dumps(target.get("coverage", {}).get(row["id"], []))
        write_rows(path / "criteria.csv", CRITERION_FIELDS, criteria_rows)
        updated.append("criteria.csv")
        route_run_id = target_row.get("route_run_id", "")
        if route_run_id:
            route_runs = read_rows(path / "route_runs.csv")
            for row in route_runs:
                if row.get("id") == route_run_id:
                    row["status"] = "passed"
                    row["finished_at"] = returned_at
                    row["review_judgment_id"] = judgment_id
            write_rows(path / "route_runs.csv", ROUTE_RUN_FIELDS, route_runs)
            updated.append("route_runs.csv")
        if target_row["review_mode"] == "exit_review" and not unverified_criteria(path):
            objective["status"] = "accepted"
            objective["updated_at"] = returned_at
            write_rows(path / "objective.csv", OBJECTIVE_FIELDS, [objective])
            write_verdict(path, objective["objective_id"], "accepted")
            updated.extend(["objective.csv", "verdict.csv"])
    elif args.verdict == "blocked" and args.feedback_action == "contract_change_required":
        objective["status"] = "blocked"
        objective["updated_at"] = returned_at
        write_rows(path / "objective.csv", OBJECTIVE_FIELDS, [objective])
        write_verdict(path, objective["objective_id"], "blocked")
        updated.extend(["objective.csv", "verdict.csv"])
    json_print(result("review-judgment-record", root, args.objective_slug, objective["objective_id"], gate="review_recorded", updated_files=updated, review_judgment_id=judgment_id, persisted=True))


def cmd_verdict(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    write_verdict(path, objective["objective_id"], terminal_verdict(path) or "pending")
    json_print(result("verdict", root, args.objective_slug, objective["objective_id"], gate=terminal_verdict(path) or "pending", verdict=single_row(path / "verdict.csv")))


def cmd_codex_session_import(args: argparse.Namespace) -> None:
    root = project_root(args)
    path, objective = require_objective(root, args.session_id, args.objective_slug)
    source = Path(args.session_jsonl).expanduser()
    if not source.is_file():
        json_print(result("codex-session-import", root, args.objective_slug, objective["objective_id"], ok=False, errors=["session JSONL file not found"]), 2)
    imported = 0
    with source.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            started_at = event.get("timestamp") or event.get("started_at") or event.get("time")
            finished_at = event.get("finished_at") or event.get("completed_at")
            if not started_at:
                continue
            event_kind = str(event.get("type") or event.get("event_kind") or "codex_event")
            duration_ms = safe_int(event.get("duration_ms"))
            precision = "timestamp_only"
            if duration_ms > 0:
                precision = "duration_ms"
            elif finished_at:
                started_ms = parse_iso_ms(str(started_at))
                finished_ms = parse_iso_ms(str(finished_at))
                if started_ms is not None and finished_ms is not None and finished_ms >= started_ms:
                    duration_ms = finished_ms - started_ms
                    precision = "paired_timestamps_ms"
                else:
                    precision = "paired_timestamps_unparsed"
            timing = classify_imported_timing(event, event_kind, duration_ms)
            budget_id = append_budget(
                path,
                objective["objective_id"],
                event_kind,
                timing["clock_domain"],
                timing["metered"],
                source=f"codex_session_jsonl:{source.name}:{line_number}:precision={precision}",
                started_at=str(started_at),
                finished_at=str(finished_at or ""),
                duration_ms=duration_ms,
                consumed_ms=timing["consumed_ms"],
                failure_kind=timing["failure_kind"],
                work_item_id=str(event.get("work_item_id") or ""),
                criterion_id=str(event.get("criterion_id") or ""),
                route_id=str(event.get("route_id") or ""),
                route_run_id=str(event.get("route_run_id") or ""),
                review_target_id=str(event.get("review_target_id") or ""),
                review_run_id=str(event.get("review_run_id") or ""),
                tool_call_id=str(event.get("tool_call_id") or ""),
            )
            sync_route_run_timebox(path, str(event.get("route_run_id") or ""), budget_id, str(finished_at or ""))
            imported += 1
    json_print(result("codex-session-import", root, args.objective_slug, objective["objective_id"], gate="imported", updated_files=["budget.csv"], imported_events=imported))


def protected_path_from_text(text: str, root: Path) -> str:
    if ".mobius" in text:
        for filename in PROTECTED_LEDGER_FILENAMES:
            if filename in text:
                return filename
    for token in re.split(r"\s+", text):
        candidate = token.strip("'\"")
        if not candidate:
            continue
        path = Path(candidate)
        if not path.is_absolute():
            path = root / path
        parts = path.parts
        if ".mobius" in parts and path.name in PROTECTED_LEDGER_FILENAMES:
            return str(path)
    return ""


WRITE_COMMAND_RE = re.compile(
    r"(^|\s)(apply_patch|ed|ex|perl\s+-pi|sed\s+-i|tee|truncate|rm|mv|cp|touch|chmod|chown|python(?:3)?\s+-c\b.*\b(open|write_text|unlink|rename|replace)\b|>\s*|>>\s*)",
    re.DOTALL,
)


def payload_requests_write(payload_text: str) -> bool:
    try:
        payload = json.loads(payload_text or "{}")
    except json.JSONDecodeError:
        payload = {}
    tool_name = str(payload.get("tool_name") or payload.get("tool") or payload.get("name") or payload.get("command_name") or "")
    if re.search(r"(apply_patch|edit|write|delete|remove|move|rename)", tool_name, re.IGNORECASE):
        return True
    args = payload.get("args")
    nested_command = args.get("command", "") if isinstance(args, dict) else ""
    command = str(payload.get("command") or payload.get("cmd") or nested_command)
    text = command or payload_text
    return bool(WRITE_COMMAND_RE.search(text))


def cmd_hook(args: argparse.Namespace) -> None:
    root = project_root(args)
    payload_text = sys.stdin.read()
    if args.hook_event == "pre-tool-use":
        protected = protected_path_from_text(payload_text, root)
        if protected and payload_requests_write(payload_text):
            print(f"mobius:protected-ledger-write-blocked: {protected}", file=sys.stderr)
            raise SystemExit(2)
    if args.hook_event == "stop":
        try:
            payload = json.loads(payload_text or "{}")
        except json.JSONDecodeError:
            payload = {}
        text = json_dumps(payload)
        objective_slug = payload.get("objective_slug") or payload.get("objectiveSlug")
        session_id = payload.get("session_id") or payload.get("sessionId")
        if objective_slug and session_id and re.search(r"\b(done|complete|accepted|finished)\b", text, re.IGNORECASE):
            path = objective_dir(root, str(session_id), str(objective_slug))
            if terminal_verdict(path) != "accepted":
                print("mobius:completion-blocked: no accepted verdict.csv found for claimed objective", file=sys.stderr)
                raise SystemExit(2)
    raise SystemExit(0)


def cmd_hook_health(args: argparse.Namespace) -> None:
    root = project_root(args)
    plugin_root = Path(__file__).resolve().parents[1]
    hooks = plugin_root / "hooks" / "hooks.json"
    launcher = plugin_root / "scripts" / "mobius_hook_launcher.sh"
    ok = hooks.is_file() and launcher.is_file()
    json_print(
        {
            "schema": "mobius.hook_health",
            "ok": ok,
            "command": "hook-health",
            "project_root": str(root),
            "plugin_root": str(plugin_root),
            "events": ["PreToolUse", "Stop"] if hooks.is_file() else [],
            "protected_ledgers": list(PROTECTED_LEDGER_FILENAMES),
            "errors": [] if ok else ["hook files missing"],
        },
        0 if ok else 2,
    )


def mcp_self_check(plugin_root: Path) -> dict[str, Any]:
    launcher = plugin_root / "scripts" / "mobius_review_mcp_server.sh"
    uv = os.environ.get("MOBIUS_REVIEW_UV") or shutil.which("uv")
    if not launcher.is_file():
        return {"status": "missing_launcher", "ready": False}
    if not uv:
        return {"status": "missing_uv", "ready": False}
    env = os.environ.copy()
    env["MOBIUS_REVIEW_UV"] = uv
    with tempfile.TemporaryDirectory(prefix="mobius-review-doctor-") as tmp:
        env["PLUGIN_DATA"] = tmp
        completed = subprocess.run(["/bin/bash", str(launcher), "--self-check"], text=True, capture_output=True, env=env, check=False, timeout=60)
    return {
        "status": "ready" if completed.returncode == 0 else "error",
        "ready": completed.returncode == 0,
        "stdout_tail": completed.stdout[-200:],
        "stderr_tail": completed.stderr[-500:],
    }


def cmd_doctor(args: argparse.Namespace) -> None:
    root = project_root(args)
    plugin_root = Path(__file__).resolve().parents[1]
    uv = os.environ.get("MOBIUS_REVIEW_UV") or shutil.which("uv")
    mcp = mcp_self_check(plugin_root)
    payload = {
        "schema": "mobius.doctor",
        "command": "doctor",
        "plugin_root": str(plugin_root),
        "project_root": str(root),
        "version": MOBIUS_VERSION,
        "hooks": {"status": "active" if (plugin_root / "hooks" / "hooks.json").is_file() else "inactive"},
        "mcp": {"server": "mobius-review", "uv_required": True, "uv": uv or "", "start_ready": bool(mcp["ready"]), "self_check": mcp},
        "errors": [] if mcp["ready"] else ["Mobius Review MCP self-check failed"],
    }
    json_print(payload, 0 if mcp["ready"] else 2)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Mobius v0.5 local ledger engine")
    parser.add_argument("--project-root", default=os.getcwd())
    sub = parser.add_subparsers(dest="command", required=True)

    objective = sub.add_parser("objective-start")
    objective.add_argument("--session-id", required=True)
    objective.add_argument("--slug", required=True)
    objective.add_argument("--title", required=True)
    objective.add_argument("--user-request", required=True)
    objective.add_argument("--non-claim", action="append")
    objective.set_defaults(func=cmd_objective_start)

    work_item = sub.add_parser("contract-add-work-item")
    work_item.add_argument("--session-id", required=True)
    work_item.add_argument("--objective-slug", required=True)
    work_item.add_argument("--id", required=True)
    work_item.add_argument("--title", required=True)
    work_item.add_argument("--description", required=True)
    work_item.add_argument("--depends-on-json", default="[]")
    work_item.add_argument("--scope-json", required=True)
    work_item.add_argument("--work-json", required=True)
    work_item.add_argument("--gate-json", required=True)
    work_item.add_argument("--recovery-json", required=True)
    work_item.add_argument("--timebox-json", required=True)
    work_item.add_argument("--criteria-json", required=True)
    work_item.add_argument("--revision", default="1")
    work_item.add_argument("--optional", action="store_true")
    work_item.set_defaults(func=cmd_contract_add_work_item)

    validate = sub.add_parser("contract-validate")
    validate.add_argument("--session-id", required=True)
    validate.add_argument("--objective-slug", required=True)
    validate.set_defaults(func=cmd_contract_validate)

    lock = sub.add_parser("contract-lock")
    lock.add_argument("--session-id", required=True)
    lock.add_argument("--objective-slug", required=True)
    lock.add_argument("--locked-by", default="main_agent")
    lock.set_defaults(func=cmd_contract_lock)

    route = sub.add_parser("route-run-start")
    route.add_argument("--session-id", required=True)
    route.add_argument("--objective-slug", required=True)
    route.add_argument("--work-item-id", required=True)
    route.add_argument("--route-id")
    route.add_argument("--rationale")
    route.add_argument("--timebox-ms", type=int)
    route.set_defaults(func=cmd_route_run_start)

    evidence = sub.add_parser("evidence-add")
    evidence.add_argument("--session-id", required=True)
    evidence.add_argument("--objective-slug", required=True)
    evidence.add_argument("--type", required=True)
    evidence.add_argument("--summary", required=True)
    evidence.add_argument("--supports", action="append")
    evidence.add_argument("--supports-json")
    evidence.add_argument("--artifact")
    evidence.add_argument("--artifact-json")
    evidence.add_argument("--created-by", default="main_agent")
    evidence.set_defaults(func=cmd_evidence_add)

    evidence_list = sub.add_parser("evidence-list")
    evidence_list.add_argument("--session-id", required=True)
    evidence_list.add_argument("--objective-slug", required=True)
    evidence_list.add_argument("--criterion-id")
    evidence_list.set_defaults(func=cmd_evidence_list)

    budget = sub.add_parser("budget-add")
    budget.add_argument("--session-id", required=True)
    budget.add_argument("--objective-slug", required=True)
    budget.add_argument("--event-kind", required=True)
    budget.add_argument("--clock-domain", required=True)
    budget.add_argument("--metered", default="false")
    budget.add_argument("--source", required=True)
    budget.add_argument("--started-at")
    budget.add_argument("--finished-at")
    budget.add_argument("--duration-ms", type=int, default=0)
    budget.add_argument("--consumed-ms", type=int)
    budget.add_argument("--failure-kind")
    budget.add_argument("--tool-call-id")
    budget.add_argument("--work-item-id")
    budget.add_argument("--criterion-id")
    budget.add_argument("--route-id")
    budget.add_argument("--route-run-id")
    budget.add_argument("--review-target-id")
    budget.add_argument("--review-run-id")
    budget.set_defaults(func=cmd_budget_add)

    import_session = sub.add_parser("codex-session-import")
    import_session.add_argument("--session-id", required=True)
    import_session.add_argument("--objective-slug", required=True)
    import_session.add_argument("--session-jsonl", required=True)
    import_session.set_defaults(func=cmd_codex_session_import)

    target = sub.add_parser("review-target-create")
    target.add_argument("--session-id", required=True)
    target.add_argument("--objective-slug", required=True)
    target.add_argument("--review-mode", choices=sorted(REVIEW_TYPES), required=True)
    target.add_argument("--work-item-id")
    target.set_defaults(func=cmd_review_target_create)

    target_read = sub.add_parser("review-target-read")
    target_read.add_argument("--session-id", required=True)
    target_read.add_argument("--objective-slug", required=True)
    target_read.add_argument("--review-mode", choices=sorted(REVIEW_TYPES))
    target_read.add_argument("--review-target-id")
    target_read.set_defaults(func=cmd_review_target_read)

    judgment = sub.add_parser("review-judgment-record")
    judgment.add_argument("--session-id", required=True)
    judgment.add_argument("--objective-slug", required=True)
    judgment.add_argument("--review-target-id", required=True)
    judgment.add_argument("--reviewer", default="codex-subagent")
    judgment.add_argument("--verdict", choices=sorted(JUDGMENT_VERDICTS), required=True)
    judgment.add_argument("--checked-criteria-json", required=True)
    judgment.add_argument("--blocking-findings-json", default="[]")
    judgment.add_argument("--required-revisions-json", default="[]")
    judgment.add_argument("--feedback-action", choices=sorted(FEEDBACK_ACTIONS), default="none")
    judgment.add_argument("--level", default="1")
    judgment.add_argument("--raw-ref")
    judgment.add_argument("--raw-result")
    judgment.add_argument("--started-at")
    judgment.set_defaults(func=cmd_review_judgment_record)

    for name, func in {
        "explain": cmd_explain,
        "loop-status": cmd_loop_status,
        "ledger-audit": cmd_ledger_audit,
        "continue": cmd_continue,
        "verdict": cmd_verdict,
    }.items():
        item = sub.add_parser(name)
        item.add_argument("--session-id", required=True)
        item.add_argument("--objective-slug", required=True)
        item.set_defaults(func=func)

    hook = sub.add_parser("hook")
    hook.add_argument("hook_event", choices=["pre-tool-use", "stop"])
    hook.set_defaults(func=cmd_hook)

    hook_health = sub.add_parser("hook-health")
    hook_health.set_defaults(func=cmd_hook_health)

    doctor = sub.add_parser("doctor")
    doctor.set_defaults(func=cmd_doctor)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    args.func(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
