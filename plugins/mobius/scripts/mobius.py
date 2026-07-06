#!/usr/bin/env python3
"""Mobius local CSV ledger utilities."""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import os
import re
import shlex
import shutil
import sys
import tempfile
import tomllib
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MOBIUS_VERSION = "0.1.0"
ACCEPTANCE_RULE = "accepted iff every required plan and acceptance row is locked, and every required acceptance item is pass and backed by a stateless non-degraded MobiusCV MCP exit_review"

RUN_FIELDS = ["schema", "run_id", "codex_session_id", "project_root", "created_at", "mobius_version", "codex_json", "goals_json"]
GOAL_FIELDS = [
    "schema",
    "goal_id",
    "run_id",
    "goal_slug",
    "status",
    "created_at",
    "updated_at",
    "contract_path",
    "contract_sha256_tail",
]
LOCK_FIELDS = ["locked", "locked_at", "locked_by", "supersedes_id", "change_reason", "lock_hash"]
PLAN_STRUCTURAL_FIELDS = [
    "schema",
    "goal_id",
    "revision",
    "id",
    "title",
    "description",
    "required",
    "depends_on_json",
    "scope_json",
    "work_json",
    "gate_json",
    "recovery_json",
    "budget_json",
    "acceptance_ids_json",
]
ACCEPTANCE_STRUCTURAL_FIELDS = [
    "schema",
    "goal_id",
    "id",
    "plan_item_id",
    "requirement",
    "observable_outcome",
    "evidence_required_json",
    "verifier_json",
    "review_focus_json",
    "required",
]
PLAN_FIELDS = [
    "schema",
    "goal_id",
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
    "budget_json",
    "acceptance_ids_json",
    *LOCK_FIELDS,
]
ACCEPTANCE_FIELDS = [
    "schema",
    "goal_id",
    "id",
    "plan_item_id",
    "requirement",
    "observable_outcome",
    "evidence_required_json",
    "verifier_json",
    "review_focus_json",
    "required",
    "status",
    "evidence_ids_json",
    "cv_id",
    "verified_by",
    "verified_at",
    *LOCK_FIELDS,
]
EVIDENCE_FIELDS = ["schema", "id", "goal_id", "type", "summary", "supports_json", "artifact_json", "created_by", "created_at"]
PACKET_FIELDS = [
    "schema",
    "packet_id",
    "goal_id",
    "goal_slug",
    "review_mode",
    "stateless",
    "scope",
    "created_at",
    "packet_json",
    "packet_sha256",
]
REVIEW_ATTEMPT_FIELDS = [
    "schema",
    "attempt_id",
    "packet_id",
    "review_mode",
    "status",
    "started_at",
    "finished_at",
    "reviewer_summary_ref",
]
CV_FIELDS = [
    "schema",
    "cv_id",
    "goal_id",
    "packet_id",
    "review_mode",
    "level",
    "stateless",
    "reviewers_json",
    "comparison_json",
    "input_refs_json",
    "result_json",
    "raw_ref",
    "raw_hash_tail",
    "returned_at",
]
VERDICT_FIELDS = [
    "schema",
    "goal_id",
    "overall",
    "adjudicated_by",
    "adjudicated_at",
    "rule",
    "derived_from_json",
    "unverified_plan_item_ids_json",
    "unverified_acceptance_ids_json",
    "blocked_acceptance_ids_json",
]
LOOP_FIELDS = [
    "schema",
    "goal_id",
    "plan_item_id",
    "status",
    "attempt",
    "last_packet_id",
    "last_cv_id",
    "blocking_findings_json",
    "updated_at",
]
LOOP_STATUSES = {"pending", "running", "passed", "blocked"}
PUBLIC_LOOP_START_STATUSES = {"running"}
PLAN_STATUSES = {"pending", "superseded"}
ACCEPTANCE_STATUSES = {"unknown", "pass", "blocked", "superseded"}
GOAL_STATUSES = {"planning", "active", "accepted", "blocked"}
REVIEW_ATTEMPT_STATUSES = {"started", "recorded", "interrupted", "failed"}
TERMINAL_VERDICTS = {"accepted", "blocked"}
TERMINAL_NEXT_REQUIRED_ACTION = "goal_terminal_start_new_goal_or_explicit_reopen"
REVIEW_POLICY_SCHEMA = "mobius.review_gate_policy"
REVIEW_POLICY_NAMES = {"delta_light", "delta_kimi", "exit_strict"}
PLAN_SCHEMA = "mobius.plan"
ACCEPTANCE_SCHEMA = "mobius.acceptance"
VAGUE_ACCEPTANCE_TERMS = {"sufficient", "robust", "appropriate", "complete"}
EVIDENCE_TYPES = {"change_set_scope", "file_ref", "command_result", "test_result", "human_assertion"}
REVIEW_VERIFIER_TYPES = {"mobiuscv_delta", "mobiuscv_exit"}
VERIFIER_TYPES = EVIDENCE_TYPES | REVIEW_VERIFIER_TYPES
STRUCTURED_PROOF_TYPES = EVIDENCE_TYPES - {"human_assertion"}
PATH_PROOF_TYPES = {"file_ref"}
CHANGE_SET_SCOPE_COVERAGE = {"tracked", "staged", "untracked", "intent_to_add"}
LOOP_STOP_REASONS = {"review_blocked", "repair_budget_exhausted", "contract_change_required", "no_runnable_action"}

STATE_TRANSITIONS = {
    "goal": {
        "planning": {"active"},
        "active": {"accepted", "blocked"},
        "accepted": set(),
        "blocked": set(),
    },
    "loop": {
        "pending": {"running"},
        "running": {"passed", "blocked"},
        "blocked": {"running"},
        "passed": {"running"},
    },
    "acceptance": {
        "unknown": {"pass", "blocked", "superseded"},
        "pass": {"superseded"},
        "blocked": {"superseded"},
        "superseded": set(),
    },
    "review_attempt": {
        "started": {"recorded", "interrupted", "failed"},
        "recorded": set(),
        "interrupted": set(),
        "failed": set(),
    },
    "verdict": {
        "pending": {"accepted", "blocked"},
        "accepted": set(),
        "blocked": set(),
    },
}


def validate_state_value(kind: str, status: str) -> None:
    if status not in STATE_TRANSITIONS[kind]:
        raise MobiusError(f"invalid {kind} status: {status}")


def validate_state_transition(kind: str, current: str, next_status: str) -> None:
    validate_state_value(kind, next_status)
    if not current or current == next_status:
        return
    validate_state_value(kind, current)
    if next_status not in STATE_TRANSITIONS[kind].get(current, set()):
        raise MobiusError(f"invalid {kind} transition: {current} -> {next_status}")


class MobiusError(Exception):
    """Deterministic Mobius state transition failure."""


def now_iso() -> str:
    return datetime.now(timezone.utc).astimezone().isoformat(timespec="seconds")


def as_json_cell(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def json_print(value: dict[str, Any]) -> None:
    print(as_json_cell(value))


def goal_context(goal_dir: Path | None) -> tuple[str, str]:
    if goal_dir is None:
        return "", ""
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    return goal.get("goal_id", ""), goal_dir.name


def command_result(
    command: str,
    *,
    ok: bool = True,
    goal_dir: Path | None = None,
    updated_files: list[str] | None = None,
    gate: str = "pending",
    next_required_action: str = "",
    errors: list[str] | None = None,
    data: dict[str, Any] | None = None,
) -> dict[str, Any]:
    goal_id, goal_slug = goal_context(goal_dir)
    result = {
        "schema": "mobius.command_result",
        "ok": ok,
        "command": command,
        "goal_id": goal_id,
        "goal_slug": goal_slug,
        "updated_files": updated_files or [],
        "gate": gate,
        "next_required_action": next_required_action,
        "errors": errors or [],
    }
    if data:
        result.update(data)
    return result


def loop_command_result(
    command: str,
    root: Path,
    session_id: str,
    goal_slug: str,
    *,
    ok: bool = True,
    updated_files: list[str] | None = None,
    errors: list[str] | None = None,
    data: dict[str, Any] | None = None,
) -> dict[str, Any]:
    goal_dir = load_goal_dir(root, session_id, goal_slug)
    audit = ledger_audit_data(root, session_id, goal_slug)
    loop = audit["loop"]
    payload = {
        **(data or {}),
        "audit": audit,
        "loop": loop,
        "next_plan_item_id": audit.get("next_plan_item_id", ""),
        "packet_id": loop.get("packet_id", ""),
        "review_mode": loop.get("review_mode", ""),
    }
    return command_result(
        command,
        ok=ok,
        goal_dir=goal_dir,
        updated_files=updated_files,
        gate=audit["terminal_verdict"] or audit["loop_gate"],
        next_required_action=loop["next_required_action"],
        errors=errors,
        data=payload,
    )


def from_json_cell(value: str, default: Any) -> Any:
    if value is None or value == "":
        return default
    return json.loads(value)


def as_bool_cell(value: bool) -> str:
    return "true" if value else "false"


def from_bool_cell(value: str, default: bool = False) -> bool:
    if value == "":
        return default
    return value.lower() == "true"


CsvWrite = tuple[Path, list[str], list[dict[str, Any]]]


def csv_rows_text(fieldnames: list[str], rows: list[dict[str, Any]]) -> str:
    buffer = io.StringIO(newline="")
    writer = csv.DictWriter(buffer, fieldnames=fieldnames, extrasaction="ignore")
    writer.writeheader()
    for row in rows:
        writer.writerow({field: row.get(field, "") for field in fieldnames})
    return buffer.getvalue()


def csv_rows_sha256(fieldnames: list[str], rows: list[dict[str, Any]]) -> str:
    encoded = csv_rows_text(fieldnames, rows).encode("utf-8")
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def write_text_temp(path: Path, text: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    handle = tempfile.NamedTemporaryFile(
        "w",
        encoding="utf-8",
        newline="",
        dir=path.parent,
        prefix=f".{path.name}.",
        suffix=".tmp",
        delete=False,
    )
    temp_path = Path(handle.name)
    try:
        with handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
    except BaseException:
        temp_path.unlink(missing_ok=True)
        raise
    return temp_path


def write_csv_files_atomically(writes: list[CsvWrite]) -> None:
    if not writes:
        return
    by_path: dict[Path, tuple[list[str], list[dict[str, Any]]]] = {}
    for path, fieldnames, rows in writes:
        by_path[path] = (fieldnames, rows)

    temp_paths: dict[Path, Path] = {}
    backup_paths: dict[Path, Path] = {}
    original_exists: dict[Path, bool] = {}
    replaced: list[Path] = []
    try:
        for path, (fieldnames, rows) in by_path.items():
            temp_paths[path] = write_text_temp(path, csv_rows_text(fieldnames, rows))
            original_exists[path] = path.exists()

        fail_before = os.environ.get("MOBIUS_TEST_FAIL_BEFORE_CSV_COMMIT", "")
        if fail_before and (fail_before == "1" or fail_before in {path.name for path in by_path}):
            raise MobiusError("injected failure before CSV commit")

        for path in by_path:
            if not path.exists():
                continue
            backup = tempfile.NamedTemporaryFile(
                "wb",
                dir=path.parent,
                prefix=f".{path.name}.backup.",
                suffix=".tmp",
                delete=False,
            )
            backup_path = Path(backup.name)
            backup.close()
            shutil.copy2(path, backup_path)
            backup_paths[path] = backup_path

        fail_after_backup = os.environ.get("MOBIUS_TEST_FAIL_AFTER_CSV_BACKUP", "")
        if fail_after_backup and (fail_after_backup == "1" or fail_after_backup in {path.name for path in by_path}):
            raise MobiusError("injected failure after CSV backup")

        for path, temp_path in list(temp_paths.items()):
            os.replace(temp_path, path)
            temp_paths.pop(path, None)
            replaced.append(path)
            fail_after_replace = os.environ.get("MOBIUS_TEST_FAIL_AFTER_CSV_REPLACE", "")
            if fail_after_replace and (fail_after_replace == "1" or fail_after_replace == path.name):
                raise MobiusError("injected failure after CSV replace")
    except BaseException:
        for path in reversed(replaced):
            backup_path = backup_paths.get(path)
            if backup_path and backup_path.exists():
                shutil.copy2(backup_path, path)
            elif not original_exists.get(path, False):
                path.unlink(missing_ok=True)
        raise
    finally:
        for temp_path in temp_paths.values():
            temp_path.unlink(missing_ok=True)
        for backup_path in backup_paths.values():
            backup_path.unlink(missing_ok=True)


def write_csv_rows(path: Path, fieldnames: list[str], rows: list[dict[str, Any]]) -> None:
    write_csv_files_atomically([(path, fieldnames, rows)])


def read_csv_rows(path: Path) -> list[dict[str, str]]:
    if not path.exists():
        return []
    with path.open("r", encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle))


def read_single_csv(path: Path) -> dict[str, str] | None:
    rows = read_csv_rows(path)
    return rows[0] if rows else None


def write_single_csv(path: Path, fieldnames: list[str], row: dict[str, Any]) -> None:
    write_csv_rows(path, fieldnames, [row])


def append_csv_row(path: Path, fieldnames: list[str], row: dict[str, Any]) -> None:
    rows = read_csv_rows(path)
    rows.append(row)
    write_csv_rows(path, fieldnames, rows)


def ensure_csv_file(path: Path, fieldnames: list[str]) -> None:
    rows = read_csv_rows(path)
    write_csv_rows(path, fieldnames, rows)


def ensure_loop_file(goal_dir: Path) -> None:
    ensure_csv_file(goal_dir / "loop.csv", LOOP_FIELDS)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return "sha256:" + digest.hexdigest()


def sha256_text(value: str) -> str:
    return "sha256:" + hashlib.sha256(value.encode("utf-8")).hexdigest()


def sha256_tail(value: str) -> str:
    digest = value.split(":", 1)[1] if value.startswith("sha256:") else value
    return digest[-7:]


def short_hash_ref(value: str) -> str:
    return "h:" + sha256_tail(value)


def path_is_relative_to(path: Path, base: Path) -> bool:
    try:
        path.relative_to(base)
    except ValueError:
        return False
    return True


def root_relative_path_errors(label: str, paths: list[str]) -> list[str]:
    errors: list[str] = []
    for value in paths:
        path = str(value).strip()
        if not path:
            errors.append(f"{label}: path must not be empty")
            continue
        candidate = Path(path)
        if candidate.is_absolute():
            errors.append(f"{label}: path must be root-relative: {path}")
        if ".." in candidate.parts:
            errors.append(f"{label}: path must not contain '..': {path}")
    return errors


def toml_value(value: Any) -> str:
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ", ".join(toml_value(str(item)) for item in value) + "]"
    raise TypeError(f"unsupported TOML value: {type(value).__name__}")


def goal_contract_text(front: dict[str, Any], user_goal: str, latest_user_request: str) -> str:
    ordered = [
        "schema",
        "goal_id",
        "run_id",
        "goal_slug",
        "title",
        "created_at",
        "locked_at",
        "locked_by",
        "non_goals",
    ]
    front_matter = "\n".join(f"{key} = {toml_value(front.get(key, [] if key == 'non_goals' else ''))}" for key in ordered)
    title = str(front.get("title", "")).strip() or str(front.get("goal_slug", "")).strip() or "Mobius Goal"
    return (
        "+++\n"
        + front_matter
        + "\n+++\n\n"
        + f"# {title}\n\n"
        + "## User Goal\n\n"
        + user_goal.strip()
        + "\n\n"
        + "## Latest User Request\n\n"
        + latest_user_request.strip()
        + "\n"
    )


def parse_goal_contract(path: Path) -> tuple[dict[str, Any], str]:
    text = path.read_text(encoding="utf-8")
    if not text.startswith("+++\n"):
        raise MobiusError("goal.md must start with TOML front matter")
    end = text.find("\n+++\n", 4)
    if end < 0:
        raise MobiusError("goal.md TOML front matter is not closed")
    front = tomllib.loads(text[4:end])
    body = text[end + len("\n+++\n") :]
    if not isinstance(front, dict):
        raise MobiusError("goal.md TOML front matter must be an object")
    return front, body


def write_goal_contract(
    goal_dir: Path,
    *,
    goal_id: str,
    run_id: str,
    goal_slug: str,
    title: str,
    user_goal: str,
    latest_user_request: str,
    non_goals: list[str],
    created_at: str,
    locked_at: str = "",
    locked_by: str = "",
) -> str:
    front = {
        "schema": "mobius.goal_contract",
        "goal_id": goal_id,
        "run_id": run_id,
        "goal_slug": goal_slug,
        "title": title,
        "created_at": created_at,
        "locked_at": locked_at,
        "locked_by": locked_by,
        "non_goals": [str(item) for item in non_goals],
    }
    path = goal_dir / "goal.md"
    path.write_text(goal_contract_text(front, user_goal, latest_user_request), encoding="utf-8")
    return sha256_tail(sha256_file(path))


def lock_goal_contract(goal_dir: Path, locked_at: str, locked_by: str) -> str:
    path = goal_dir / "goal.md"
    front, body = parse_goal_contract(path)
    front["locked_at"] = locked_at
    front["locked_by"] = locked_by
    ordered = [
        "schema",
        "goal_id",
        "run_id",
        "goal_slug",
        "title",
        "created_at",
        "locked_at",
        "locked_by",
        "non_goals",
    ]
    front_matter = "\n".join(f"{key} = {toml_value(front.get(key, [] if key == 'non_goals' else ''))}" for key in ordered)
    path.write_text("+++\n" + front_matter + "\n+++\n" + body, encoding="utf-8")
    return sha256_tail(sha256_file(path))


def artifact_record(root: Path, artifact: str, purpose: str) -> dict[str, Any]:
    artifact_path = Path(artifact).expanduser()
    if not artifact_path.is_absolute():
        artifact_path = (root / artifact_path).resolve()
    else:
        artifact_path = artifact_path.resolve()
    if not artifact_path.exists():
        raise MobiusError(f"artifact does not exist: {artifact_path}")
    if not artifact_path.is_file():
        raise MobiusError(f"artifact is not a file: {artifact_path}")
    if not path_is_relative_to(artifact_path, root):
        raise MobiusError(f"artifact must be inside project root: {artifact_path}")
    return {
        "path": artifact_path.relative_to(root).as_posix(),
        "path_mode": "relative_to_project_root",
        "sha256": sha256_file(artifact_path),
        "size": artifact_path.stat().st_size,
        "purpose": purpose,
    }


def artifact_json_record(root: Path, artifact_json: str, evidence_type: str) -> dict[str, Any]:
    try:
        parsed = json.loads(artifact_json)
    except json.JSONDecodeError as exc:
        raise MobiusError(f"artifact-json is invalid JSON: {exc.msg}") from exc
    if not isinstance(parsed, dict):
        raise MobiusError("artifact-json must be an object")
    effective_type = str(parsed.get("type") or evidence_type)
    if effective_type not in EVIDENCE_TYPES:
        raise MobiusError(f"unsupported artifact-json evidence type: {effective_type}")
    if parsed.get("path") and effective_type not in PATH_PROOF_TYPES:
        raise MobiusError(f"artifact-json path refs are only allowed for evidence types: {sorted_join(PATH_PROOF_TYPES)}")
    if parsed.get("path"):
        path_record = artifact_record(root, str(parsed["path"]), str(parsed.get("purpose", "")))
        merged = {**parsed, **path_record}
        if parsed.get("type"):
            merged["type"] = parsed["type"]
        if parsed.get("name"):
            merged["name"] = parsed["name"]
        if "exit_code" in parsed:
            merged["exit_code"] = parsed["exit_code"]
        merged["hash_tail"] = sha256_tail(str(merged["sha256"]))
        return merged
    if evidence_type in {"command_result", "test_result"} and not str(parsed.get("command", "")).strip():
        raise MobiusError(f"{evidence_type} evidence requires artifact-json.command")
    if evidence_type == "change_set_scope":
        coverage = parsed.get("coverage")
        if not isinstance(coverage, dict):
            raise MobiusError("change_set_scope evidence requires artifact-json.coverage")
        missing = sorted(CHANGE_SET_SCOPE_COVERAGE - set(str(key) for key in coverage))
        if missing:
            raise MobiusError("change_set_scope coverage missing: " + ",".join(missing))
        invalid_coverage = [
            key
            for key in sorted(CHANGE_SET_SCOPE_COVERAGE)
            if coverage.get(key) is not True
        ]
        if invalid_coverage:
            raise MobiusError("change_set_scope coverage flags must be true booleans: " + ",".join(invalid_coverage))
        if "paths" not in parsed or not isinstance(parsed.get("paths"), list) or not parsed.get("paths"):
            raise MobiusError("change_set_scope evidence requires non-empty artifact-json.paths")
        path_errors = root_relative_path_errors("change_set_scope.paths", [str(item) for item in parsed.get("paths", [])])
        if path_errors:
            raise MobiusError("; ".join(path_errors))
        classes = parsed.get("allowed_change_classes")
        if not isinstance(classes, list) or not any(str(item).strip() for item in classes):
            raise MobiusError("change_set_scope evidence requires non-empty artifact-json.allowed_change_classes")
        forbidden = parsed.get("forbidden_paths")
        if not isinstance(forbidden, list):
            raise MobiusError("change_set_scope evidence requires artifact-json.forbidden_paths list")
        forbidden_errors = root_relative_path_errors("change_set_scope.forbidden_paths", [str(item) for item in forbidden])
        if forbidden_errors:
            raise MobiusError("; ".join(forbidden_errors))
        parsed = dict(parsed)
        parsed["hash_tail"] = sha256_tail(sha256_text(json.dumps(parsed, sort_keys=True, ensure_ascii=False, separators=(",", ":"))))
    return parsed


def evidence_refs_for_packet(goal_dir: Path, required_ids: list[str]) -> dict[str, list[str]]:
    required = {str(item) for item in required_ids}
    refs: dict[str, list[str]] = {}
    for row in read_csv_rows(goal_dir / "evidence.csv"):
        try:
            supports = from_json_cell(row.get("supports_json", ""), [])
        except json.JSONDecodeError as exc:
            raise MobiusError(f"evidence.csv:{row.get('id', '')}: invalid supports_json: {exc.msg}") from exc
        if not isinstance(supports, list):
            raise MobiusError(f"evidence.csv:{row.get('id', '')}: supports_json must be a list")
        normalized_supports = [str(item) for item in supports]
        if not required.intersection(normalized_supports):
            continue
        try:
            artifact = from_json_cell(row.get("artifact_json", ""), None)
        except json.JSONDecodeError as exc:
            raise MobiusError(f"evidence.csv:{row.get('id', '')}: invalid artifact_json: {exc.msg}") from exc
        evidence_id = row.get("id", "")
        evidence_type = row.get("type", "")
        label = row.get("summary", "") or evidence_type
        hash_source = csv_rows_sha256(EVIDENCE_FIELDS, [row])
        if isinstance(artifact, dict):
            hash_source = str(artifact.get("sha256") or artifact.get("hash_tail") or hash_source)
            if artifact.get("path"):
                path_errors = root_relative_path_errors(f"evidence.csv:{evidence_id}:path", [str(artifact["path"])])
                if path_errors:
                    raise MobiusError("; ".join(path_errors))
                label = str(artifact["path"])
            elif artifact.get("paths"):
                paths = [str(item) for item in artifact.get("paths", [])]
                path_errors = root_relative_path_errors(f"evidence.csv:{evidence_id}:paths", paths)
                if path_errors:
                    raise MobiusError("; ".join(path_errors))
                label = ",".join(paths)
            elif artifact.get("command"):
                label = str(artifact["command"])
        refs[evidence_id] = [evidence_type, label, short_hash_ref(hash_source)]
    return refs

def structural_hash(row: dict[str, str], fields: list[str]) -> str:
    material = {field: row.get(field, "") for field in fields}
    encoded = json.dumps(material, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def active_required_unlocked_ids(goal_dir: Path) -> list[str]:
    unlocked: list[str] = []
    for filename in ("plan.csv", "acceptance.csv"):
        for row in read_csv_rows(goal_dir / filename):
            row_status = row.get("contract_status") if filename == "plan.csv" else row.get("status")
            if row_status == "superseded":
                continue
            if from_bool_cell(row.get("required", ""), True) and not from_bool_cell(row.get("locked", "")):
                unlocked.append(row.get("id", ""))
    return unlocked


def unlocked_contract_error(goal_dir: Path) -> str:
    return "unlocked contract rows: " + ",".join(active_required_unlocked_ids(goal_dir))


def locked_contract_command_result(command: str, goal_dir: Path) -> dict[str, Any] | None:
    if not active_required_unlocked_ids(goal_dir):
        return None
    return command_result(
        command,
        ok=False,
        goal_dir=goal_dir,
        errors=[unlocked_contract_error(goal_dir)],
        next_required_action="needs_contract_change",
    )


def require_locked_contract(goal_dir: Path) -> None:
    if active_required_unlocked_ids(goal_dir):
        raise MobiusError(unlocked_contract_error(goal_dir))


def slugify(value: str) -> str:
    slug = re.sub(r"[^a-zA-Z0-9]+", "-", value.strip().lower()).strip("-")
    return slug or "goal"


def dated_slug(value: str) -> str:
    slug = slugify(value)
    if re.match(r"^\d{4}-\d{2}-\d{2}-", slug):
        return slug
    return f"{datetime.now().date().isoformat()}-{slug}"


def project_root(args: argparse.Namespace) -> Path:
    return Path(args.project_root).expanduser().resolve()


def run_dir(root: Path, session_id: str) -> Path:
    return root / ".mobius" / "runs" / f"codex-session-{session_id}"


def ensure_private_store(root: Path) -> None:
    mobius_dir = root / ".mobius"
    mobius_dir.mkdir(parents=True, exist_ok=True)
    gitignore = mobius_dir / ".gitignore"
    if not gitignore.exists():
        gitignore.write_text("*\n!.gitignore\n", encoding="utf-8")


def ensure_run(root: Path, session_id: str) -> Path:
    ensure_private_store(root)
    directory = run_dir(root, session_id)
    run_path = directory / "run.csv"
    run = read_single_csv(run_path)
    if run is None:
        run = {
            "schema": "mobius.run",
            "run_id": f"codex-session-{session_id}",
            "codex_session_id": session_id,
            "project_root": str(root),
            "created_at": now_iso(),
            "mobius_version": MOBIUS_VERSION,
            "codex_json": as_json_cell({"cli_version": "unknown", "active_plugins": ["mobius"]}),
            "goals_json": as_json_cell([]),
        }
    write_single_csv(run_path, RUN_FIELDS, run)
    return directory


def load_goal_dir(root: Path, session_id: str, goal_slug: str) -> Path:
    return run_dir(root, session_id) / goal_slug


def cmd_init(args: argparse.Namespace) -> int:
    directory = ensure_run(project_root(args), args.session_id)
    json_print(
        command_result(
            "init-run",
            updated_files=[str(directory / "run.csv")],
            next_required_action="create_or_select_goal",
            data={"run_dir": str(directory), "run_id": f"codex-session-{args.session_id}"},
        )
    )
    return 0


def cmd_goal_start(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_slug = dated_slug(args.slug)
    directory = run_dir(root, args.session_id)
    goal_dir = directory / goal_slug
    if goal_dir.exists():
        terminal_result = terminal_command_result("goal-start", goal_dir)
        if terminal_result is not None:
            json_print(terminal_result)
            return 2
        existing_goal = read_single_csv(goal_dir / "goal.csv") or {}
        try:
            existing_front, _existing_body = parse_goal_contract(goal_dir / "goal.md")
        except (MobiusError, tomllib.TOMLDecodeError, FileNotFoundError):
            existing_front = {}
        contract_locked = bool(str(existing_front.get("locked_at", "")).strip() or str(existing_front.get("locked_by", "")).strip())
        if existing_goal.get("status", "planning") != "planning" or contract_locked:
            json_print(
                command_result(
                    "goal-start",
                    ok=False,
                    goal_dir=goal_dir,
                    errors=["goal-start cannot modify an active or locked goal contract"],
                    next_required_action="select_existing_goal_or_start_new_goal",
                )
            )
            return 2
    directory = ensure_run(root, args.session_id)
    goal_id = "goal_" + re.sub(r"[^0-9a-zA-Z]+", "_", goal_slug).strip("_")
    goal_dir = directory / goal_slug
    goal_dir.mkdir(parents=True, exist_ok=True)

    timestamp = now_iso()
    existing_goal = read_single_csv(goal_dir / "goal.csv") or {}
    run_id = f"codex-session-{args.session_id}"
    contract_tail = write_goal_contract(
        goal_dir,
        goal_id=goal_id,
        run_id=run_id,
        goal_slug=goal_slug,
        title=args.title,
        user_goal=args.user_goal,
        latest_user_request=args.latest_user_request or args.user_goal,
        non_goals=[str(item) for item in (args.non_goal or [])],
        created_at=existing_goal.get("created_at", timestamp),
    )
    goal = {
        "schema": "mobius.goal_state",
        "goal_id": goal_id,
        "run_id": run_id,
        "goal_slug": goal_slug,
        "status": existing_goal.get("status", "planning"),
        "created_at": existing_goal.get("created_at", timestamp),
        "updated_at": timestamp,
        "contract_path": "goal.md",
        "contract_sha256_tail": contract_tail,
    }
    write_single_csv(goal_dir / "goal.csv", GOAL_FIELDS, goal)

    ensure_csv_file(goal_dir / "plan.csv", PLAN_FIELDS)
    ensure_csv_file(goal_dir / "acceptance.csv", ACCEPTANCE_FIELDS)
    for name, fields in (("evidence.csv", EVIDENCE_FIELDS), ("cv.csv", CV_FIELDS), ("loop.csv", LOOP_FIELDS), ("review_attempts.csv", REVIEW_ATTEMPT_FIELDS)):
        ensure_csv_file(goal_dir / name, fields)
    if not (goal_dir / "verdict.csv").exists():
        write_single_csv(
            goal_dir / "verdict.csv",
            VERDICT_FIELDS,
            {
                "schema": "mobius.verdict",
                "goal_id": goal_id,
                "overall": "pending",
                "adjudicated_by": "mobius_gate",
                "adjudicated_at": timestamp,
                "rule": ACCEPTANCE_RULE,
                "derived_from_json": as_json_cell({}),
                "unverified_plan_item_ids_json": as_json_cell([]),
                "unverified_acceptance_ids_json": as_json_cell([]),
                "blocked_acceptance_ids_json": as_json_cell([]),
            },
        )

    run_path = directory / "run.csv"
    run = read_single_csv(run_path) or {}
    goals = from_json_cell(run.get("goals_json", ""), [])
    goals = [item for item in goals if item.get("path") != goal_slug]
    goals.append({"goal_id": goal_id, "slug": goal_slug, "path": goal_slug, "status": goal["status"]})
    run["goals_json"] = as_json_cell(goals)
    write_single_csv(run_path, RUN_FIELDS, run)
    ensure_csv_file(goal_dir / "packets.csv", PACKET_FIELDS)
    updated_files = ["goal.md", "goal.csv", "plan.csv", "acceptance.csv", "evidence.csv", "cv.csv", "loop.csv", "review_attempts.csv", "verdict.csv", "packets.csv"]
    errors = validate_contract_dir(goal_dir, require_complete=False)
    if errors:
        json_print(command_contract_error("goal-start", goal_dir, errors, updated_files=updated_files, data={"goal_dir": str(goal_dir), "run_dir": str(directory)}))
        return 2
    json_print(
        command_result(
            "goal-start",
            goal_dir=goal_dir,
            updated_files=updated_files,
            next_required_action="add_stage_contracts",
            data={"goal_dir": str(goal_dir), "run_dir": str(directory)},
        )
    )
    return 0


def next_id(path: Path, prefix: str) -> str:
    return f"{prefix}{len(read_csv_rows(path)) + 1}"


def parse_required_json_cell(command: str, goal_dir: Path, field: str, value: str, expected_type: type | tuple[type, ...]) -> tuple[Any, int | None]:
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError as exc:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=[f"{field}: invalid JSON: {exc.msg}"]))
        return None, 2
    if not isinstance(parsed, expected_type):
        expected = (
            " or ".join(item.__name__ for item in expected_type)
            if isinstance(expected_type, tuple)
            else expected_type.__name__
        )
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=[f"{field}: expected {expected}"]))
        return None, 2
    return parsed, None


def local_contract_default_cells(plan_id: str) -> dict[str, str]:
    defaults = {
        "depends-on-json": [],
        "scope-json": {
            "allowed_paths": ["**"],
            "forbidden_paths": [".mobius/**"],
            "non_goals": [],
            "invariants": [],
            "side_effect_level": "local",
        },
        "gate-json": {
            "entry": ["contract locked"],
            "exit": ["linked acceptance proof obligations satisfied"],
            "verifiers": ["command_result", "mobiuscv_delta"],
            "review_focus": [f"{plan_id} scope and proof obligations"],
        },
        "recovery-json": {
            "rollback_boundary": "revert selected stage changes",
            "restart_rule": "restart selected stage from pending",
            "escalation_rule": "surface blocker to user",
        },
        "budget-json": {
            "retry_limit": 2,
            "max_stage_attempts": 3,
            "stop_condition": "recorded review blocks or passes",
        },
    }
    return {key: json.dumps(value, separators=(",", ":")) for key, value in defaults.items()}


def contract_stage_json_inputs(command: str, args: argparse.Namespace, goal_dir: Path, plan_id: str) -> tuple[dict[str, str], int | None]:
    fields = ["depends-on-json", "scope-json", "work-json", "gate-json", "recovery-json", "budget-json"]
    raw = {
        "depends-on-json": args.depends_on_json,
        "scope-json": args.scope_json,
        "work-json": args.work_json,
        "gate-json": args.gate_json,
        "recovery-json": args.recovery_json,
        "budget-json": args.budget_json,
    }
    if args.contract_defaults == "local":
        defaults = local_contract_default_cells(plan_id)
        for field in fields:
            if raw[field] is None or not str(raw[field]).strip():
                raw[field] = defaults.get(field)
    missing = [field for field in fields if raw[field] is None or not str(raw[field]).strip()]
    if missing:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=["missing required JSON arguments: " + ",".join(missing)]))
        return {}, 2
    return {field: str(raw[field]) for field in fields}, None


def normalize_acceptance_contracts(
    command: str,
    goal_dir: Path,
    goal_id: str,
    plan_item_id: str,
    acceptance_json: str,
    *,
    allow_supersession: bool = False,
    default_change_reason: str = "",
    required_supersedes_ids: set[str] | None = None,
) -> tuple[list[str], list[dict[str, Any]], int | None]:
    parsed, failed = parse_required_json_cell(command, goal_dir, "acceptance-json", acceptance_json, list)
    if failed is not None:
        return [], [], failed
    if not parsed:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=["acceptance-json: empty acceptance arrays are not allowed"]))
        return [], [], 2
    acceptance_ids: list[str] = []
    rows: list[dict[str, Any]] = []
    for index, item in enumerate(parsed, start=1):
        if not isinstance(item, dict):
            json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=[f"acceptance-json[{index}]: expected object"]))
            return [], [], 2
        acceptance_id = str(item.get("id", "")).strip()
        if not acceptance_id:
            json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=[f"acceptance-json[{index}]: id is required"]))
            return [], [], 2
        if acceptance_id in acceptance_ids:
            json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=[f"duplicate acceptance id in payload: {acceptance_id}"]))
            return [], [], 2
        supersedes_id = str(item.get("supersedes_id", "") or "").strip()
        change_reason = str(item.get("change_reason", "") or "").strip()
        if not allow_supersession and (supersedes_id or change_reason):
            json_print(
                command_result(
                    command,
                    ok=False,
                    goal_dir=goal_dir,
                    errors=["acceptance supersession fields are only valid through contract-supersede-stage"],
                )
            )
            return [], [], 2
        if allow_supersession:
            if required_supersedes_ids is not None and supersedes_id not in required_supersedes_ids:
                json_print(
                    command_result(
                        command,
                        ok=False,
                        goal_dir=goal_dir,
                        errors=[f"acceptance-json[{index}]: supersedes_id must reference one superseded acceptance id"],
                    )
                )
                return [], [], 2
            if supersedes_id and not change_reason:
                change_reason = default_change_reason
            if change_reason and not supersedes_id:
                json_print(
                    command_result(
                        command,
                        ok=False,
                        goal_dir=goal_dir,
                        errors=[f"acceptance-json[{index}]: change_reason requires supersedes_id"],
                    )
                )
                return [], [], 2
        noncanonical_keys = sorted(set(item) & {"evidence_required_json", "verifier_json", "review_focus_json"})
        if noncanonical_keys:
            json_print(
                command_result(
                    command,
                    ok=False,
                    goal_dir=goal_dir,
                    errors=[f"acceptance-json[{index}]: noncanonical keys are not allowed: {','.join(noncanonical_keys)}"],
                )
            )
            return [], [], 2
        evidence_required = item.get("evidence_required", [])
        verifier = item.get("verifier", [])
        review_focus = item.get("review_focus", [])
        row = {
            "schema": ACCEPTANCE_SCHEMA,
            "goal_id": goal_id,
            "id": acceptance_id,
            "plan_item_id": plan_item_id,
            "requirement": str(item.get("requirement", "")).strip(),
            "observable_outcome": str(item.get("observable_outcome", "")).strip(),
            "evidence_required_json": as_json_cell(evidence_required),
            "verifier_json": as_json_cell(verifier),
            "review_focus_json": as_json_cell(review_focus),
            "required": as_bool_cell(bool(item.get("required", True))),
            "status": "unknown",
            "evidence_ids_json": as_json_cell([]),
            "cv_id": "",
            "verified_by": "",
            "verified_at": "",
            "locked": "",
            "locked_at": "",
            "locked_by": "",
            "supersedes_id": supersedes_id,
            "change_reason": change_reason,
            "lock_hash": "",
        }
        acceptance_ids.append(acceptance_id)
        rows.append(row)
    return acceptance_ids, rows, None


def cmd_contract_add_stage(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    terminal_result = terminal_command_result("contract-add-stage", goal_dir)
    if terminal_result is not None:
        json_print(terminal_result)
        return 2
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    plan_id = args.id.strip()
    if not plan_id:
        json_print(command_result("contract-add-stage", ok=False, goal_dir=goal_dir, errors=["id is required"]))
        return 2
    plan_path = goal_dir / "plan.csv"
    acceptance_path = goal_dir / "acceptance.csv"
    plan_rows = read_csv_rows(plan_path)
    acceptance_rows = read_csv_rows(acceptance_path)
    if any(row.get("id") == plan_id for row in plan_rows):
        json_print(command_result("contract-add-stage", ok=False, goal_dir=goal_dir, errors=[f"duplicate plan id: {plan_id}"]))
        return 2
    json_inputs, failed = contract_stage_json_inputs("contract-add-stage", args, goal_dir, plan_id)
    if failed is not None:
        return failed
    parsed_cells: dict[str, Any] = {}
    for field, expected_type in (
        ("depends-on-json", list),
        ("scope-json", dict),
        ("work-json", dict),
        ("gate-json", dict),
        ("recovery-json", dict),
        ("budget-json", dict),
    ):
        parsed, failed = parse_required_json_cell("contract-add-stage", goal_dir, field, json_inputs[field], expected_type)
        if failed is not None:
            return failed
        parsed_cells[field] = parsed
    acceptance_ids, new_acceptance_rows, failed = normalize_acceptance_contracts(
        "contract-add-stage",
        goal_dir,
        goal.get("goal_id", ""),
        plan_id,
        args.acceptance_json,
    )
    if failed is not None:
        return failed
    existing_acceptance_ids = {row.get("id", "") for row in acceptance_rows if row.get("id")}
    duplicate_acceptance = sorted(existing_acceptance_ids.intersection(acceptance_ids))
    if duplicate_acceptance:
        json_print(command_result("contract-add-stage", ok=False, goal_dir=goal_dir, errors=["duplicate acceptance id: " + ",".join(duplicate_acceptance)]))
        return 2
    row = {
        "schema": PLAN_SCHEMA,
        "goal_id": goal.get("goal_id", ""),
        "revision": args.revision,
        "id": plan_id,
        "title": args.title,
        "description": args.description,
        "contract_status": "pending",
        "required": as_bool_cell(not args.optional),
        "depends_on_json": as_json_cell(parsed_cells["depends-on-json"]),
        "scope_json": as_json_cell(parsed_cells["scope-json"]),
        "work_json": as_json_cell(parsed_cells["work-json"]),
        "gate_json": as_json_cell(parsed_cells["gate-json"]),
        "recovery_json": as_json_cell(parsed_cells["recovery-json"]),
        "budget_json": as_json_cell(parsed_cells["budget-json"]),
        "acceptance_ids_json": as_json_cell(acceptance_ids),
        "locked": "",
        "locked_at": "",
        "locked_by": "",
        "supersedes_id": "",
        "change_reason": "",
        "lock_hash": "",
    }
    staged_plan_rows = [*plan_rows, row]
    staged_acceptance_rows = [*acceptance_rows, *new_acceptance_rows]
    errors = validate_contract_snapshot(
        goal_dir,
        plan_rows=staged_plan_rows,
        acceptance_rows=staged_acceptance_rows,
        require_complete=True,
    )
    if errors:
        json_print(
            command_contract_error(
                "contract-add-stage",
                goal_dir,
                errors,
                data={"plan_item_id": plan_id, "plan_row": row, "acceptance_rows": new_acceptance_rows},
            )
        )
        return 2
    writes: list[CsvWrite] = [
        (plan_path, PLAN_FIELDS, staged_plan_rows),
        (acceptance_path, ACCEPTANCE_FIELDS, staged_acceptance_rows),
    ]
    try:
        write_csv_files_atomically(writes)
    except MobiusError as exc:
        json_print(command_result("contract-add-stage", ok=False, goal_dir=goal_dir, errors=[str(exc)], next_required_action="retry_after_storage_error"))
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(
            command_contract_error(
                "contract-add-stage",
                goal_dir,
                errors,
                updated_files=["plan.csv", "acceptance.csv"],
                data={"plan_item_id": plan_id, "plan_row": row, "acceptance_rows": new_acceptance_rows},
            )
        )
        return 2
    json_print(
        command_result(
            "contract-add-stage",
            goal_dir=goal_dir,
            updated_files=["acceptance.csv", "plan.csv"],
            next_required_action="add_more_stages_or_validate_contract",
            data={"plan_item_id": plan_id, "acceptance_ids": acceptance_ids, "plan_row": row, "acceptance_rows": new_acceptance_rows},
        )
    )
    return 0


def cmd_contract_supersede_stage(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    command = "contract-supersede-stage"
    terminal_result = terminal_command_result(command, goal_dir)
    if terminal_result is not None:
        json_print(terminal_result)
        return 2
    old_plan_id = args.supersedes_id.strip()
    new_plan_id = args.id.strip()
    change_reason = args.change_reason.strip()
    if not old_plan_id:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=["supersedes-id is required"]))
        return 2
    if not new_plan_id:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=["id is required"]))
        return 2
    if old_plan_id == new_plan_id:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=["replacement id must differ from supersedes-id"]))
        return 2
    if not change_reason:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=["change-reason is required"]))
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error(command, goal_dir, errors))
        return 2
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    plan_path = goal_dir / "plan.csv"
    acceptance_path = goal_dir / "acceptance.csv"
    plan_rows = read_csv_rows(plan_path)
    acceptance_rows = read_csv_rows(acceptance_path)
    active_plan_rows = [row for row in plan_rows if row.get("contract_status") != "superseded"]
    old_plan = next((row for row in active_plan_rows if row.get("id") == old_plan_id), None)
    if old_plan is None:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=[f"active plan id not found: {old_plan_id}"]))
        return 2
    if any(row.get("id") == new_plan_id for row in plan_rows):
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=[f"duplicate plan id: {new_plan_id}"]))
        return 2
    active_dependents: list[str] = []
    for row in active_plan_rows:
        if row.get("id") == old_plan_id:
            continue
        try:
            dependencies = from_json_cell(row.get("depends_on_json", ""), [])
        except json.JSONDecodeError:
            dependencies = []
        if isinstance(dependencies, list) and old_plan_id in {str(item) for item in dependencies}:
            active_dependents.append(row.get("id", ""))
    if active_dependents:
        json_print(
            command_result(
                command,
                ok=False,
                goal_dir=goal_dir,
                errors=["cannot supersede plan item with active dependents: " + ",".join(sorted(active_dependents))],
                next_required_action="supersede_dependent_stages_first",
            )
        )
        return 2
    try:
        old_acceptance_ids = [str(item) for item in from_json_cell(old_plan.get("acceptance_ids_json", ""), [])]
    except json.JSONDecodeError as exc:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=[f"plan.csv:{old_plan_id}:acceptance_ids_json: invalid JSON cell: {exc.msg}"]))
        return 2
    active_old_acceptance_ids = {
        row.get("id", "")
        for row in acceptance_rows
        if row.get("id", "") in set(old_acceptance_ids) and row.get("status") != "superseded"
    }
    json_inputs, failed = contract_stage_json_inputs(command, args, goal_dir, new_plan_id)
    if failed is not None:
        return failed
    parsed_cells: dict[str, Any] = {}
    for field, expected_type in (
        ("depends-on-json", list),
        ("scope-json", dict),
        ("work-json", dict),
        ("gate-json", dict),
        ("recovery-json", dict),
        ("budget-json", dict),
    ):
        parsed, failed = parse_required_json_cell(command, goal_dir, field, json_inputs[field], expected_type)
        if failed is not None:
            return failed
        parsed_cells[field] = parsed
    acceptance_ids, new_acceptance_rows, failed = normalize_acceptance_contracts(
        command,
        goal_dir,
        goal.get("goal_id", ""),
        new_plan_id,
        args.acceptance_json,
        allow_supersession=True,
        default_change_reason=change_reason,
        required_supersedes_ids=active_old_acceptance_ids,
    )
    if failed is not None:
        return failed
    replacement_supersedes = [row.get("supersedes_id", "") for row in new_acceptance_rows]
    if sorted(replacement_supersedes) != sorted(active_old_acceptance_ids):
        json_print(
            command_result(
                command,
                ok=False,
                goal_dir=goal_dir,
                errors=["replacement acceptance rows must cover exactly the superseded acceptance ids"],
            )
        )
        return 2
    existing_acceptance_ids = {row.get("id", "") for row in acceptance_rows if row.get("id")}
    duplicate_acceptance = sorted(existing_acceptance_ids.intersection(acceptance_ids))
    if duplicate_acceptance:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=["duplicate acceptance id: " + ",".join(duplicate_acceptance)]))
        return 2
    new_plan_row = {
        "schema": PLAN_SCHEMA,
        "goal_id": goal.get("goal_id", ""),
        "revision": args.revision,
        "id": new_plan_id,
        "title": args.title,
        "description": args.description,
        "contract_status": "pending",
        "required": as_bool_cell(not args.optional),
        "depends_on_json": as_json_cell(parsed_cells["depends-on-json"]),
        "scope_json": as_json_cell(parsed_cells["scope-json"]),
        "work_json": as_json_cell(parsed_cells["work-json"]),
        "gate_json": as_json_cell(parsed_cells["gate-json"]),
        "recovery_json": as_json_cell(parsed_cells["recovery-json"]),
        "budget_json": as_json_cell(parsed_cells["budget-json"]),
        "acceptance_ids_json": as_json_cell(acceptance_ids),
        "locked": "",
        "locked_at": "",
        "locked_by": "",
        "supersedes_id": old_plan_id,
        "change_reason": change_reason,
        "lock_hash": "",
    }
    staged_plan_rows = [dict(row) for row in plan_rows]
    staged_acceptance_rows = [dict(row) for row in acceptance_rows]
    for row in staged_plan_rows:
        if row.get("id") == old_plan_id:
            row["contract_status"] = "superseded"
            row["change_reason"] = change_reason
    for row in staged_acceptance_rows:
        if row.get("id") in active_old_acceptance_ids:
            validate_state_transition("acceptance", row.get("status", "unknown"), "superseded")
            row["status"] = "superseded"
            row["change_reason"] = change_reason
    staged_plan_rows.append(new_plan_row)
    staged_acceptance_rows.extend(new_acceptance_rows)
    errors = validate_contract_snapshot(
        goal_dir,
        plan_rows=staged_plan_rows,
        acceptance_rows=staged_acceptance_rows,
        require_complete=True,
    )
    if errors:
        json_print(
            command_contract_error(
                command,
                goal_dir,
                errors,
                data={"superseded_plan_item_id": old_plan_id, "replacement_plan_item_id": new_plan_id},
            )
        )
        return 2
    try:
        write_csv_files_atomically(
            [
                (plan_path, PLAN_FIELDS, staged_plan_rows),
                (acceptance_path, ACCEPTANCE_FIELDS, staged_acceptance_rows),
            ]
        )
    except MobiusError as exc:
        json_print(command_result(command, ok=False, goal_dir=goal_dir, errors=[str(exc)], next_required_action="retry_after_storage_error"))
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(
            command_contract_error(
                command,
                goal_dir,
                errors,
                updated_files=["plan.csv", "acceptance.csv"],
                data={"superseded_plan_item_id": old_plan_id, "replacement_plan_item_id": new_plan_id},
            )
        )
        return 2
    json_print(
        command_result(
            command,
            goal_dir=goal_dir,
            updated_files=["acceptance.csv", "plan.csv"],
            next_required_action="lock_contract",
            data={
                "superseded_plan_item_id": old_plan_id,
                "superseded_acceptance_ids": sorted(active_old_acceptance_ids),
                "replacement_plan_item_id": new_plan_id,
                "replacement_acceptance_ids": acceptance_ids,
                "plan_row": new_plan_row,
                "acceptance_rows": new_acceptance_rows,
            },
        )
    )
    return 0


def cmd_evidence_add(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    terminal_result = terminal_command_result("evidence-add", goal_dir)
    if terminal_result is not None:
        json_print(terminal_result)
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("evidence-add", goal_dir, errors))
        return 2
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    evidence_path = goal_dir / "evidence.csv"
    artifact = None
    if args.artifact_json:
        try:
            artifact = artifact_json_record(root, args.artifact_json, args.type)
        except MobiusError as exc:
            json_print(command_result("evidence-add", ok=False, goal_dir=goal_dir, errors=[str(exc)], next_required_action="fix_artifact_json"))
            return 2
    if args.artifact:
        if args.type not in PATH_PROOF_TYPES:
            json_print(
                command_result(
                    "evidence-add",
                    ok=False,
                    goal_dir=goal_dir,
                    errors=[f"--artifact path refs are only allowed for evidence types: {sorted_join(PATH_PROOF_TYPES)}"],
                    next_required_action="fix_artifact_path",
                )
            )
            return 2
        try:
            file_artifact = artifact_record(root, args.artifact, args.summary)
            artifact = {**(artifact or {}), **file_artifact}
        except MobiusError as exc:
            json_print(command_result("evidence-add", ok=False, goal_dir=goal_dir, errors=[str(exc)], next_required_action="fix_artifact_path"))
            return 2
    record = {
        "schema": "mobius.evidence",
        "id": next_id(evidence_path, "E"),
        "goal_id": goal.get("goal_id", ""),
        "type": args.type,
        "summary": args.summary,
        "supports_json": as_json_cell(args.supports),
        "artifact_json": as_json_cell(artifact),
        "created_by": args.created_by,
        "created_at": now_iso(),
    }
    evidence_errors = validate_evidence_record_against_acceptance(goal_dir, record)
    if evidence_errors:
        json_print(command_result("evidence-add", ok=False, goal_dir=goal_dir, errors=evidence_errors, next_required_action="fix_evidence_contract"))
        return 2
    evidence_rows = [*read_csv_rows(evidence_path), record]
    verdict = derive_verdict(goal_dir, evidence_rows=evidence_rows)
    try:
        write_csv_files_atomically(
            [
                (evidence_path, EVIDENCE_FIELDS, evidence_rows),
                (goal_dir / "verdict.csv", VERDICT_FIELDS, [verdict]),
            ]
        )
    except MobiusError as exc:
        json_print(command_result("evidence-add", ok=False, goal_dir=goal_dir, errors=[str(exc)], next_required_action="retry_after_storage_error"))
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("evidence-add", goal_dir, errors, updated_files=["evidence.csv", "verdict.csv"], data={"evidence_id": record["id"], "row": record, "verdict": verdict}))
        return 2
    json_print(
        loop_command_result(
            "evidence-add",
            root,
            args.session_id,
            args.goal_slug,
            updated_files=["evidence.csv", "verdict.csv"],
            data={"evidence_id": record["id"], "row": record, "verdict": verdict},
        )
    )
    return 0


def cmd_contract_lock(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    terminal_result = terminal_command_result("contract-lock", goal_dir)
    if terminal_result is not None:
        json_print(terminal_result)
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_result("contract-lock", ok=False, goal_dir=goal_dir, errors=errors, next_required_action="fix_contract"))
        return 2
    timestamp = now_iso()
    locked_counts: dict[str, int] = {}

    for filename, fields, structural_fields in (
        ("plan.csv", PLAN_FIELDS, PLAN_STRUCTURAL_FIELDS),
        ("acceptance.csv", ACCEPTANCE_FIELDS, ACCEPTANCE_STRUCTURAL_FIELDS),
    ):
        path = goal_dir / filename
        rows = read_csv_rows(path)
        locked = 0
        for row in rows:
            if row.get("locked") != "true":
                row["locked"] = "true"
                row["locked_at"] = timestamp
                row["locked_by"] = args.locked_by
                row.setdefault("supersedes_id", "")
                row.setdefault("change_reason", "")
                row["lock_hash"] = structural_hash(row, structural_fields)
                locked += 1
            elif not row.get("lock_hash"):
                row["lock_hash"] = structural_hash(row, structural_fields)
        write_csv_rows(path, fields, rows)
        locked_counts[filename] = locked

    goal_path = goal_dir / "goal.csv"
    goal = read_single_csv(goal_path) or {}
    contract_already_locked = False
    if goal:
        try:
            front, _body = parse_goal_contract(goal_dir / "goal.md")
        except (MobiusError, tomllib.TOMLDecodeError) as exc:
            json_print(command_result("contract-lock", ok=False, goal_dir=goal_dir, errors=[f"goal.md: {exc}"], next_required_action="fix_contract"))
            return 2
        contract_already_locked = bool(str(front.get("locked_at", "")).strip() and str(front.get("locked_by", "")).strip())
        if contract_already_locked:
            contract_tail = sha256_tail(sha256_file(goal_dir / "goal.md"))
        else:
            contract_tail = lock_goal_contract(goal_dir, timestamp, args.locked_by)
        validate_state_transition("goal", goal.get("status", "planning"), "active")
        goal["status"] = "active"
        goal["updated_at"] = timestamp
        goal["contract_sha256_tail"] = contract_tail
        write_single_csv(goal_path, GOAL_FIELDS, goal)
        run_path = run_dir(root, args.session_id) / "run.csv"
        run = read_single_csv(run_path) or {}
        goals = from_json_cell(run.get("goals_json", ""), [])
        for item in goals:
            if item.get("path") == args.goal_slug:
                validate_state_transition("goal", str(item.get("status", "planning")), "active")
                item["status"] = "active"
        run["goals_json"] = as_json_cell(goals)
        write_single_csv(run_path, RUN_FIELDS, run)

    verdict = compute_verdict(goal_dir)
    updated_files = ["goal.csv", "run.csv", "plan.csv", "acceptance.csv", "verdict.csv"]
    if goal and not contract_already_locked:
        updated_files.insert(0, "goal.md")
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("contract-lock", goal_dir, errors, updated_files=updated_files))
        return 2
    json_print(
        command_result(
            "contract-lock",
            goal_dir=goal_dir,
            updated_files=updated_files,
            next_required_action="start_loop",
            data={"locked_counts": locked_counts, "verdict": verdict},
        )
    )
    return 0


def validate_json_cell(errors: list[str], path: Path, row_id: str, row: dict[str, str], field: str, default: Any) -> None:
    try:
        from_json_cell(row.get(field, ""), default)
    except json.JSONDecodeError as exc:
        errors.append(f"{path.name}:{row_id}:{field}: invalid JSON cell: {exc.msg}")


def parse_json_for_validation(
    errors: list[str],
    path: Path,
    row_id: str,
    row: dict[str, str],
    field: str,
    default: Any,
    expected_type: type | tuple[type, ...],
) -> Any:
    try:
        parsed = from_json_cell(row.get(field, ""), default)
    except json.JSONDecodeError as exc:
        errors.append(f"{path.name}:{row_id}:{field}: invalid JSON cell: {exc.msg}")
        return default
    if not isinstance(parsed, expected_type):
        if isinstance(expected_type, tuple):
            expected = " or ".join(item.__name__ for item in expected_type)
        else:
            expected = expected_type.__name__
        errors.append(f"{path.name}:{row_id}:{field}: must be {expected}")
        return default
    return parsed


def non_empty_list(value: Any) -> bool:
    return isinstance(value, list) and any(str(item).strip() for item in value)


def non_empty_value(value: Any) -> bool:
    if isinstance(value, str):
        return bool(value.strip())
    if isinstance(value, (list, dict, tuple, set)):
        return bool(value)
    return value is not None


def object_value(data: dict[str, Any], *keys: str) -> Any:
    for key in keys:
        if key in data:
            return data.get(key)
    return None


def object_has_non_empty(data: dict[str, Any], *keys: str) -> bool:
    return any(non_empty_value(object_value(data, key)) for key in keys)


def text_has_vague_term(text: str) -> bool:
    lowered = text.lower()
    return any(re.search(rf"\b{re.escape(term)}\b", lowered) for term in VAGUE_ACCEPTANCE_TERMS)


def concrete_observable(text: str) -> bool:
    stripped = text.strip()
    if not stripped:
        return False
    if text_has_vague_term(stripped):
        return False
    return bool(re.search(r"\b(command|exit code|file|path|row|csv|json|artifact|log|packet|verdict|output|status|review)\b", stripped, re.IGNORECASE))


def sorted_join(values: set[str]) -> str:
    return ",".join(sorted(values))


def validate_typed_contract_items(
    errors: list[str],
    *,
    path_name: str,
    row_id: str,
    field: str,
    items: list[Any],
    allowed_types: set[str],
    type_label: str,
) -> None:
    for index, item in enumerate(items):
        if not isinstance(item, dict):
            continue
        item_type = str(item.get("type", "")).strip()
        if not item_type:
            errors.append(f"{path_name}:{row_id}: {field}[{index}] must declare type")
        elif item_type not in allowed_types:
            errors.append(
                f"{path_name}:{row_id}: unsupported {type_label} type in {field}[{index}]: "
                f"{item_type}; supported types: {sorted_join(allowed_types)}"
            )


def detect_dependency_cycle(graph: dict[str, list[str]]) -> list[str]:
    visiting: set[str] = set()
    visited: set[str] = set()
    stack: list[str] = []

    def visit(node: str) -> list[str]:
        if node in visiting:
            try:
                index = stack.index(node)
            except ValueError:
                index = 0
            return [*stack[index:], node]
        if node in visited:
            return []
        visiting.add(node)
        stack.append(node)
        for dep in graph.get(node, []):
            cycle = visit(dep)
            if cycle:
                return cycle
        stack.pop()
        visiting.remove(node)
        visited.add(node)
        return []

    for node in graph:
        cycle = visit(node)
        if cycle:
            return cycle
    return []


def validate_contract_snapshot(
    goal_dir: Path,
    *,
    plan_rows: list[dict[str, str]] | None = None,
    acceptance_rows: list[dict[str, str]] | None = None,
    require_complete: bool = True,
) -> list[str]:
    with tempfile.TemporaryDirectory(prefix="mobius-contract-validate-") as tmp:
        temp_goal = Path(tmp) / goal_dir.name
        temp_goal.mkdir(parents=True)
        for name in ("goal.md", "goal.csv", "evidence.csv", "packets.csv", "cv.csv", "loop.csv", "review_attempts.csv", "verdict.csv"):
            source = goal_dir / name
            if source.exists():
                shutil.copy2(source, temp_goal / name)
        write_csv_rows(temp_goal / "plan.csv", PLAN_FIELDS, plan_rows if plan_rows is not None else read_csv_rows(goal_dir / "plan.csv"))
        write_csv_rows(
            temp_goal / "acceptance.csv",
            ACCEPTANCE_FIELDS,
            acceptance_rows if acceptance_rows is not None else read_csv_rows(goal_dir / "acceptance.csv"),
        )
        return validate_contract_dir(temp_goal, require_complete=require_complete)


def validate_contract_dir(goal_dir: Path, *, require_complete: bool = True) -> list[str]:
    errors: list[str] = []
    goal_path = goal_dir / "goal.csv"
    goal = read_single_csv(goal_path) or {}
    if not goal:
        errors.append("goal.csv:goal: missing")
    else:
        with goal_path.open("r", encoding="utf-8", newline="") as handle:
            header = csv.DictReader(handle).fieldnames or []
        if header != GOAL_FIELDS:
            errors.append("goal.csv: header must match reduced goal state fields")
        if goal.get("schema") != "mobius.goal_state":
            errors.append(f"goal.csv:goal: unsupported schema: {goal.get('schema', '')}; expected mobius.goal_state")
        if goal.get("status") not in GOAL_STATUSES:
            errors.append(f"goal.csv:goal: invalid status: {goal.get('status', '')}")
        if goal.get("goal_slug") != goal_dir.name:
            errors.append(f"goal.csv:goal: goal_slug mismatch: expected {goal_dir.name}, got {goal.get('goal_slug', '')}")
        contract_path = goal.get("contract_path", "")
        if contract_path != "goal.md":
            errors.append("goal.csv:goal: contract_path must be goal.md")
        contract = goal_dir / contract_path
        if not contract.exists():
            errors.append("goal.md: missing")
        else:
            try:
                front, _body = parse_goal_contract(contract)
            except (MobiusError, tomllib.TOMLDecodeError) as exc:
                errors.append(f"goal.md: {exc}")
            else:
                required_front = {"schema", "goal_id", "run_id", "goal_slug", "title", "created_at", "locked_at", "locked_by", "non_goals"}
                missing_front = sorted(required_front - set(front))
                if missing_front:
                    errors.append("goal.md: missing front matter keys: " + ",".join(missing_front))
                if front.get("schema") != "mobius.goal_contract":
                    errors.append(f"goal.md: unsupported schema: {front.get('schema', '')}; expected mobius.goal_contract")
                for key in ("goal_id", "run_id", "goal_slug"):
                    if str(front.get(key, "")) != str(goal.get(key, "")):
                        errors.append(f"goal.md: {key} mismatch")
                non_goals = front.get("non_goals")
                if not isinstance(non_goals, list):
                    errors.append("goal.md: non_goals must be a list")
                if goal.get("status") != "planning":
                    if not str(front.get("locked_at", "")).strip():
                        errors.append("goal.md: locked_at is required once goal status is active or terminal")
                    if not str(front.get("locked_by", "")).strip():
                        errors.append("goal.md: locked_by is required once goal status is active or terminal")
                actual_tail = sha256_tail(sha256_file(contract))
                if goal.get("contract_sha256_tail") != actual_tail:
                    errors.append("goal.csv:goal: contract_sha256_tail mismatch")

    parsed_plan_acceptance: dict[str, list[str]] = {}
    parsed_plan_dependencies: dict[str, list[str]] = {}
    parsed_plan_scope: dict[str, dict[str, Any]] = {}
    parsed_plan_work: dict[str, dict[str, Any]] = {}
    parsed_plan_gate: dict[str, dict[str, Any]] = {}
    parsed_plan_recovery: dict[str, dict[str, Any]] = {}
    parsed_plan_budget: dict[str, dict[str, Any]] = {}
    parsed_acceptance_evidence_required: dict[str, list[Any]] = {}
    parsed_acceptance_verifier: dict[str, list[Any]] = {}
    parsed_acceptance_review_focus: dict[str, list[Any]] = {}
    parsed_acceptance_evidence_ids: dict[str, list[str]] = {}
    for filename, fields, structural_fields, json_fields, expected_schema in (
        (
            "plan.csv",
            PLAN_FIELDS,
            PLAN_STRUCTURAL_FIELDS,
            ["depends_on_json", "scope_json", "work_json", "gate_json", "recovery_json", "budget_json", "acceptance_ids_json"],
            PLAN_SCHEMA,
        ),
        (
            "acceptance.csv",
            ACCEPTANCE_FIELDS,
            ACCEPTANCE_STRUCTURAL_FIELDS,
            ["evidence_required_json", "verifier_json", "review_focus_json", "evidence_ids_json"],
            ACCEPTANCE_SCHEMA,
        ),
    ):
        path = goal_dir / filename
        if not path.exists():
            errors.append(f"{goal_dir}:{filename}: missing")
            continue
        rows = read_csv_rows(path)
        with path.open("r", encoding="utf-8", newline="") as handle:
            reader = csv.DictReader(handle)
            header = reader.fieldnames or []
        missing = [field for field in fields if field not in header]
        if missing:
            errors.append(f"{path}: missing columns: {','.join(missing)}")
        seen_ids: dict[str, int] = {}
        for row in rows:
            row_id_value = row.get("id", "")
            if row_id_value:
                seen_ids[row_id_value] = seen_ids.get(row_id_value, 0) + 1
        all_ids = set(seen_ids)
        active_ids: set[str] = set()
        for index, row in enumerate(rows, start=2):
            row_id = row.get("id", "") or f"line-{index}"
            if not row.get("id"):
                errors.append(f"{path.name}:{row_id}: missing id")
            if row.get("schema") != expected_schema:
                errors.append(f"{path.name}:{row_id}: unsupported schema: {row.get('schema', '')}; expected {expected_schema}")
            if row.get("id") and seen_ids.get(row.get("id", ""), 0) > 1:
                errors.append(f"{path.name}:{row.get('id', '')}: reused id is not allowed")
            row_status = row.get("contract_status") if filename == "plan.csv" else row.get("status")
            if row_status != "superseded":
                if row_id in active_ids:
                    errors.append(f"{path.name}:{row_id}: duplicate active id")
                active_ids.add(row_id)
            if filename == "plan.csv" and row.get("contract_status") not in PLAN_STATUSES:
                errors.append(f"{path.name}:{row_id}: invalid contract_status: {row.get('contract_status', '')}")
            if filename == "acceptance.csv" and row.get("status") not in ACCEPTANCE_STATUSES:
                errors.append(f"{path.name}:{row_id}: invalid status: {row.get('status', '')}")
            for field in json_fields:
                validate_json_cell(errors, path, row_id, row, field, [])
            if filename == "plan.csv":
                deps = parse_json_for_validation(errors, path, row_id, row, "depends_on_json", [], list)
                ids = parse_json_for_validation(errors, path, row_id, row, "acceptance_ids_json", [], list)
                parsed_plan_dependencies[row_id] = [str(item) for item in deps]
                parsed_plan_acceptance[row_id] = [str(item) for item in ids]
                parsed_plan_scope[row_id] = parse_json_for_validation(errors, path, row_id, row, "scope_json", {}, dict)
                parsed_plan_work[row_id] = parse_json_for_validation(errors, path, row_id, row, "work_json", {}, dict)
                parsed_plan_gate[row_id] = parse_json_for_validation(errors, path, row_id, row, "gate_json", {}, dict)
                parsed_plan_recovery[row_id] = parse_json_for_validation(errors, path, row_id, row, "recovery_json", {}, dict)
                parsed_plan_budget[row_id] = parse_json_for_validation(errors, path, row_id, row, "budget_json", {}, dict)
            if filename == "acceptance.csv":
                parsed_acceptance_evidence_required[row_id] = parse_json_for_validation(
                    errors, path, row_id, row, "evidence_required_json", [], list
                )
                parsed_acceptance_verifier[row_id] = parse_json_for_validation(errors, path, row_id, row, "verifier_json", [], list)
                parsed_acceptance_review_focus[row_id] = parse_json_for_validation(errors, path, row_id, row, "review_focus_json", [], list)
                parsed_acceptance_evidence_ids[row_id] = [
                    str(item)
                    for item in parse_json_for_validation(errors, path, row_id, row, "evidence_ids_json", [], list)
                ]
            if row.get("supersedes_id"):
                if row["supersedes_id"] not in all_ids:
                    errors.append(f"{path.name}:{row_id}: supersedes_id does not point to an existing row")
                if not row.get("change_reason"):
                    errors.append(f"{path.name}:{row_id}: change_reason is required when supersedes_id is set")
            if from_bool_cell(row.get("locked", "")):
                if not row.get("locked_at"):
                    errors.append(f"{path.name}:{row_id}: locked_at is required when locked=true")
                if not row.get("locked_by"):
                    errors.append(f"{path.name}:{row_id}: locked_by is required when locked=true")
                expected = structural_hash(row, structural_fields)
                if not row.get("lock_hash"):
                    errors.append(f"{path.name}:{row_id}: lock_hash is required when locked=true")
                elif row.get("lock_hash") != expected:
                    errors.append(f"{path.name}:{row_id}: locked structural fields changed after lock")
    plan_rows = [row for row in read_csv_rows(goal_dir / "plan.csv") if row.get("contract_status") != "superseded"]
    acceptance_rows = [row for row in read_csv_rows(goal_dir / "acceptance.csv") if row.get("status") != "superseded"]
    plan_ids = {row.get("id", "") for row in plan_rows if row.get("id")}
    acceptance_ids = {row.get("id", "") for row in acceptance_rows if row.get("id")}
    required_acceptance_ids = {
        row.get("id", "")
        for row in acceptance_rows
        if row.get("id") and from_bool_cell(row.get("required", ""), True)
    }
    required_plan_ids = {row.get("id", "") for row in plan_rows if from_bool_cell(row.get("required", ""), True)}
    optional_plan_ids = {row.get("id", "") for row in plan_rows if not from_bool_cell(row.get("required", ""), True)}
    first_required_plan_id = next((row.get("id", "") for row in plan_rows if from_bool_cell(row.get("required", ""), True)), "")
    if require_complete and not required_plan_ids:
        errors.append("plan.csv: contract requires at least one active required plan item")
    reachable_required_acceptance: set[str] = set()
    dependency_graph: dict[str, list[str]] = {}
    for row in plan_rows:
        plan_id = row.get("id", "")
        linked_ids = parsed_plan_acceptance.get(plan_id, [])
        if require_complete and from_bool_cell(row.get("required", ""), True) and not linked_ids:
            errors.append(f"plan.csv:{plan_id}: required plan item must link at least one acceptance id")
        if require_complete and from_bool_cell(row.get("required", ""), True) and linked_ids:
            linked_required_ids = sorted(set(linked_ids) & required_acceptance_ids)
            if not linked_required_ids:
                errors.append(f"plan.csv:{plan_id}: required plan item must link at least one required acceptance id")
        for acceptance_id in linked_ids:
            if acceptance_id not in acceptance_ids:
                errors.append(f"plan.csv:{plan_id}: acceptance_ids_json references missing acceptance id: {acceptance_id}")
            else:
                reachable_required_acceptance.add(acceptance_id)
        dependencies = parsed_plan_dependencies.get(plan_id, [])
        if from_bool_cell(row.get("required", ""), True):
            if require_complete and plan_id != first_required_plan_id and not dependencies:
                errors.append(f"plan.csv:{plan_id}: required non-root plan item must declare depends_on_json")
            dependency_graph[plan_id] = dependencies
        for dep in dependencies:
            if dep in optional_plan_ids:
                errors.append(f"plan.csv:{plan_id}: depends_on_json references optional-only predecessor: {dep}")
            elif dep not in plan_ids:
                errors.append(f"plan.csv:{plan_id}: depends_on_json references missing plan item: {dep}")
        if from_bool_cell(row.get("required", ""), True):
            scope = parsed_plan_scope.get(plan_id, {})
            work = parsed_plan_work.get(plan_id, {})
            gate = parsed_plan_gate.get(plan_id, {})
            recovery = parsed_plan_recovery.get(plan_id, {})
            budget = parsed_plan_budget.get(plan_id, {})
            for field, parsed in (
                ("scope_json", scope),
                ("work_json", work),
                ("gate_json", gate),
                ("recovery_json", recovery),
                ("budget_json", budget),
            ):
                if require_complete and not parsed:
                    errors.append(f"plan.csv:{plan_id}: required plan item missing {field}")
            if require_complete:
                recovery_aliases = sorted(set(recovery) & {"rollback", "restart", "escalation"})
                if recovery_aliases:
                    errors.append(f"plan.csv:{plan_id}: recovery_json contains noncanonical keys: {','.join(recovery_aliases)}")
                budget_aliases = sorted(set(budget) & {"retries", "stop"})
                if budget_aliases:
                    errors.append(f"plan.csv:{plan_id}: budget_json contains noncanonical keys: {','.join(budget_aliases)}")
                if not non_empty_list(scope.get("allowed_paths")):
                    errors.append(f"plan.csv:{plan_id}: scope_json.allowed_paths must be non-empty")
                if "forbidden_paths" not in scope or not isinstance(scope.get("forbidden_paths"), list):
                    errors.append(f"plan.csv:{plan_id}: scope_json.forbidden_paths must be present as a list")
                if not (non_empty_list(work.get("target_refs")) or non_empty_list(work.get("deliverables"))):
                    errors.append(f"plan.csv:{plan_id}: work_json requires target_refs or deliverables")
                if not non_empty_list(gate.get("exit")):
                    errors.append(f"plan.csv:{plan_id}: gate_json.exit must be non-empty")
                if not non_empty_list(gate.get("verifiers")):
                    errors.append(f"plan.csv:{plan_id}: gate_json.verifiers must be non-empty")
                else:
                    for index, verifier in enumerate(gate.get("verifiers", [])):
                        verifier_type = str(verifier.get("type", "") if isinstance(verifier, dict) else verifier).strip()
                        if verifier_type and verifier_type not in VERIFIER_TYPES:
                            errors.append(
                                f"plan.csv:{plan_id}: unsupported verifier type in gate_json.verifiers[{index}]: "
                                f"{verifier_type}; supported types: {sorted_join(VERIFIER_TYPES)}"
                            )
                if not object_has_non_empty(recovery, "rollback_boundary"):
                    errors.append(f"plan.csv:{plan_id}: recovery_json requires rollback_boundary")
                if not object_has_non_empty(recovery, "restart_rule"):
                    errors.append(f"plan.csv:{plan_id}: recovery_json requires restart_rule")
                if not object_has_non_empty(recovery, "escalation_rule"):
                    errors.append(f"plan.csv:{plan_id}: recovery_json requires escalation_rule")
                if not object_has_non_empty(budget, "retry_limit", "max_stage_attempts"):
                    errors.append(f"plan.csv:{plan_id}: budget_json requires retry_limit or max_stage_attempts")
                if not object_has_non_empty(budget, "stop_condition"):
                    errors.append(f"plan.csv:{plan_id}: budget_json requires stop_condition")
    cycle = detect_dependency_cycle(dependency_graph)
    if cycle:
        errors.append("plan.csv: dependency cycle detected: " + " -> ".join(cycle))
    for row in acceptance_rows:
        acceptance_id = row.get("id", "")
        plan_item_id = row.get("plan_item_id", "")
        if plan_item_id not in plan_ids:
            errors.append(f"acceptance.csv:{acceptance_id}: plan_item_id does not exist: {plan_item_id}")
        if (
            require_complete
            and from_bool_cell(row.get("required", ""), True)
            and plan_item_id in required_plan_ids
            and acceptance_id not in reachable_required_acceptance
        ):
            errors.append(f"acceptance.csv:{acceptance_id}: required acceptance is not reachable from a required plan item")
        if require_complete and from_bool_cell(row.get("required", ""), True):
            if not row.get("requirement", "").strip():
                errors.append(f"acceptance.csv:{acceptance_id}: requirement is required")
            if not row.get("observable_outcome", "").strip():
                errors.append(f"acceptance.csv:{acceptance_id}: observable_outcome is required")
            evidence_required = parsed_acceptance_evidence_required.get(acceptance_id, [])
            verifier = parsed_acceptance_verifier.get(acceptance_id, [])
            if not evidence_required:
                errors.append(f"acceptance.csv:{acceptance_id}: required acceptance must declare evidence_required_json")
            elif any(not isinstance(item, dict) for item in evidence_required):
                errors.append(f"acceptance.csv:{acceptance_id}: evidence_required_json entries must be objects")
            else:
                validate_typed_contract_items(
                    errors,
                    path_name="acceptance.csv",
                    row_id=acceptance_id,
                    field="evidence_required_json",
                    items=evidence_required,
                    allowed_types=EVIDENCE_TYPES,
                    type_label="evidence",
                )
            if not verifier:
                errors.append(f"acceptance.csv:{acceptance_id}: required acceptance must declare verifier_json")
            elif any(not isinstance(item, dict) for item in verifier):
                errors.append(f"acceptance.csv:{acceptance_id}: verifier_json entries must be objects")
            else:
                validate_typed_contract_items(
                    errors,
                    path_name="acceptance.csv",
                    row_id=acceptance_id,
                    field="verifier_json",
                    items=verifier,
                    allowed_types=VERIFIER_TYPES,
                    type_label="verifier",
                )
            if any(not isinstance(item, (str, dict)) for item in parsed_acceptance_review_focus.get(acceptance_id, [])):
                errors.append(f"acceptance.csv:{acceptance_id}: review_focus_json entries must be strings or objects")
            if text_has_vague_term(row.get("requirement", "")) and not concrete_observable(row.get("observable_outcome", "")):
                errors.append(f"acceptance.csv:{acceptance_id}: vague requirement must be tied to a concrete observable_outcome")
    loop_path = goal_dir / "loop.csv"
    if loop_path.exists():
        with loop_path.open("r", encoding="utf-8", newline="") as handle:
            header = csv.DictReader(handle).fieldnames or []
        missing = [field for field in LOOP_FIELDS if field not in header]
        if missing:
            errors.append(f"{loop_path}: missing columns: {','.join(missing)}")
        for row in read_csv_rows(loop_path):
            plan_item_id = row.get("plan_item_id", "") or "line"
            if row.get("schema") != "mobius.loop_state":
                errors.append(f"loop.csv:{plan_item_id}: unsupported schema: {row.get('schema', '')}; expected mobius.loop_state")
            if row.get("plan_item_id", "") and row.get("plan_item_id", "") not in plan_ids:
                errors.append(f"loop.csv:{plan_item_id}: plan_item_id does not exist")
            if row.get("status", "") not in LOOP_STATUSES:
                errors.append(f"loop.csv:{plan_item_id}: invalid status: {row.get('status', '')}")
            if safe_int(row.get("attempt"), -1) < 0:
                errors.append(f"loop.csv:{plan_item_id}: attempt must be a non-negative integer")
            validate_json_cell(errors, loop_path, plan_item_id, row, "blocking_findings_json", [])
    for evidence in read_csv_rows(goal_dir / "evidence.csv"):
        evidence_id = evidence.get("id", "") or "line"
        supports = parse_json_for_validation(errors, goal_dir / "evidence.csv", evidence_id, evidence, "supports_json", [], list)
        try:
            artifact = from_json_cell(evidence.get("artifact_json", ""), None)
        except json.JSONDecodeError as exc:
            errors.append(f"evidence.csv:{evidence_id}:artifact_json: invalid JSON cell: {exc.msg}")
            artifact = None
        if artifact is not None and not isinstance(artifact, dict):
            errors.append(f"evidence.csv:{evidence_id}: artifact_json must be an object")
            artifact = None
        for acceptance_id in [str(item) for item in supports]:
            if acceptance_id not in acceptance_ids:
                errors.append(f"evidence.csv:{evidence_id}: supports_json references unknown acceptance id: {acceptance_id}")
                continue
            requirements = parsed_acceptance_evidence_required.get(acceptance_id, [])
            matching_structured_required = [
                item
                for item in requirements
                if isinstance(item, dict)
                and str(item.get("type", "")).strip() == evidence.get("type", "")
                and str(item.get("type", "")).strip() in STRUCTURED_PROOF_TYPES
            ]
            if matching_structured_required and not artifact:
                errors.append(f"evidence.csv:{evidence_id}: artifact_json is required for structured proof supporting {acceptance_id}")
    return errors


def contract_error_text(errors: list[str]) -> str:
    return "contract invalid: " + "; ".join(errors)


def command_contract_error(
    command: str,
    goal_dir: Path,
    errors: list[str],
    *,
    updated_files: list[str] | None = None,
    data: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return command_result(
        command,
        ok=False,
        goal_dir=goal_dir,
        updated_files=updated_files,
        errors=errors,
        next_required_action="fix_contract",
        data=data,
    )


def iter_goal_dirs(root: Path, session_id: str | None = None, goal_slug: str | None = None) -> list[Path]:
    mobius_runs = root / ".mobius" / "runs"
    if session_id and goal_slug:
        return [load_goal_dir(root, session_id, goal_slug)]
    if not mobius_runs.exists():
        return []
    goal_dirs: list[Path] = []
    run_dirs = [mobius_runs / f"codex-session-{session_id}"] if session_id else sorted(mobius_runs.glob("codex-session-*"))
    for run in run_dirs:
        if not run.exists():
            continue
        if goal_slug:
            candidates = [run / goal_slug]
        else:
            candidates = [path for path in sorted(run.iterdir()) if path.is_dir()]
        goal_dirs.extend(path for path in candidates if path.exists())
    return goal_dirs


def is_contract_goal_dir(goal_dir: Path) -> bool:
    return (goal_dir / "goal.csv").exists()


def terminal_verdict(goal_dir: Path) -> str:
    verdict = read_single_csv(goal_dir / "verdict.csv") or {}
    overall = str(verdict.get("overall", ""))
    return overall if overall in TERMINAL_VERDICTS else ""


def terminal_goal_error(command: str, overall: str) -> str:
    return f"{command} is not allowed for terminal goal: {overall}"


def require_nonterminal_goal(goal_dir: Path, command: str) -> None:
    overall = terminal_verdict(goal_dir)
    if overall:
        raise MobiusError(terminal_goal_error(command, overall))


def terminal_command_result(command: str, goal_dir: Path) -> dict[str, Any] | None:
    overall = terminal_verdict(goal_dir)
    if not overall:
        return None
    return command_result(
        command,
        ok=False,
        goal_dir=goal_dir,
        gate=overall,
        next_required_action=TERMINAL_NEXT_REQUIRED_ACTION,
        errors=[terminal_goal_error(command, overall)],
    )


def cmd_validate_contract(args: argparse.Namespace) -> int:
    root = project_root(args)
    errors: list[str] = []
    goal_dirs = iter_goal_dirs(root, args.session_id, args.goal_slug)
    for goal_dir in goal_dirs:
        errors.extend(validate_contract_dir(goal_dir))
    if errors:
        json_print(command_result("contract-validate", ok=False, errors=errors, data={"checked_goal_count": len(goal_dirs)}))
        return 2
    json_print(command_result("contract-validate", next_required_action="lock_or_continue_loop", data={"checked_goal_count": len(goal_dirs)}))
    return 0


def packet_hash(packet: dict[str, Any]) -> str:
    encoded = json.dumps(packet, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def packet_mode(review_mode: str) -> str:
    return "exit" if review_mode == "exit_review" else "delta"


def review_mode_from_packet_mode(mode: str) -> str:
    return "exit_review" if mode == "exit" else "delta_review"


def packet_envelope_from_row(row: dict[str, Any]) -> dict[str, Any]:
    try:
        packet = from_json_cell(str(row.get("packet_json", "")), {})
    except json.JSONDecodeError as exc:
        raise MobiusError(f"packets.csv:{row.get('packet_id', '')}: packet_json invalid JSON: {exc.msg}") from exc
    if not isinstance(packet, dict):
        raise MobiusError(f"packets.csv:{row.get('packet_id', '')}: packet_json must be an object")
    return packet


def packet_required_acceptance_ids(packet: dict[str, Any]) -> list[str]:
    coverage = packet.get("coverage")
    if not isinstance(coverage, dict):
        return []
    return [str(item) for item in coverage]


def compact_goal_brief(goal_dir: Path) -> dict[str, Any]:
    contract = goal_dir / "goal.md"
    try:
        front, body = parse_goal_contract(contract)
    except (MobiusError, tomllib.TOMLDecodeError):
        return {"objective": goal_dir.name, "non_goals": [], "risks": []}
    objective = str(front.get("title") or goal_dir.name)
    if body.strip():
        first_line = next((line.strip() for line in body.splitlines() if line.strip() and not line.startswith("#")), "")
        if first_line:
            objective = first_line[:240]
    return {
        "objective": objective,
        "non_goals": [str(item) for item in front.get("non_goals", [])] if isinstance(front.get("non_goals"), list) else [],
        "risks": ["unverified acceptance", "scope drift"],
    }


def packet_envelope(
    root: Path,
    goal_dir: Path,
    packet_id: str,
    goal_slug: str,
    review_mode: str,
    scope: str,
    required_ids: list[str],
) -> dict[str, Any]:
    support = supporting_evidence_by_acceptance(goal_dir)
    refs = evidence_refs_for_packet(goal_dir, required_ids)
    coverage = {acceptance_id: support.get(acceptance_id, []) for acceptance_id in required_ids}
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    try:
        ledger_root = goal_dir.relative_to(root).as_posix()
    except ValueError:
        ledger_root = goal_dir.name
    return {
        "schema": "mobius.packet",
        "packet": packet_id,
        "goal": goal_slug,
        "mode": packet_mode(review_mode),
        "scope": scope,
        "ledger": {
            "root": ledger_root,
            "hash": goal.get("contract_sha256_tail", ""),
        },
        "brief": compact_goal_brief(goal_dir),
        "coverage": coverage,
        "refs": refs,
    }


def cmd_packet_create(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    terminal_result = terminal_command_result("packet-create", goal_dir)
    if terminal_result is not None:
        json_print(terminal_result)
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("packet-create", goal_dir, errors))
        return 2
    locked_result = locked_contract_command_result("packet-create", goal_dir)
    if locked_result is not None:
        json_print(locked_result)
        return 2
    mode_short = packet_mode(args.review_mode)
    packet_ledger_path = goal_dir / "packets.csv"
    existing_packets = read_csv_rows(packet_ledger_path)
    packet_count = len([row for row in existing_packets if row.get("review_mode") == args.review_mode]) + 1
    packet_id = f"packet_{mode_short}_{packet_count:03d}"
    acceptance_rows = read_csv_rows(goal_dir / "acceptance.csv")
    active_ids = [item["id"] for item in acceptance_rows if item.get("status") != "superseded" and from_bool_cell(item.get("required", ""), True)]
    scoped_acceptance_ids = getattr(args, "acceptance_id", None)
    required_ids = [str(item) for item in scoped_acceptance_ids] if args.review_mode == "delta_review" and scoped_acceptance_ids else active_ids
    missing_delta_ids = sorted(set(required_ids) - set(active_ids))
    if missing_delta_ids:
        json_print(
            command_result(
                "packet-create",
                ok=False,
                goal_dir=goal_dir,
                errors=["unknown acceptance ids: " + ",".join(missing_delta_ids)],
                next_required_action="fix_packet_scope",
            )
        )
        return 2
    target_plan_item_id = ""
    if args.review_mode == "delta_review":
        plan_item_ids = {
            row.get("plan_item_id", "")
            for row in acceptance_rows
            if row.get("id") in required_ids and row.get("status") != "superseded"
        }
        if len(plan_item_ids) != 1:
            json_print(
                command_result(
                    "packet-create",
                    ok=False,
                    goal_dir=goal_dir,
                    errors=["delta packet acceptance ids must belong to exactly one plan item"],
                    next_required_action="fix_packet_scope",
                )
            )
            return 2
        target_plan_item_id = next(iter(plan_item_ids))
        stage_required_ids = required_acceptance_ids_for_plan_item(goal_dir, target_plan_item_id)
        if sorted(required_ids) != sorted(stage_required_ids):
            json_print(
                command_result(
                    "packet-create",
                    ok=False,
                    goal_dir=goal_dir,
                    errors=["delta packet must include all linked required acceptance ids for stage: " + target_plan_item_id],
                    next_required_action="fix_packet_scope",
                )
            )
            return 2
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    scope = target_plan_item_id if args.review_mode == "delta_review" else "all"
    envelope = packet_envelope(root, goal_dir, packet_id, args.goal_slug, args.review_mode, scope, required_ids)
    row = {
        "schema": "mobius.packet",
        "packet_id": packet_id,
        "goal_id": goal.get("goal_id", ""),
        "goal_slug": args.goal_slug,
        "review_mode": args.review_mode,
        "stateless": as_bool_cell(True),
        "scope": scope,
        "created_at": now_iso(),
        "packet_json": as_json_cell(envelope),
        "packet_sha256": "",
    }
    row["packet_sha256"] = packet_hash(envelope)
    packet_rows = [*existing_packets, row]
    writes: list[CsvWrite] = [(packet_ledger_path, PACKET_FIELDS, packet_rows)]
    updated_files = ["packets.csv"]
    if args.review_mode == "delta_review":
        try:
            loop_rows = sync_loop_with_plan(goal_dir, commit=False)
            loop_row = next((item for item in loop_rows if item.get("plan_item_id") == target_plan_item_id), {})
            if loop_row.get("status") != "running":
                raise MobiusError(f"delta packet requires running stage: {target_plan_item_id}")
            upsert_loop_state_in_rows(goal_dir, loop_rows, target_plan_item_id, "running", last_packet_id=packet_id)
            writes.append((goal_dir / "loop.csv", LOOP_FIELDS, loop_rows))
            updated_files.append("loop.csv")
        except MobiusError as exc:
            json_print(command_result("packet-create", ok=False, goal_dir=goal_dir, errors=[str(exc)], next_required_action="fix_loop_state"))
            return 2
    verdict = derive_verdict(goal_dir)
    writes.append((goal_dir / "verdict.csv", VERDICT_FIELDS, [verdict]))
    updated_files.append("verdict.csv")
    try:
        write_csv_files_atomically(writes)
    except MobiusError as exc:
        json_print(command_result("packet-create", ok=False, goal_dir=goal_dir, errors=[str(exc)], next_required_action="retry_after_storage_error"))
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("packet-create", goal_dir, errors, updated_files=updated_files, data={"packet": envelope, "packet_sha256": row["packet_sha256"]}))
        return 2
    json_print(
        loop_command_result(
            "packet-create",
            root,
            args.session_id,
            args.goal_slug,
            updated_files=updated_files,
            data={"packet": envelope, "packet_sha256": row["packet_sha256"], "verdict": verdict},
        )
    )
    return 0


def active_required_acceptance_ids(goal_dir: Path) -> list[str]:
    return [
        item.get("id", "")
        for item in read_csv_rows(goal_dir / "acceptance.csv")
        if item.get("status") != "superseded" and from_bool_cell(item.get("required", ""), True)
    ]


def required_acceptance_ids_for_plan_item(goal_dir: Path, plan_item_id: str) -> list[str]:
    active_acceptance = active_required_acceptance_rows(goal_dir)
    ids = {row.get("id", "") for row in active_acceptance if row.get("plan_item_id") == plan_item_id}
    for plan in active_required_plan_items(goal_dir):
        if plan.get("id") != plan_item_id:
            continue
        try:
            linked = from_json_cell(plan.get("acceptance_ids_json", ""), [])
        except json.JSONDecodeError:
            linked = []
        if isinstance(linked, list):
            ordered = [str(item) for item in linked if str(item) in ids]
            if ordered:
                return ordered
    return sorted(ids)


def active_required_plan_items(goal_dir: Path) -> list[dict[str, str]]:
    return [
        item
        for item in read_csv_rows(goal_dir / "plan.csv")
        if item.get("contract_status") != "superseded" and from_bool_cell(item.get("required", ""), True)
    ]


def active_required_acceptance_rows(goal_dir: Path) -> list[dict[str, str]]:
    return [
        item
        for item in read_csv_rows(goal_dir / "acceptance.csv")
        if item.get("status") != "superseded" and from_bool_cell(item.get("required", ""), True)
    ]


def supporting_evidence_by_acceptance(goal_dir: Path) -> dict[str, list[str]]:
    support: dict[str, list[str]] = {}
    for record in read_csv_rows(goal_dir / "evidence.csv"):
        evidence_id = record.get("id", "")
        if not evidence_id:
            continue
        try:
            ids = from_json_cell(record.get("supports_json", ""), [])
        except json.JSONDecodeError:
            continue
        if not isinstance(ids, list):
            continue
        for acceptance_id in ids:
            support.setdefault(str(acceptance_id), []).append(evidence_id)
    return support


def dedupe_strings(items: list[str]) -> list[str]:
    seen: set[str] = set()
    out: list[str] = []
    for item in items:
        if item not in seen:
            seen.add(item)
            out.append(item)
    return out


def review_gate_policy(
    review_mode: str,
    explicit_policy: Any | None = None,
    contract_gate: Any | None = None,
) -> dict[str, Any]:
    if review_mode not in {"delta_review", "exit_review"}:
        raise MobiusError("review_mode must be exit_review or delta_review")
    policy: dict[str, Any] = explicit_policy if isinstance(explicit_policy, dict) else {}
    name = str(policy.get("name") or "").strip() if isinstance(explicit_policy, dict) else str(explicit_policy or "").strip()
    level = policy.get("level")
    if not name and isinstance(contract_gate, dict):
        gate_policy = contract_gate.get("review_policy") or contract_gate.get("review_gate_policy")
        if isinstance(gate_policy, dict):
            name = str(gate_policy.get("name", "")).strip()
        elif isinstance(gate_policy, str):
            name = gate_policy.strip()
    if not name:
        try:
            level_int = int(level)
        except (TypeError, ValueError):
            level_int = 0
        if review_mode == "exit_review":
            name = "exit_strict"
        elif level_int >= 2:
            name = "delta_kimi"
        else:
            name = "delta_light"
    if review_mode == "exit_review":
        name = "exit_strict"
    if name not in REVIEW_POLICY_NAMES:
        raise MobiusError(f"unsupported review policy: {name}")
    if review_mode == "delta_review" and name == "exit_strict":
        raise MobiusError("exit_strict policy is only valid for exit_review")
    if review_mode == "exit_review" and name != "exit_strict":
        raise MobiusError("exit_review requires exit_strict policy")
    if name == "delta_light":
        minimum_level = 1
        required_reviewers = ["codex-subagent"]
    else:
        minimum_level = 2
        required_reviewers = ["codex-subagent", "kimi-code"]
    return {
        "schema": REVIEW_POLICY_SCHEMA,
        "name": name,
        "review_mode": review_mode,
        "minimum_level": minimum_level,
        "required_reviewers": required_reviewers,
        "require_full_coverage": True,
        "require_no_degraded_reviewers": True,
        "require_all_completed_reviewers_pass": True,
    }


def explicit_review_policy(cv_result: dict[str, Any]) -> dict[str, Any] | None:
    input_refs = cv_result.get("input_refs")
    if not isinstance(input_refs, dict):
        return None
    policy = input_refs.get("review_policy")
    return policy if isinstance(policy, dict) else None


def derive_cv_aggregate(
    reviewers: list[Any],
    required_acceptance_ids: list[str],
    review_mode: str,
    policy: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if policy is None:
        policy = review_gate_policy(review_mode)
    valid_reviewers = [item for item in reviewers if isinstance(item, dict)]
    checked: set[str] = set()
    unchecked: set[str] = set(str(item) for item in required_acceptance_ids)
    blocking: list[str] = []
    revisions: list[str] = []
    degraded: list[str] = []
    verdicts: dict[str, str] = {}

    for reviewer in valid_reviewers:
        reviewer_id = str(reviewer.get("reviewer_id", "unknown"))
        verdict = str(reviewer.get("verdict", "unknown"))
        verdicts[reviewer_id] = verdict
        if reviewer.get("status") != "completed":
            degraded.append(reviewer_id)
        for item in reviewer.get("checked_acceptance_ids", []) or []:
            checked.add(str(item))
            unchecked.discard(str(item))
        for item in reviewer.get("unchecked_acceptance_ids", []) or []:
            unchecked.add(str(item))
        blocking.extend(str(item) for item in reviewer.get("blocking_findings", []) or [])
        revisions.extend(str(item) for item in reviewer.get("required_revisions", []) or [])

    all_completed_reviewers_pass = bool(valid_reviewers) and all(
        item.get("status") == "completed" and item.get("verdict") == "pass" for item in valid_reviewers
    )
    completed_pass_reviewers = {
        str(item.get("reviewer_id", ""))
        for item in valid_reviewers
        if item.get("status") == "completed" and item.get("verdict") == "pass"
    }
    required_reviewers = {str(item) for item in policy.get("required_reviewers", [])}
    policy_reviewers_pass = required_reviewers.issubset(completed_pass_reviewers)
    policy_coverage_pass = not unchecked if policy.get("require_full_coverage", True) else True
    policy_degraded_pass = not degraded if policy.get("require_no_degraded_reviewers", True) else True
    policy_all_pass = all_completed_reviewers_pass if policy.get("require_all_completed_reviewers_pass", True) else True
    policy_pass = bool(valid_reviewers) and policy_reviewers_pass and policy_coverage_pass and policy_degraded_pass and policy_all_pass
    if len(valid_reviewers) < len(reviewers):
        agreement = "not_comparable"
    elif any(item.get("status") != "completed" for item in valid_reviewers):
        agreement = "not_comparable"
    else:
        agreement = "agree" if len(set(verdicts.values())) <= 1 else "disagree"
    if any(item.get("verdict") == "blocked" for item in valid_reviewers):
        overall = "blocked"
    elif any(item.get("verdict") == "fail" for item in valid_reviewers) or blocking or revisions:
        overall = "fail"
    elif policy_pass:
        overall = "pass"
    else:
        overall = "unknown"

    return {
        "agreement": agreement,
        "reviewer_verdicts": verdicts,
        "degraded_reviewers": sorted(set(degraded)),
        "checked_acceptance_ids": sorted(checked),
        "unchecked_acceptance_ids": sorted(unchecked),
        "blocking_findings": dedupe_strings(blocking),
        "required_revisions": dedupe_strings(revisions),
        "overall": overall,
    }


def validate_cv_envelope(
    cv_result: dict[str, Any],
    required_acceptance_ids: list[str],
    require_checked_ids: bool = False,
) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    warnings: list[str] = []
    if cv_result.get("schema") != "mobius.cv_result":
        errors.append("schema must be mobius.cv_result")
    if cv_result.get("review_mode") not in {"exit_review", "delta_review"}:
        errors.append("review_mode must be exit_review or delta_review")
    if cv_result.get("stateless") is not True:
        errors.append("stateless must be true")
    reviewers = cv_result.get("reviewers")
    if not isinstance(reviewers, list):
        errors.append("reviewers must be a list")
        reviewers = []
    elif not reviewers:
        errors.append("reviewers must not be empty")
    comparison = cv_result.get("comparison")
    if not isinstance(comparison, dict):
        errors.append("comparison must be an object")
        comparison = {}
    elif not comparison:
        errors.append("comparison must not be empty")
    result = cv_result.get("result")
    if not isinstance(result, dict):
        errors.append("result must be an object")
        result = {}
    overall = result.get("overall")
    if overall not in {"pass", "fail", "unknown", "blocked"}:
        errors.append("result.overall must be pass, fail, unknown, or blocked")
    checked = result.get("checked_acceptance_ids", [])
    unchecked = result.get("unchecked_acceptance_ids", [])
    if not isinstance(checked, list):
        errors.append("result.checked_acceptance_ids must be a list")
        checked = []
    if not isinstance(unchecked, list):
        errors.append("result.unchecked_acceptance_ids must be a list")
        unchecked = []
    if cv_result.get("review_mode") == "exit_review" or require_checked_ids:
        missing = sorted(set(required_acceptance_ids) - {str(item) for item in checked})
        if missing and overall == "pass":
            errors.append("review did not check required acceptance ids: " + ",".join(missing))
    if cv_result.get("review_mode") == "delta_review" and overall == "pass":
        warnings.append("delta_review pass cannot support final acceptance")
    if overall == "pass" and unchecked:
        errors.append("pass result cannot contain unchecked_acceptance_ids")
    degraded = comparison.get("degraded_reviewers", [])
    if not isinstance(degraded, list):
        errors.append("comparison.degraded_reviewers must be a list")
    elif overall == "pass" and degraded:
        errors.append("pass result cannot contain degraded_reviewers")
    level = cv_result.get("level")
    try:
        level_int = int(level)
    except (TypeError, ValueError):
        level_int = 0
    policy = explicit_review_policy(cv_result)
    if overall == "pass" and policy is None:
        errors.append("pass result requires input_refs.review_policy")
    try:
        normalized_policy = review_gate_policy(str(cv_result.get("review_mode", "")), policy or {"level": level_int})
    except MobiusError as exc:
        fallback_mode = str(cv_result.get("review_mode", ""))
        normalized_policy = review_gate_policy(fallback_mode if fallback_mode in {"delta_review", "exit_review"} else "delta_review")
        errors.append(str(exc))
    if policy is not None and policy != normalized_policy:
        errors.append("input_refs.review_policy is not canonical")
    if overall == "pass" and level_int < int(normalized_policy.get("minimum_level", 0)):
        errors.append(f"pass result requires review level >= {normalized_policy.get('minimum_level')}")
    if overall == "pass":
        completed_pass_reviewers = {
            str(item.get("reviewer_id", ""))
            for item in reviewers
            if isinstance(item, dict) and item.get("status") == "completed" and item.get("verdict") == "pass"
        }
        missing_reviewers = sorted(set(normalized_policy.get("required_reviewers", [])) - completed_pass_reviewers)
        if missing_reviewers:
            errors.append("pass result missing required completed reviewers: " + ",".join(missing_reviewers))
    if isinstance(reviewers, list) and isinstance(result, dict) and isinstance(comparison, dict) and reviewers:
        derived = derive_cv_aggregate(reviewers, required_acceptance_ids, str(cv_result.get("review_mode", "")), normalized_policy)
        expected_comparison = {
            "agreement": derived["agreement"],
            "reviewer_verdicts": derived["reviewer_verdicts"],
            "degraded_reviewers": derived["degraded_reviewers"],
        }
        for key, expected_value in expected_comparison.items():
            if comparison.get(key) != expected_value:
                errors.append(f"comparison.{key} does not match reviewer rows")
        for key in ("overall", "checked_acceptance_ids", "unchecked_acceptance_ids", "blocking_findings", "required_revisions"):
            if result.get(key) != derived[key]:
                errors.append(f"result.{key} does not match reviewer rows")
    return errors, warnings


def cv_row_from_envelope(goal_dir: Path, cv_result: dict[str, Any], required_acceptance_ids: list[str] | None = None) -> dict[str, Any]:
    reviewers = strip_transient_raw(cv_result.get("reviewers", []))
    required_ids = [str(item) for item in (required_acceptance_ids or [])]
    policy = explicit_review_policy(cv_result) or review_gate_policy(str(cv_result.get("review_mode", "")), {"level": cv_result.get("level")})
    derived = derive_cv_aggregate(reviewers if isinstance(reviewers, list) else [], required_ids, str(cv_result.get("review_mode", "")), policy)
    comparison = {
        "agreement": derived["agreement"],
        "reviewer_verdicts": derived["reviewer_verdicts"],
        "degraded_reviewers": derived["degraded_reviewers"],
    }
    result = {
        "overall": derived["overall"],
        "checked_acceptance_ids": derived["checked_acceptance_ids"],
        "unchecked_acceptance_ids": derived["unchecked_acceptance_ids"],
        "blocking_findings": derived["blocking_findings"],
        "required_revisions": derived["required_revisions"],
    }
    raw_ref, raw_hash_tail = write_cv_raw_file(goal_dir, cv_result)
    return {
        "schema": "mobius.cv_result",
        "cv_id": cv_result.get("cv_id", ""),
        "goal_id": cv_result.get("goal_id", ""),
        "packet_id": cv_result.get("packet_id", ""),
        "review_mode": cv_result.get("review_mode", ""),
        "level": cv_result.get("level", ""),
        "stateless": as_bool_cell(cv_result.get("stateless") is True),
        "reviewers_json": as_json_cell(reviewers if isinstance(reviewers, list) else []),
        "comparison_json": as_json_cell(comparison if isinstance(comparison, dict) else {}),
        "input_refs_json": as_json_cell(cv_result.get("input_refs", {}) if isinstance(cv_result.get("input_refs", {}), dict) else {}),
        "result_json": as_json_cell(result if isinstance(result, dict) else {}),
        "raw_ref": raw_ref,
        "raw_hash_tail": raw_hash_tail,
        "returned_at": cv_result.get("returned_at") or now_iso(),
    }


def prepare_cv_append(
    goal_dir: Path,
    cv_result: dict[str, Any],
    expected_goal_id: str | None = None,
    required_acceptance_ids: list[str] | None = None,
    require_checked_ids: bool = False,
) -> tuple[str, list[str], dict[str, Any]]:
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    expected = expected_goal_id or goal.get("goal_id", "")
    actual = str(cv_result.get("goal_id", ""))
    if not actual:
        raise MobiusError("goal_id is required")
    if expected and actual != expected:
        raise MobiusError(f"goal_id mismatch: expected {expected}, got {actual}")
    if not str(cv_result.get("packet_id", "")):
        raise MobiusError("packet_id is required")
    required_ids = required_acceptance_ids if required_acceptance_ids is not None else active_required_acceptance_ids(goal_dir)
    errors, warnings = validate_cv_envelope(cv_result, required_ids, require_checked_ids=require_checked_ids)
    if errors:
        raise MobiusError("; ".join(errors))
    cv_id = str(cv_result.get("cv_id", ""))
    if not cv_id:
        raise MobiusError("cv_id is required")
    packet_id = str(cv_result.get("packet_id", ""))
    cv_path = goal_dir / "cv.csv"
    cv_rows = read_csv_rows(cv_path)
    if packet_has_recorded_review(goal_dir, packet_id):
        raise MobiusError(f"packet_id already has a recorded review: {packet_id}")
    if any(row.get("cv_id") == cv_id for row in cv_rows):
        raise MobiusError(f"duplicate cv_id: {cv_id}")
    return cv_id, warnings, cv_row_from_envelope(goal_dir, cv_result, required_ids)


def validate_packet_for_goal(
    goal_dir: Path,
    packet: dict[str, Any],
    expected_review_mode: str,
    expected_acceptance_ids: list[str] | None = None,
) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    if not isinstance(packet, dict):
        return {}, ["packet must be an object"]
    if packet.get("schema") != "mobius.packet":
        errors.append("packet schema must be mobius.packet")
    packet_id = str(packet.get("packet", ""))
    if not packet_id:
        errors.append("packet is required")
    if str(packet.get("goal", "")) != goal_dir.name:
        errors.append(f"packet goal mismatch: expected {goal_dir.name}, got {packet.get('goal', '')}")
    expected_mode = packet_mode(expected_review_mode)
    if packet.get("mode") != expected_mode:
        errors.append(f"packet mode mismatch: expected {expected_mode}, got {packet.get('mode', '')}")
    if expected_review_mode == "delta_review" and not str(packet.get("scope", "")).strip():
        errors.append("delta packet scope is required")
    if expected_review_mode == "exit_review" and packet.get("scope") != "all":
        errors.append("exit packet scope must be all")
    expected_ids = expected_acceptance_ids if expected_acceptance_ids is not None else active_required_acceptance_ids(goal_dir)
    coverage = packet.get("coverage")
    if not isinstance(coverage, dict):
        errors.append("packet coverage must be an object")
        coverage = {}
    elif sorted(str(item) for item in coverage) != sorted(str(item) for item in expected_ids):
        errors.append("packet coverage acceptance ids mismatch")
    else:
        support = supporting_evidence_by_acceptance(goal_dir)
        for acceptance_id in [str(item) for item in expected_ids]:
            value = coverage.get(acceptance_id)
            if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
                errors.append(f"packet coverage.{acceptance_id} must be a string array")
            elif value != support.get(acceptance_id, []):
                errors.append(f"packet coverage.{acceptance_id} mismatch")
    refs = packet.get("refs")
    if not isinstance(refs, dict):
        errors.append("packet refs must be an object")
    else:
        try:
            expected_refs = evidence_refs_for_packet(goal_dir, [str(item) for item in expected_ids])
        except MobiusError as exc:
            errors.append(str(exc))
            expected_refs = {}
        if refs != expected_refs:
            errors.append("packet refs mismatch")
        for evidence_id, ref in refs.items():
            if not isinstance(ref, list) or len(ref) != 3:
                errors.append(f"packet refs.{evidence_id} must be [type,label,h:xxxxxxx]")
                continue
            evidence_type = str(ref[0])
            label = str(ref[1])
            hash_ref = str(ref[2])
            if evidence_type in {"file_ref", "change_set_scope"}:
                label_paths = label.split(",") if evidence_type == "change_set_scope" else [label]
                errors.extend(f"packet refs.{evidence_id} {error}" for error in root_relative_path_errors("label", label_paths))
            if not re.fullmatch(r"h:[0-9a-f]{7}", hash_ref):
                errors.append(f"packet refs.{evidence_id} hash ref must be 7 hex chars")
    ledger = packet.get("ledger")
    if not isinstance(ledger, dict):
        errors.append("packet ledger must be an object")
    else:
        root_ref = str(ledger.get("root", ""))
        if not root_ref or Path(root_ref).is_absolute():
            errors.append("packet ledger.root must be a root-relative path")
        if ledger.get("hash") != goal.get("contract_sha256_tail", ""):
            errors.append("packet ledger.hash mismatch")
    ledger_rows = [row for row in read_csv_rows(goal_dir / "packets.csv") if row.get("packet_id") == packet_id]
    if not ledger_rows:
        errors.append("packet_id is not recorded in packets.csv")
    else:
        try:
            ledger_envelope = packet_envelope_from_row(ledger_rows[-1])
        except MobiusError as exc:
            errors.append(str(exc))
            ledger_envelope = {}
        if ledger_envelope and packet_hash(ledger_envelope) != ledger_rows[-1].get("packet_sha256"):
            errors.append("packets.csv packet hash mismatch")
        if packet_hash(packet) != ledger_rows[-1].get("packet_sha256"):
            errors.append("packet envelope does not match packets.csv")
    return packet, errors


def packet_envelope_from_ledger(goal_dir: Path, packet_id: str) -> dict[str, Any] | None:
    if not packet_id:
        return None
    rows = [row for row in read_csv_rows(goal_dir / "packets.csv") if row.get("packet_id") == packet_id]
    if not rows:
        return None
    return packet_envelope_from_row(rows[-1])


def packet_has_recorded_review(goal_dir: Path, packet_id: str) -> bool:
    return bool(packet_id and any(row.get("packet_id") == packet_id for row in read_csv_rows(goal_dir / "cv.csv")))


def project_root_from_goal_dir(goal_dir: Path) -> Path:
    try:
        return goal_dir.parents[3]
    except IndexError as exc:
        raise MobiusError(f"cannot derive project root from goal dir: {goal_dir}") from exc


def strip_transient_raw(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: strip_transient_raw(item) for key, item in value.items() if not key.startswith("_raw_")}
    if isinstance(value, list):
        return [strip_transient_raw(item) for item in value]
    return value


def write_cv_raw_file(goal_dir: Path, cv_result: dict[str, Any]) -> tuple[str, str]:
    reviewers = cv_result.get("reviewers", [])
    raw_reviewers: list[dict[str, str]] = []
    if isinstance(reviewers, list):
        for reviewer in reviewers:
            if not isinstance(reviewer, dict):
                continue
            raw_text = str(reviewer.get("_raw_text", ""))
            if not raw_text:
                continue
            raw_reviewers.append(
                {
                    "reviewer_id": str(reviewer.get("reviewer_id", "")),
                    "status": str(reviewer.get("status", "")),
                    "verdict": str(reviewer.get("verdict", "")),
                    "raw_text": raw_text,
                }
            )
    if not raw_reviewers:
        return "", ""
    raw_payload = {
        "schema": "mobius.cv_raw_result",
        "cv_id": str(cv_result.get("cv_id", "")),
        "packet_id": str(cv_result.get("packet_id", "")),
        "review_mode": str(cv_result.get("review_mode", "")),
        "reviewers": raw_reviewers,
    }
    text = json.dumps(raw_payload, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    raw_dir = goal_dir / "raw_reviews"
    raw_dir.mkdir(parents=True, exist_ok=True)
    raw_path = raw_dir / f"{str(cv_result.get('cv_id', '') or 'cv')}.json"
    temp_path = write_text_temp(raw_path, text)
    os.replace(temp_path, raw_path)
    digest = sha256_file(raw_path)
    raw_ref = raw_path.relative_to(project_root_from_goal_dir(goal_dir)).as_posix()
    return raw_ref, sha256_tail(digest)


def ensure_review_attempts_file(goal_dir: Path) -> None:
    ensure_csv_file(goal_dir / "review_attempts.csv", REVIEW_ATTEMPT_FIELDS)


def review_attempt_started(goal_dir: Path, packet_id: str, review_mode: str) -> str:
    ensure_review_attempts_file(goal_dir)
    path = goal_dir / "review_attempts.csv"
    rows = read_csv_rows(path)
    attempt_id = f"attempt_{len(rows) + 1:03d}"
    rows.append(
        {
            "schema": "mobius.review_attempt",
            "attempt_id": attempt_id,
            "packet_id": packet_id,
            "review_mode": review_mode,
            "status": "started",
            "started_at": now_iso(),
            "finished_at": "",
            "reviewer_summary_ref": "",
        }
    )
    write_csv_rows(path, REVIEW_ATTEMPT_FIELDS, rows)
    return attempt_id


def review_attempt_finished(goal_dir: Path, attempt_id: str, status: str, reviewer_summary_ref: str = "") -> None:
    validate_state_value("review_attempt", status)
    if status == "started":
        raise MobiusError("review attempt finish status cannot be started")
    ensure_review_attempts_file(goal_dir)
    path = goal_dir / "review_attempts.csv"
    rows = read_csv_rows(path)
    for row in rows:
        if row.get("attempt_id") == attempt_id:
            validate_state_transition("review_attempt", row.get("status", "started"), status)
            row["status"] = status
            row["finished_at"] = now_iso()
            row["reviewer_summary_ref"] = reviewer_summary_ref
            write_csv_rows(path, REVIEW_ATTEMPT_FIELDS, rows)
            return
    raise MobiusError(f"unknown review attempt id: {attempt_id}")


def visible_review_attempts(goal_dir: Path) -> dict[str, list[dict[str, str]]]:
    rows = read_csv_rows(goal_dir / "review_attempts.csv")
    open_attempts: list[dict[str, str]] = []
    interrupted_attempts: list[dict[str, str]] = []
    failed_attempts: list[dict[str, str]] = []
    for row in rows:
        status = row.get("status", "")
        if status == "started" and not row.get("finished_at"):
            open_attempts.append(row)
            interrupted_attempts.append({**row, "status": "interrupted"})
        elif status == "interrupted":
            interrupted_attempts.append(row)
        elif status == "failed":
            failed_attempts.append(row)
    return {
        "open_review_attempts": open_attempts,
        "interrupted_review_attempts": interrupted_attempts,
        "failed_review_attempts": failed_attempts,
    }


def canonical_cv_parts(cv: dict[str, str]) -> tuple[dict[str, Any], dict[str, Any], list[Any], dict[str, Any], list[str]]:
    errors: list[str] = []
    result = from_json_cell(cv.get("result_json", ""), {})
    comparison = from_json_cell(cv.get("comparison_json", ""), {})
    reviewers = from_json_cell(cv.get("reviewers_json", ""), [])
    input_refs = from_json_cell(cv.get("input_refs_json", ""), {})
    if not isinstance(result, dict):
        errors.append("result_json must be an object")
        result = {}
    if not isinstance(comparison, dict):
        errors.append("comparison_json must be an object")
        comparison = {}
    if not isinstance(reviewers, list):
        errors.append("reviewers_json must be a list")
        reviewers = []
    if not isinstance(input_refs, dict):
        errors.append("input_refs_json must be an object")
        input_refs = {}
    if not comparison:
        errors.append("comparison_json is required")
    if not reviewers:
        errors.append("reviewers_json is required")
    return result, comparison, reviewers, input_refs, errors


def derive_verdict(
    goal_dir: Path,
    *,
    plan_rows: list[dict[str, str]] | None = None,
    acceptance_rows: list[dict[str, str]] | None = None,
    evidence_rows: list[dict[str, str]] | None = None,
    cv_rows: list[dict[str, str]] | None = None,
) -> dict[str, Any]:
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    acceptance_path = goal_dir / "acceptance.csv"
    evidence_path = goal_dir / "evidence.csv"
    cv_path = goal_dir / "cv.csv"
    plan_path = goal_dir / "plan.csv"
    plan_rows = plan_rows if plan_rows is not None else read_csv_rows(plan_path)
    acceptance_rows = acceptance_rows if acceptance_rows is not None else read_csv_rows(acceptance_path)
    evidence_rows = evidence_rows if evidence_rows is not None else read_csv_rows(evidence_path)
    cv_rows = cv_rows if cv_rows is not None else read_csv_rows(cv_path)
    evidence_ids = {record.get("id") for record in evidence_rows}
    cv_records = {record.get("cv_id"): record for record in cv_rows}

    required_plan_items = [
        item
        for item in plan_rows
        if item.get("contract_status") != "superseded" and from_bool_cell(item.get("required", ""), True)
    ]
    required_items = [
        item
        for item in acceptance_rows
        if item.get("status") != "superseded" and from_bool_cell(item.get("required", ""), True)
    ]
    required_ids = {item.get("id") for item in required_items}
    unverified_plan: list[str] = []
    unverified_acceptance: list[str] = []
    blocked: list[str] = []
    used_cv_ids: list[str] = []
    used_packet_ids: list[str] = []

    for item in required_plan_items:
        item_id = item.get("id", "")
        if not from_bool_cell(item.get("locked", "")):
            unverified_plan.append(item_id)

    for item in required_items:
        item_id = item.get("id", "")
        if not from_bool_cell(item.get("locked", "")):
            unverified_acceptance.append(item_id)
            continue
        if item.get("status") == "blocked":
            blocked.append(item_id)
            continue
        if item.get("status") != "pass":
            unverified_acceptance.append(item_id)
            continue
        if item.get("verified_by") != "mobius_cv_mcp" or not item.get("verified_at"):
            unverified_acceptance.append(item_id)
            continue
        evidence_ids_for_item = from_json_cell(item.get("evidence_ids_json", ""), [])
        if not evidence_ids_for_item or not set(evidence_ids_for_item).issubset(evidence_ids):
            unverified_acceptance.append(item_id)
            continue
        if not acceptance_evidence_satisfied(item, evidence_rows):
            unverified_acceptance.append(item_id)
            continue
        cv_id = item.get("cv_id", "")
        cv = cv_records.get(cv_id)
        if not cv:
            unverified_acceptance.append(item_id)
            continue
        try:
            result, comparison, _reviewers, input_refs, cv_errors = canonical_cv_parts(cv)
        except json.JSONDecodeError:
            unverified_acceptance.append(item_id)
            continue
        reconstructed_cv = {
            "schema": cv.get("schema", ""),
            "cv_id": cv.get("cv_id", ""),
            "goal_id": cv.get("goal_id", ""),
            "packet_id": cv.get("packet_id", ""),
            "review_mode": cv.get("review_mode", ""),
            "level": cv.get("level", ""),
            "stateless": from_bool_cell(cv.get("stateless", "")),
            "reviewers": _reviewers,
            "comparison": comparison,
            "result": result,
            "input_refs": input_refs,
        }
        envelope_errors, _warnings = validate_cv_envelope(
            reconstructed_cv,
            sorted(str(item) for item in required_ids),
            require_checked_ids=True,
        )
        checked = set(result.get("checked_acceptance_ids", []))
        if cv_errors or envelope_errors:
            unverified_acceptance.append(item_id)
        elif cv.get("review_mode") != "exit_review" or not from_bool_cell(cv.get("stateless", "")):
            unverified_acceptance.append(item_id)
        elif comparison.get("degraded_reviewers"):
            unverified_acceptance.append(item_id)
        elif result.get("overall") != "pass" or result.get("unchecked_acceptance_ids"):
            unverified_acceptance.append(item_id)
        elif not required_ids.issubset(checked):
            unverified_acceptance.append(item_id)
        else:
            used_cv_ids.append(cv_id)
            used_packet_ids.append(cv.get("packet_id", ""))

    if blocked:
        overall = "blocked"
    elif unverified_plan or unverified_acceptance or not required_items:
        overall = "pending"
    else:
        overall = "accepted"

    verdict = {
        "schema": "mobius.verdict",
        "goal_id": goal.get("goal_id", ""),
        "overall": overall,
        "adjudicated_by": "mobius_gate",
        "adjudicated_at": now_iso(),
        "rule": ACCEPTANCE_RULE,
        "derived_from_json": as_json_cell(
            {
                "plan_sha256": csv_rows_sha256(PLAN_FIELDS, plan_rows),
                "acceptance_sha256": csv_rows_sha256(ACCEPTANCE_FIELDS, acceptance_rows),
                "evidence_log_sha256": csv_rows_sha256(EVIDENCE_FIELDS, evidence_rows),
                "cv_log_sha256": csv_rows_sha256(CV_FIELDS, cv_rows),
                "cv_ids": sorted(set(used_cv_ids)),
                "packet_ids": sorted(set(item for item in used_packet_ids if item)),
            }
        ),
        "unverified_plan_item_ids_json": as_json_cell(unverified_plan),
        "unverified_acceptance_ids_json": as_json_cell(unverified_acceptance),
        "blocked_acceptance_ids_json": as_json_cell(blocked),
    }
    return verdict


def compute_verdict(goal_dir: Path) -> dict[str, Any]:
    verdict = derive_verdict(goal_dir)
    write_single_csv(goal_dir / "verdict.csv", VERDICT_FIELDS, verdict)
    return verdict


def cmd_verdict_compute(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    terminal_result = terminal_command_result("verdict-compute", goal_dir)
    if terminal_result is not None:
        json_print(terminal_result)
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("verdict-compute", goal_dir, errors))
        return 2
    verdict = compute_verdict(goal_dir)
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("verdict-compute", goal_dir, errors, updated_files=["verdict.csv"], data={"verdict": verdict}))
        return 2
    json_print(
        command_result(
            "verdict-compute",
            goal_dir=goal_dir,
            updated_files=["verdict.csv"],
            gate=verdict["overall"],
            next_required_action="completion_allowed" if verdict["overall"] == "accepted" else "continue_loop",
            data={"verdict": verdict},
        )
    )
    return 0


def active_acceptance_by_id(goal_dir: Path) -> dict[str, dict[str, str]]:
    return {item.get("id", ""): item for item in active_required_acceptance_rows(goal_dir) if item.get("id")}


def required_evidence_items(row: dict[str, str]) -> list[dict[str, Any]]:
    try:
        parsed = from_json_cell(row.get("evidence_required_json", ""), [])
    except json.JSONDecodeError:
        return []
    if not isinstance(parsed, list):
        return []
    return [item for item in parsed if isinstance(item, dict)]


def evidence_artifact(row: dict[str, str]) -> dict[str, Any]:
    try:
        parsed = from_json_cell(row.get("artifact_json", ""), None)
    except json.JSONDecodeError:
        return {}
    return parsed if isinstance(parsed, dict) else {}


def evidence_matches_required_item(evidence: dict[str, str], required: dict[str, Any]) -> bool:
    required_type = str(required.get("type", "")).strip()
    if required_type and evidence.get("type") != required_type:
        return False
    artifact = evidence_artifact(evidence)
    if required_type in STRUCTURED_PROOF_TYPES and not artifact:
        return False
    if required_type in PATH_PROOF_TYPES and not artifact.get("path"):
        return False
    required_name = str(required.get("name", "")).strip()
    if required_name:
        haystack = " ".join(
            str(value)
            for value in (
                evidence.get("summary", ""),
                artifact.get("name", ""),
                artifact.get("command", ""),
                artifact.get("path", ""),
                artifact.get("purpose", ""),
            )
            if value is not None
        )
        if required_name not in haystack:
            return False
    if "exit_code" in required and artifact.get("exit_code") != required.get("exit_code"):
        return False
    return True


def acceptance_evidence_satisfied(acceptance: dict[str, str], evidence_rows: list[dict[str, str]]) -> bool:
    required_items = required_evidence_items(acceptance)
    if not required_items:
        return False
    acceptance_id = acceptance.get("id", "")
    supporting: list[dict[str, str]] = []
    for evidence in evidence_rows:
        try:
            supports = from_json_cell(evidence.get("supports_json", ""), [])
        except json.JSONDecodeError:
            continue
        if isinstance(supports, list) and acceptance_id in [str(item) for item in supports]:
            supporting.append(evidence)
    if not supporting:
        return False
    for required in required_items:
        if not any(evidence_matches_required_item(evidence, required) for evidence in supporting):
            return False
    return True


def validate_evidence_record_against_acceptance(goal_dir: Path, record: dict[str, str]) -> list[str]:
    errors: list[str] = []
    try:
        supports = from_json_cell(record.get("supports_json", ""), [])
    except json.JSONDecodeError as exc:
        return [f"supports_json: invalid JSON cell: {exc.msg}"]
    if not isinstance(supports, list) or not supports:
        return ["supports_json must be a non-empty list"]
    active_acceptance = active_acceptance_by_id(goal_dir)
    for acceptance_id in [str(item) for item in supports]:
        acceptance = active_acceptance.get(acceptance_id)
        if acceptance is None:
            errors.append(f"supports_json references unknown acceptance id: {acceptance_id}")
            continue
        required_items = required_evidence_items(acceptance)
        matching_items = [item for item in required_items if str(item.get("type", "")).strip() == record.get("type")]
        if matching_items and not any(evidence_matches_required_item(record, item) for item in matching_items):
            errors.append(f"evidence does not satisfy any required proof for acceptance id: {acceptance_id}")
    return errors


def sync_loop_with_plan(goal_dir: Path, *, commit: bool = True) -> list[dict[str, str]]:
    ensure_loop_file(goal_dir)
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    existing = read_csv_rows(goal_dir / "loop.csv")
    by_plan = {row.get("plan_item_id", ""): row for row in existing if row.get("plan_item_id")}
    timestamp = now_iso()
    changed = False
    for item in active_required_plan_items(goal_dir):
        plan_item_id = item.get("id", "")
        if not plan_item_id or plan_item_id in by_plan:
            continue
        row = {
            "schema": "mobius.loop_state",
            "goal_id": goal.get("goal_id", ""),
            "plan_item_id": plan_item_id,
            "status": "pending",
            "attempt": "0",
            "last_packet_id": "",
            "last_cv_id": "",
            "blocking_findings_json": as_json_cell([]),
            "updated_at": timestamp,
        }
        existing.append(row)
        by_plan[plan_item_id] = row
        changed = True
    if changed and commit:
        write_csv_rows(goal_dir / "loop.csv", LOOP_FIELDS, existing)
    return existing


def upsert_loop_state_in_rows(
    goal_dir: Path,
    rows: list[dict[str, str]],
    plan_item_id: str,
    status: str,
    last_packet_id: str | None = None,
    last_cv_id: str | None = None,
    blocking_findings: list[str] | None = None,
    increment_attempt: bool = False,
    attempt: int | None = None,
) -> dict[str, str]:
    validate_state_value("loop", status)
    if not plan_item_id:
        raise MobiusError("plan_item_id is required")
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    row = next((item for item in rows if item.get("plan_item_id") == plan_item_id), None)
    if row is None:
        row = {
            "schema": "mobius.loop_state",
            "goal_id": goal.get("goal_id", ""),
            "plan_item_id": plan_item_id,
            "attempt": "0",
        }
        rows.append(row)
    current_status = row.get("status") or "pending"
    validate_state_transition("loop", current_status, status)
    current_attempt = int(row.get("attempt") or "0")
    if attempt is not None:
        row["attempt"] = str(attempt)
    elif increment_attempt:
        row["attempt"] = str(current_attempt + 1)
    else:
        row["attempt"] = str(current_attempt)
    row["schema"] = "mobius.loop_state"
    row["goal_id"] = goal.get("goal_id", row.get("goal_id", ""))
    row["status"] = status
    if last_packet_id is not None:
        row["last_packet_id"] = last_packet_id
    if last_cv_id is not None:
        row["last_cv_id"] = last_cv_id
    if blocking_findings is not None:
        row["blocking_findings_json"] = as_json_cell(blocking_findings)
    elif not row.get("blocking_findings_json"):
        row["blocking_findings_json"] = as_json_cell([])
    row["updated_at"] = now_iso()
    return row


def upsert_loop_state(
    goal_dir: Path,
    plan_item_id: str,
    status: str,
    last_packet_id: str | None = None,
    last_cv_id: str | None = None,
    blocking_findings: list[str] | None = None,
    increment_attempt: bool = False,
    attempt: int | None = None,
) -> dict[str, str]:
    rows = sync_loop_with_plan(goal_dir, commit=False)
    row = upsert_loop_state_in_rows(
        goal_dir,
        rows,
        plan_item_id,
        status,
        last_packet_id=last_packet_id,
        last_cv_id=last_cv_id,
        blocking_findings=blocking_findings,
        increment_attempt=increment_attempt,
        attempt=attempt,
    )
    write_csv_rows(goal_dir / "loop.csv", LOOP_FIELDS, rows)
    return row


def loop_next_plan_item(goal_dir: Path, *, commit: bool = True) -> str:
    rows = {row.get("plan_item_id", ""): row for row in sync_loop_with_plan(goal_dir, commit=commit)}
    for item in active_required_plan_items(goal_dir):
        item_id = item.get("id", "")
        if not item_id:
            continue
        state = rows.get(item_id, {})
        if state.get("status") == "passed":
            continue
        try:
            dependencies = from_json_cell(item.get("depends_on_json", ""), [])
        except json.JSONDecodeError:
            dependencies = []
        if isinstance(dependencies, list) and any(rows.get(str(dep), {}).get("status") != "passed" for dep in dependencies):
            continue
        return item_id
    return ""


def latest_packet_id(goal_dir: Path, review_mode: str) -> str:
    rows = [row for row in read_csv_rows(goal_dir / "packets.csv") if row.get("review_mode") == review_mode]
    return rows[-1].get("packet_id", "") if rows else ""


def cv_id_for_packet(goal_dir: Path, packet_id: str, review_mode: str) -> str:
    if not packet_id:
        return ""
    rows = [
        row
        for row in read_csv_rows(goal_dir / "cv.csv")
        if row.get("packet_id") == packet_id and row.get("review_mode") == review_mode
    ]
    return rows[-1].get("cv_id", "") if rows else ""


def loop_action_for_plan_item(goal_dir: Path, row: dict[str, str]) -> dict[str, Any]:
    plan_item_id = row.get("plan_item_id", "")
    status = row.get("status") or "pending"
    target_ids = required_acceptance_ids_for_plan_item(goal_dir, plan_item_id) if plan_item_id else []
    attempt = safe_int(row.get("attempt"), 0)
    attempt_limit = stage_attempt_limit(goal_dir, plan_item_id) if plan_item_id else 0
    packet_id = row.get("last_packet_id", "")
    base = {
        "next_plan_item_id": plan_item_id,
        "packet_id": packet_id,
        "review_mode": "delta_review" if packet_id else "",
        "repair_from_cv_id": "",
        "repair_findings": [],
        "missing_acceptance_ids": [],
        "attempt": attempt,
        "attempt_limit": attempt_limit,
    }
    if status == "pending":
        return {**base, "loop_gate": "ready", "next_required_action": "start_next_stage", "review_mode": ""}
    if status == "blocked":
        findings = from_json_cell(row.get("blocking_findings_json", ""), [])
        blocked_action = "repair_budget_exhausted" if any(str(item).startswith("repair_budget_exhausted:") for item in findings) else "goal_blocked"
        return {
            **base,
            "loop_gate": "blocked",
            "next_required_action": blocked_action,
            "repair_from_cv_id": row.get("last_cv_id", ""),
            "repair_findings": findings,
        }
    if status != "running":
        return {**base, "loop_gate": status or "unknown", "next_required_action": "needs_contract_change"}

    last_cv_id = row.get("last_cv_id", "")
    if last_cv_id:
        result = cv_result_by_id(goal_dir, last_cv_id)
        comparison = cv_comparison_by_id(goal_dir, last_cv_id)
        missing = target_unsatisfied_evidence(goal_dir, target_ids)
        if result.get("overall") == "pass":
            next_action = missing_evidence_action(goal_dir, target_ids) if missing else "create_new_packet"
            return {
                **base,
                "loop_gate": "running",
                "next_required_action": next_action,
                "repair_from_cv_id": last_cv_id,
                "repair_findings": from_json_cell(row.get("blocking_findings_json", ""), []),
                "missing_acceptance_ids": missing,
            }
        classification = classify_delta_review(
            goal_dir,
            plan_item_id,
            target_ids,
            result,
            comparison,
            attempt=attempt,
        )
        return {
            **base,
            "loop_gate": classification["gate"],
            "next_required_action": classification["next_required_action"],
            "repair_from_cv_id": last_cv_id if classification["next_required_action"] else "",
            "repair_findings": classification["blocking_findings"],
            "missing_acceptance_ids": missing,
            "attempt_limit": classification["attempt_limit"],
        }

    last_packet_id = row.get("last_packet_id", "")
    if last_packet_id:
        if packet_has_recorded_review(goal_dir, last_packet_id):
            return {**base, "loop_gate": "running", "next_required_action": "create_new_packet"}
        return {**base, "loop_gate": "running", "next_required_action": "record_delta_review"}

    missing = target_unsatisfied_evidence(goal_dir, target_ids)
    if missing:
        return {
            **base,
            "loop_gate": "running",
            "next_required_action": missing_evidence_action(goal_dir, target_ids),
            "missing_acceptance_ids": missing,
        }
    return {**base, "loop_gate": "running", "next_required_action": "create_delta_packet", "review_mode": "delta_review"}


def ledger_audit_data(root: Path, session_id: str, goal_slug: str) -> dict[str, Any]:
    goal_dir = load_goal_dir(root, session_id, goal_slug)
    derived = derive_verdict(goal_dir)
    terminal = derived["overall"] if derived["overall"] in TERMINAL_VERDICTS else ""
    latest_exit_packet = latest_packet_id(goal_dir, "exit_review")
    exit_cv_id = cv_id_for_packet(goal_dir, latest_exit_packet, "exit_review")
    unverified_acceptance_ids = from_json_cell(str(derived.get("unverified_acceptance_ids_json", "")), [])
    attempt_visibility = visible_review_attempts(goal_dir)
    interrupted_exit_attempts = [
        row
        for row in attempt_visibility["interrupted_review_attempts"]
        if row.get("review_mode") == "exit_review" and (not latest_exit_packet or row.get("packet_id") == latest_exit_packet)
    ]
    loop_rows = sync_loop_with_plan(goal_dir, commit=False)
    next_plan_item_id = ""
    loop_context: dict[str, Any] = {}
    packet_id = ""
    review_mode = ""
    if terminal:
        loop_gate = terminal
    elif active_required_unlocked_ids(goal_dir):
        loop_gate = "unlocked"
    else:
        try:
            next_plan_item_id = loop_next_plan_item(goal_dir, commit=False)
        except MobiusError:
            next_plan_item_id = ""
        if next_plan_item_id:
            loop_row = next((row for row in loop_rows if row.get("plan_item_id") == next_plan_item_id), {})
            loop_context = loop_action_for_plan_item(goal_dir, loop_row)
            loop_gate = str(loop_context.get("loop_gate", "ready"))
            packet_id = str(loop_context.get("packet_id", ""))
            review_mode = str(loop_context.get("review_mode", ""))
        elif loop_rows and all(row.get("status") == "passed" for row in loop_rows):
            loop_gate = "awaiting_exit_review"
        elif loop_rows:
            loop_gate = ",".join(sorted({row.get("status", "") for row in loop_rows if row.get("status")}))
        else:
            loop_gate = "ready"
    if terminal:
        next_action = {
            "accepted": "completion_allowed",
            "blocked": "goal_blocked",
        }[terminal]
    elif active_required_unlocked_ids(goal_dir):
        next_action = "needs_contract_change"
    elif next_plan_item_id:
        next_action = str(loop_context.get("next_required_action", "start_next_stage"))
    elif not latest_exit_packet:
        next_action = "create_exit_packet"
        review_mode = "exit_review"
    elif interrupted_exit_attempts and not exit_cv_id:
        next_action = "retry_review"
        packet_id = latest_exit_packet
        review_mode = "exit_review"
    elif not exit_cv_id:
        next_action = "record_exit_review"
        packet_id = latest_exit_packet
        review_mode = "exit_review"
    elif unverified_acceptance_ids:
        unverified_ids = [str(item) for item in unverified_acceptance_ids]
        exit_result = cv_result_by_id(goal_dir, exit_cv_id)
        exit_comparison = cv_comparison_by_id(goal_dir, exit_cv_id)
        if (
            exit_result.get("overall") in {"fail", "unknown"}
            or exit_result.get("unchecked_acceptance_ids")
            or exit_comparison.get("degraded_reviewers")
        ):
            next_action = "create_new_packet"
            review_mode = "exit_review"
        else:
            missing = target_unsatisfied_evidence(goal_dir, unverified_ids)
            next_action = missing_evidence_action(goal_dir, unverified_ids)
    else:
        next_action = "continue_loop"
    audit = {
        "schema": "mobius.ledger_audit",
        "goal_dir": str(goal_dir),
        "loop_gate": loop_gate,
        "terminal_verdict": terminal,
        "exit_cv_id": exit_cv_id,
        "packet_id": packet_id,
        "review_mode": review_mode,
        "unverified_acceptance_ids": unverified_acceptance_ids,
        "next_required_action": next_action,
        "next_plan_item_id": next_plan_item_id,
        **loop_context,
        **attempt_visibility,
        "derived_verdict": derived,
    }
    audit["loop"] = loop_decision(audit)
    audit["next_required_action"] = audit["loop"]["next_required_action"]
    audit["packet_id"] = audit["loop"]["packet_id"]
    audit["review_mode"] = audit["loop"]["review_mode"]
    return audit


def loop_decision(audit: dict[str, Any]) -> dict[str, Any]:
    next_action = str(audit.get("next_required_action", ""))
    next_plan_item_id = str(audit.get("next_plan_item_id", ""))
    packet_id = str(audit.get("packet_id", ""))
    review_mode = str(audit.get("review_mode", ""))
    terminal = str(audit.get("terminal_verdict", ""))
    continuing_actions = {
        "start_next_stage": "loop-start-stage",
        "repair_stage": "loop-start-stage",
        "record_missing_evidence": "evidence-add",
        "run_missing_command_evidence": "evidence-add",
        "create_delta_packet": "packet-create",
        "create_exit_packet": "packet-create",
        "record_delta_review": "packet-read",
        "record_exit_review": "packet-read",
        "retry_review": "packet-read",
        "create_new_packet": "packet-create",
    }
    stop_reasons = {
        "completion_allowed": "no_runnable_action",
        "goal_blocked": "review_blocked",
        "needs_contract_change": "contract_change_required",
        "repair_budget_exhausted": "repair_budget_exhausted",
        "continue_loop": "no_runnable_action",
    }
    next_command = continuing_actions.get(next_action, "")
    if next_action in {"start_next_stage", "repair_stage"} and next_plan_item_id:
        next_command = f"loop-start-stage --plan-item-id {next_plan_item_id}"
    elif next_action == "create_delta_packet" and next_plan_item_id:
        acceptance_args = " ".join(
            f"--acceptance-id {shlex.quote(item)}"
            for item in required_acceptance_ids_for_plan_item(Path(str(audit.get("goal_dir", ""))), next_plan_item_id)
        ) if audit.get("goal_dir") else ""
        next_command = f"packet-create --review-mode delta_review {acceptance_args}".strip()
    elif next_action == "create_exit_packet":
        next_command = "packet-create --review-mode exit_review"
    elif next_action in {"record_delta_review", "record_exit_review", "retry_review"} and packet_id:
        mode = review_mode or ("delta_review" if next_action == "record_delta_review" else "exit_review")
        next_command = f"packet-read --review-mode {mode} --packet-id {packet_id}"
    elif next_action == "create_new_packet" and (review_mode == "delta_review" or next_plan_item_id):
        acceptance_args = " ".join(
            f"--acceptance-id {shlex.quote(item)}"
            for item in required_acceptance_ids_for_plan_item(Path(str(audit.get("goal_dir", ""))), next_plan_item_id)
        ) if audit.get("goal_dir") else ""
        next_command = f"packet-create --review-mode delta_review {acceptance_args}".strip()
    elif next_action == "create_new_packet" and review_mode == "exit_review":
        next_command = "packet-create --review-mode exit_review"
    elif next_action in {"record_missing_evidence", "run_missing_command_evidence"}:
        next_command = "evidence-add"
    return {
        "schema": "mobius.loop",
        "mode": "full_plan",
        "agent_must_continue": next_action in continuing_actions,
        "agent_must_stop": next_action not in continuing_actions,
        "next_required_action": next_action,
        "next_command": next_command,
        "next_plan_item_id": next_plan_item_id,
        "packet_id": packet_id if next_action in {"record_delta_review", "record_exit_review", "retry_review"} else "",
        "review_mode": review_mode,
        "repair_from_cv_id": str(audit.get("repair_from_cv_id", "")),
        "repair_findings": list(audit.get("repair_findings", []) or []),
        "missing_acceptance_ids": list(audit.get("missing_acceptance_ids", []) or []),
        "attempt": safe_int(audit.get("attempt"), 0),
        "attempt_limit": safe_int(audit.get("attempt_limit"), 0),
        "terminal_verdict": terminal,
        "stop_reason": "" if next_action in continuing_actions else stop_reasons.get(next_action, "no_runnable_action"),
    }


def stage_contract_for_plan_item(goal_dir: Path, plan_item_id: str) -> dict[str, Any]:
    plan = next((item for item in active_required_plan_items(goal_dir) if item.get("id") == plan_item_id), None)
    if plan is None:
        raise MobiusError(f"unknown active required plan item: {plan_item_id}")
    linked_acceptance_ids = required_acceptance_ids_for_plan_item(goal_dir, plan_item_id)
    acceptance_rows = [
        row
        for row in active_required_acceptance_rows(goal_dir)
        if row.get("id", "") in set(linked_acceptance_ids)
    ]
    acceptance_by_id = {row.get("id", ""): row for row in acceptance_rows}
    ordered_acceptance = [acceptance_by_id[item] for item in linked_acceptance_ids if item in acceptance_by_id]

    def parsed(field: str, default: Any) -> Any:
        return from_json_cell(plan.get(field, ""), default)

    def parsed_acceptance(row: dict[str, str]) -> dict[str, Any]:
        return {
            "id": row.get("id", ""),
            "plan_item_id": row.get("plan_item_id", ""),
            "requirement": row.get("requirement", ""),
            "observable_outcome": row.get("observable_outcome", ""),
            "evidence_required": from_json_cell(row.get("evidence_required_json", ""), []),
            "verifier": from_json_cell(row.get("verifier_json", ""), []),
            "review_focus": from_json_cell(row.get("review_focus_json", ""), []),
            "required": from_bool_cell(row.get("required", ""), True),
            "status": row.get("status", ""),
        }

    return {
        "plan_item_id": plan_item_id,
        "title": plan.get("title", ""),
        "description": plan.get("description", ""),
        "depends_on": parsed("depends_on_json", []),
        "scope": parsed("scope_json", {}),
        "work": parsed("work_json", {}),
        "gate": parsed("gate_json", {}),
        "recovery": parsed("recovery_json", {}),
        "budget": parsed("budget_json", {}),
        "acceptance": [parsed_acceptance(row) for row in ordered_acceptance],
    }


def validate_delta_targets(goal_dir: Path, target_plan_item_id: str, target_acceptance_ids: list[str]) -> None:
    plan_ids = {item.get("id", "") for item in active_required_plan_items(goal_dir)}
    if target_plan_item_id not in plan_ids:
        raise MobiusError(f"target_plan_item_id is not an active required plan item: {target_plan_item_id}")
    if not target_acceptance_ids:
        raise MobiusError("target_acceptance_ids is required")
    expected_ids = required_acceptance_ids_for_plan_item(goal_dir, target_plan_item_id)
    if sorted(target_acceptance_ids) != sorted(expected_ids):
        raise MobiusError(f"target_acceptance_ids must match linked required acceptance ids for {target_plan_item_id}")
    acceptance = active_acceptance_by_id(goal_dir)
    for acceptance_id in target_acceptance_ids:
        row = acceptance.get(acceptance_id)
        if row is None:
            raise MobiusError(f"target_acceptance_id is not active required: {acceptance_id}")
        if row.get("plan_item_id") != target_plan_item_id:
            raise MobiusError(f"target_acceptance_id {acceptance_id} does not belong to plan item {target_plan_item_id}")


def acceptance_rows_from_exit_review(goal_dir: Path, cv_result: dict[str, Any]) -> tuple[list[str], list[dict[str, str]]]:
    result = cv_result.get("result", {}) if isinstance(cv_result.get("result"), dict) else {}
    comparison = cv_result.get("comparison", {}) if isinstance(cv_result.get("comparison"), dict) else {}
    if (
        cv_result.get("review_mode") != "exit_review"
        or cv_result.get("stateless") is not True
        or comparison.get("degraded_reviewers")
    ):
        return [], read_csv_rows(goal_dir / "acceptance.csv")
    overall = result.get("overall")
    if overall not in {"pass", "blocked"} or result.get("unchecked_acceptance_ids"):
        return [], read_csv_rows(goal_dir / "acceptance.csv")
    checked = {str(item) for item in result.get("checked_acceptance_ids", [])}
    support = supporting_evidence_by_acceptance(goal_dir)
    evidence_rows = read_csv_rows(goal_dir / "evidence.csv")
    path = goal_dir / "acceptance.csv"
    rows = read_csv_rows(path)
    updated: list[str] = []
    timestamp = now_iso()
    for row in rows:
        acceptance_id = row.get("id", "")
        if row.get("status") == "superseded" or not from_bool_cell(row.get("required", ""), True):
            continue
        if acceptance_id not in checked:
            continue
        evidence_ids = support.get(acceptance_id, [])
        try:
            current_ids = from_json_cell(row.get("evidence_ids_json", ""), [])
        except json.JSONDecodeError:
            current_ids = []
        if not isinstance(current_ids, list):
            current_ids = []
        if overall == "pass":
            if not evidence_ids or not acceptance_evidence_satisfied(row, evidence_rows):
                continue
            validate_state_transition("acceptance", row.get("status", "unknown"), "pass")
            row["status"] = "pass"
        else:
            validate_state_transition("acceptance", row.get("status", "unknown"), "blocked")
            row["status"] = "blocked"
        row["evidence_ids_json"] = as_json_cell(sorted({*(str(item) for item in current_ids), *evidence_ids}))
        row["cv_id"] = str(cv_result.get("cv_id", ""))
        row["verified_by"] = "mobius_cv_mcp"
        row["verified_at"] = timestamp
        updated.append(acceptance_id)
    return updated, rows


def terminal_status_writes(root: Path, session_id: str, goal_slug: str, verdict_overall: str) -> tuple[list[CsvWrite], list[str]]:
    terminal_map = {"accepted": "accepted", "blocked": "blocked"}
    status = terminal_map.get(verdict_overall)
    if not status:
        return [], []
    goal_dir = load_goal_dir(root, session_id, goal_slug)
    writes: list[CsvWrite] = []
    updated_files: list[str] = []
    goal_path = goal_dir / "goal.csv"
    goal_rows = read_csv_rows(goal_path)
    goal = goal_rows[0] if goal_rows else {}
    if goal_rows and goal.get("status") != status:
        validate_state_transition("goal", goal.get("status", "active"), status)
        goal["status"] = status
        goal["updated_at"] = now_iso()
        writes.append((goal_path, GOAL_FIELDS, goal_rows))
        updated_files.append("goal.csv")
    run_path = run_dir(root, session_id) / "run.csv"
    run_rows = read_csv_rows(run_path)
    run = run_rows[0] if run_rows else {}
    goals = from_json_cell(run.get("goals_json", ""), []) if run else []
    changed = False
    for item in goals:
        if item.get("path") == goal_slug:
            if item.get("status") != status:
                validate_state_transition("goal", str(item.get("status", "active")), status)
                item["status"] = status
                changed = True
    if changed:
        run["goals_json"] = as_json_cell(goals)
        writes.append((run_path, RUN_FIELDS, run_rows))
        updated_files.append("run.csv")
    return writes, updated_files


def target_unsatisfied_evidence(goal_dir: Path, target_acceptance_ids: list[str]) -> list[str]:
    acceptance = active_acceptance_by_id(goal_dir)
    evidence_rows = read_csv_rows(goal_dir / "evidence.csv")
    return [
        item
        for item in target_acceptance_ids
        if not acceptance_evidence_satisfied(acceptance.get(item, {}), evidence_rows)
    ]


def safe_int(value: Any, default: int = 0) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def stage_attempt_limit(goal_dir: Path, plan_item_id: str) -> int:
    plan = next((item for item in active_required_plan_items(goal_dir) if item.get("id") == plan_item_id), {})
    try:
        budget = from_json_cell(plan.get("budget_json", ""), {})
    except json.JSONDecodeError:
        budget = {}
    if not isinstance(budget, dict):
        return 0
    max_stage_attempts = safe_int(budget.get("max_stage_attempts"), 0)
    if max_stage_attempts > 0:
        return max_stage_attempts
    retry_limit = safe_int(budget.get("retry_limit"), 0)
    return retry_limit + 1 if retry_limit > 0 else 0


def missing_evidence_types(goal_dir: Path, target_acceptance_ids: list[str]) -> list[str]:
    acceptance = active_acceptance_by_id(goal_dir)
    evidence_rows = read_csv_rows(goal_dir / "evidence.csv")
    missing_types: list[str] = []
    for acceptance_id in target_acceptance_ids:
        row = acceptance.get(acceptance_id, {})
        if acceptance_evidence_satisfied(row, evidence_rows):
            continue
        for item in required_evidence_items(row):
            evidence_type = str(item.get("type", "")).strip()
            if evidence_type:
                missing_types.append(evidence_type)
    return sorted(set(missing_types))


def missing_evidence_action(goal_dir: Path, target_acceptance_ids: list[str]) -> str:
    types = set(missing_evidence_types(goal_dir, target_acceptance_ids))
    if types & {"command_result", "test_result"}:
        return "run_missing_command_evidence"
    return "record_missing_evidence"


def earliest_plan_item_for_acceptance_ids(goal_dir: Path, acceptance_ids: list[str]) -> str:
    target_ids = {str(item) for item in acceptance_ids if str(item)}
    if not target_ids:
        return ""
    acceptance = active_acceptance_by_id(goal_dir)
    affected_plan_ids = {
        row.get("plan_item_id", "")
        for acceptance_id, row in acceptance.items()
        if acceptance_id in target_ids and row.get("plan_item_id", "")
    }
    for plan in active_required_plan_items(goal_dir):
        plan_id = plan.get("id", "")
        if plan_id in affected_plan_ids:
            return plan_id
    return ""


def cv_result_by_id(goal_dir: Path, cv_id: str) -> dict[str, Any]:
    if not cv_id:
        return {}
    row = next((item for item in read_csv_rows(goal_dir / "cv.csv") if item.get("cv_id") == cv_id), None)
    if row is None:
        return {}
    try:
        result = from_json_cell(row.get("result_json", ""), {})
    except json.JSONDecodeError:
        return {}
    return result if isinstance(result, dict) else {}


def cv_overall_by_id(goal_dir: Path, cv_id: str) -> str:
    return str(cv_result_by_id(goal_dir, cv_id).get("overall", ""))


def cv_comparison_by_id(goal_dir: Path, cv_id: str) -> dict[str, Any]:
    if not cv_id:
        return {}
    row = next((item for item in read_csv_rows(goal_dir / "cv.csv") if item.get("cv_id") == cv_id), None)
    if row is None:
        return {}
    try:
        comparison = from_json_cell(row.get("comparison_json", ""), {})
    except json.JSONDecodeError:
        return {}
    return comparison if isinstance(comparison, dict) else {}


def classify_delta_review(
    goal_dir: Path,
    plan_item_id: str,
    target_acceptance_ids: list[str],
    result: dict[str, Any],
    comparison: dict[str, Any] | None = None,
    *,
    attempt: int,
) -> dict[str, Any]:
    comparison = comparison if isinstance(comparison, dict) else {}
    unsatisfied_evidence = target_unsatisfied_evidence(goal_dir, target_acceptance_ids)
    blocking = list(result.get("blocking_findings", []) or []) + list(result.get("required_revisions", []) or [])
    if unsatisfied_evidence:
        blocking.append("unsatisfied evidence_required_json for target acceptance ids: " + ",".join(unsatisfied_evidence))
    attempt_limit = stage_attempt_limit(goal_dir, plan_item_id)

    if result.get("overall") == "pass" and not unsatisfied_evidence:
        return {
            "status": "passed",
            "gate": "passed",
            "next_required_action": "",
            "blocking_findings": [str(item) for item in blocking],
            "attempt": attempt,
            "attempt_limit": attempt_limit,
        }

    if result.get("overall") == "blocked":
        return {
            "status": "blocked",
            "gate": "blocked",
            "next_required_action": "goal_blocked",
            "blocking_findings": [str(item) for item in blocking],
            "attempt": attempt,
            "attempt_limit": attempt_limit,
        }

    if comparison.get("degraded_reviewers") or result.get("unchecked_acceptance_ids") or result.get("overall") == "unknown":
        return {
            "status": "running",
            "gate": "running",
            "next_required_action": "create_new_packet",
            "blocking_findings": [str(item) for item in blocking],
            "attempt": attempt,
            "attempt_limit": attempt_limit,
        }

    if attempt_limit and attempt >= attempt_limit:
        blocking.append(f"repair_budget_exhausted: attempt {attempt} reached max_stage_attempts {attempt_limit}")
        return {
            "status": "blocked",
            "gate": "blocked",
            "next_required_action": "repair_budget_exhausted",
            "blocking_findings": [str(item) for item in blocking],
            "attempt": attempt,
            "attempt_limit": attempt_limit,
        }

    if unsatisfied_evidence:
        next_required_action = missing_evidence_action(goal_dir, target_acceptance_ids)
    else:
        next_required_action = "repair_stage"

    return {
        "status": "running",
        "gate": "running",
        "next_required_action": next_required_action,
        "blocking_findings": [str(item) for item in blocking],
        "attempt": attempt,
        "attempt_limit": attempt_limit,
    }


def record_cv_result(
    root: Path,
    session_id: str,
    goal_slug: str,
    cv_result: dict[str, Any],
    review_mode: str,
    target_plan_item_id: str | None = None,
    target_acceptance_ids: list[str] | None = None,
) -> dict[str, Any]:
    goal_dir = load_goal_dir(root, session_id, goal_slug)
    require_nonterminal_goal(goal_dir, "record_cv_result")
    errors = validate_contract_dir(goal_dir)
    if errors:
        raise MobiusError(contract_error_text(errors))
    require_locked_contract(goal_dir)
    goal = read_single_csv(goal_dir / "goal.csv") or {}
    if cv_result.get("review_mode") != review_mode:
        raise MobiusError(f"review_mode mismatch: expected {review_mode}, got {cv_result.get('review_mode', '')}")
    result = cv_result.get("result", {}) if isinstance(cv_result.get("result"), dict) else {}
    updated_files = ["cv.csv"]
    blocking = list(result.get("blocking_findings", []) or []) + list(result.get("required_revisions", []) or [])

    if review_mode == "delta_review":
        target_ids = [str(item) for item in (target_acceptance_ids or [])]
        if not target_ids:
            target_ids = [str(item) for item in result.get("checked_acceptance_ids", []) or []]
        if target_plan_item_id is None:
            raise MobiusError("target_plan_item_id is required for delta_review")
        validate_delta_targets(goal_dir, target_plan_item_id, target_ids)
        packet = packet_envelope_from_ledger(goal_dir, str(cv_result.get("packet_id", "")))
        if packet is None:
            raise MobiusError("packet_id is not recorded in packets.csv")
        _packet, packet_errors = validate_packet_for_goal(goal_dir, packet, review_mode, target_ids)
        if packet_errors:
            raise MobiusError("; ".join(packet_errors))
        cv_id, _warnings, cv_row = prepare_cv_append(
            goal_dir,
            cv_result,
            expected_goal_id=goal.get("goal_id", ""),
            required_acceptance_ids=target_ids,
            require_checked_ids=True,
        )
        loop_rows = sync_loop_with_plan(goal_dir, commit=False)
        current_loop = next((row for row in loop_rows if row.get("plan_item_id") == target_plan_item_id), {})
        classification = classify_delta_review(
            goal_dir,
            target_plan_item_id,
            target_ids,
            result,
            cv_result.get("comparison", {}) if isinstance(cv_result.get("comparison"), dict) else {},
            attempt=safe_int(current_loop.get("attempt"), 0),
        )
        loop_status = str(classification["status"])
        gate = str(classification["gate"])
        next_required_action = str(classification["next_required_action"])
        blocking = list(classification["blocking_findings"])
        upsert_loop_state_in_rows(
            goal_dir,
            loop_rows,
            target_plan_item_id,
            loop_status,
            last_packet_id=str(cv_result.get("packet_id", "")),
            last_cv_id=cv_id,
            blocking_findings=[str(item) for item in blocking],
        )
        cv_rows = [*read_csv_rows(goal_dir / "cv.csv"), cv_row]
        verdict = derive_verdict(goal_dir, cv_rows=cv_rows)
        write_csv_files_atomically(
            [
                (goal_dir / "cv.csv", CV_FIELDS, cv_rows),
                (goal_dir / "loop.csv", LOOP_FIELDS, loop_rows),
                (goal_dir / "verdict.csv", VERDICT_FIELDS, [verdict]),
            ]
        )
        updated_files.append("loop.csv")
        updated_files.append("verdict.csv")
        errors = validate_contract_dir(goal_dir)
        if errors:
            raise MobiusError(contract_error_text(errors))
        post_audit = ledger_audit_data(root, session_id, goal_slug)
        post_loop = post_audit["loop"]
        return {
            "schema": "mobius.cv_recorded_result",
            "ok": True,
            "persisted": True,
            "goal_id": goal.get("goal_id", ""),
            "packet_id": cv_result.get("packet_id", ""),
            "cv_id": cv_id,
            "review_mode": review_mode,
            "gate": post_audit["terminal_verdict"] or post_audit["loop_gate"],
            "updated_files": updated_files,
            "next_required_action": post_loop["next_required_action"],
            "blocking_findings": [str(item) for item in blocking],
            "errors": [],
            "verdict": verdict,
            "loop": post_loop,
        }

    required_ids = active_required_acceptance_ids(goal_dir)
    packet = packet_envelope_from_ledger(goal_dir, str(cv_result.get("packet_id", "")))
    if packet is None:
        raise MobiusError("packet_id is not recorded in packets.csv")
    _packet, packet_errors = validate_packet_for_goal(goal_dir, packet, review_mode, required_ids)
    if packet_errors:
        raise MobiusError("; ".join(packet_errors))
    cv_id, _warnings, cv_row = prepare_cv_append(
        goal_dir,
        cv_result,
        expected_goal_id=goal.get("goal_id", ""),
        required_acceptance_ids=required_ids,
        require_checked_ids=True,
    )
    cv_rows = [*read_csv_rows(goal_dir / "cv.csv"), cv_row]
    updated_acceptance, acceptance_rows = acceptance_rows_from_exit_review(goal_dir, cv_result)
    updated_files.append("acceptance.csv")
    repair_plan_item_id = ""
    loop_rows: list[dict[str, str]] | None = None
    comparison = cv_result.get("comparison", {}) if isinstance(cv_result.get("comparison"), dict) else {}
    retry_exit_review = bool(comparison.get("degraded_reviewers") or result.get("unchecked_acceptance_ids"))
    if result.get("overall") == "fail" and not retry_exit_review:
        affected_acceptance_ids = [str(item) for item in result.get("checked_acceptance_ids", []) or []]
        repair_plan_item_id = earliest_plan_item_for_acceptance_ids(goal_dir, affected_acceptance_ids)
        if repair_plan_item_id:
            if not blocking:
                blocking.append("exit_review failed checked acceptance ids: " + ",".join(affected_acceptance_ids))
            loop_rows = sync_loop_with_plan(goal_dir, commit=False)
            current_loop = next((row for row in loop_rows if row.get("plan_item_id") == repair_plan_item_id), {})
            classification = classify_delta_review(
                goal_dir,
                repair_plan_item_id,
                affected_acceptance_ids,
                result,
                comparison,
                attempt=safe_int(current_loop.get("attempt"), 0),
            )
            loop_status = str(classification["status"])
            blocking = list(classification["blocking_findings"])
            if current_loop.get("status") == "passed" and loop_status == "blocked":
                upsert_loop_state_in_rows(
                    goal_dir,
                    loop_rows,
                    repair_plan_item_id,
                    "running",
                    last_packet_id=str(cv_result.get("packet_id", "")),
                    last_cv_id=cv_id,
                    blocking_findings=[str(item) for item in blocking],
                )
            upsert_loop_state_in_rows(
                goal_dir,
                loop_rows,
                repair_plan_item_id,
                loop_status,
                last_packet_id=str(cv_result.get("packet_id", "")),
                last_cv_id=cv_id,
                blocking_findings=[str(item) for item in blocking],
            )
            updated_files.append("loop.csv")
    verdict = derive_verdict(goal_dir, acceptance_rows=acceptance_rows, cv_rows=cv_rows)
    updated_files.append("verdict.csv")
    terminal_writes, terminal_updated_files = ([], [])
    if result.get("overall") != "fail":
        terminal_writes, terminal_updated_files = terminal_status_writes(root, session_id, goal_slug, verdict["overall"])
    writes: list[CsvWrite] = [
        (goal_dir / "cv.csv", CV_FIELDS, cv_rows),
        (goal_dir / "acceptance.csv", ACCEPTANCE_FIELDS, acceptance_rows),
        (goal_dir / "verdict.csv", VERDICT_FIELDS, [verdict]),
        *terminal_writes,
    ]
    if loop_rows is not None:
        writes.append((goal_dir / "loop.csv", LOOP_FIELDS, loop_rows))
    write_csv_files_atomically(writes)
    updated_files.extend(terminal_updated_files)
    errors = validate_contract_dir(goal_dir)
    if errors:
        raise MobiusError(contract_error_text(errors))
    if result.get("unchecked_acceptance_ids"):
        blocking.append("unchecked acceptance ids: " + ",".join(str(item) for item in result.get("unchecked_acceptance_ids", [])))
    post_audit = ledger_audit_data(root, session_id, goal_slug)
    post_loop = post_audit["loop"]
    return {
        "schema": "mobius.cv_recorded_result",
        "ok": True,
        "persisted": True,
        "goal_id": goal.get("goal_id", ""),
        "packet_id": cv_result.get("packet_id", ""),
        "cv_id": cv_id,
        "review_mode": review_mode,
        "gate": post_audit["terminal_verdict"] or post_audit["loop_gate"],
        "updated_files": updated_files,
        "next_required_action": post_loop["next_required_action"],
        "blocking_findings": [str(item) for item in blocking],
        "errors": [],
        "updated_acceptance_ids": updated_acceptance,
        "verdict": verdict,
        "loop": post_loop,
    }


def cmd_loop_status(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    terminal = terminal_verdict(goal_dir)
    if terminal:
        json_print(loop_command_result("loop-status", root, args.session_id, args.goal_slug, data={"rows": read_csv_rows(goal_dir / "loop.csv"), "next_plan_item_id": ""}))
        return 0
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("loop-status", goal_dir, errors))
        return 2
    locked_result = locked_contract_command_result("loop-status", goal_dir)
    if locked_result is not None:
        json_print(locked_result)
        return 2
    rows = sync_loop_with_plan(goal_dir)
    next_item = loop_next_plan_item(goal_dir)
    json_print(
        loop_command_result(
            "loop-status",
            root,
            args.session_id,
            args.goal_slug,
            data={"rows": rows, "next_plan_item_id": next_item},
        )
    )
    return 0


def cmd_ledger_audit(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    if not goal_dir.exists():
        json_print(command_result("ledger-audit", ok=False, errors=[f"goal not found: {args.goal_slug}"], next_required_action="select_goal"))
        return 2
    json_print(
        loop_command_result(
            "ledger-audit",
            root,
            args.session_id,
            args.goal_slug,
        )
    )
    return 0


def cmd_packet_read(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("packet-read", goal_dir, errors))
        return 2
    locked_result = locked_contract_command_result("packet-read", goal_dir)
    if locked_result is not None:
        json_print(locked_result)
        return 2
    packet_id = args.packet_id or latest_packet_id(goal_dir, args.review_mode)
    if not packet_id:
        json_print(
            loop_command_result(
                "packet-read",
                root,
                args.session_id,
                args.goal_slug,
                ok=False,
                errors=[f"no packet found for review_mode: {args.review_mode}"],
            )
        )
        return 2
    packet = packet_envelope_from_ledger(goal_dir, packet_id)
    if packet is None:
        json_print(
            loop_command_result(
                "packet-read",
                root,
                args.session_id,
                args.goal_slug,
                ok=False,
                errors=[f"packet_id is not recorded in packets.csv: {packet_id}"],
            )
        )
        return 2
    expected_ids = packet_required_acceptance_ids(packet) if args.review_mode == "delta_review" else active_required_acceptance_ids(goal_dir)
    _packet, packet_errors = validate_packet_for_goal(goal_dir, packet, args.review_mode, expected_ids)
    if packet_errors:
        json_print(command_result("packet-read", ok=False, goal_dir=goal_dir, errors=packet_errors, next_required_action="fix_packet_scope"))
        return 2
    reviewed = packet_has_recorded_review(goal_dir, packet_id)
    target_plan_item_id = str(packet.get("scope", "")) if args.review_mode == "delta_review" else ""
    json_print(
        loop_command_result(
            "packet-read",
            root,
            args.session_id,
            args.goal_slug,
            data={
                "packet": packet,
                "packet_sha256": packet_hash(packet),
                "review_mode": args.review_mode,
                "review_allowed": not reviewed,
                "target_plan_item_id": target_plan_item_id,
                "required_acceptance_ids": expected_ids,
            },
        )
    )
    return 0


def cmd_loop_start_stage(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    terminal_result = terminal_command_result("loop-start-stage", goal_dir)
    if terminal_result is not None:
        json_print(terminal_result)
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("loop-start-stage", goal_dir, errors))
        return 2
    locked_result = locked_contract_command_result("loop-start-stage", goal_dir)
    if locked_result is not None:
        json_print(locked_result)
        return 2
    try:
        if args.status not in PUBLIC_LOOP_START_STATUSES:
            raise MobiusError("loop-start-stage can only write running")
        expected_next = loop_next_plan_item(goal_dir)
        if expected_next != args.plan_item_id:
            raise MobiusError(f"plan item is not the next runnable stage: {args.plan_item_id}")
        row = upsert_loop_state(
            goal_dir,
            args.plan_item_id,
            args.status,
            last_packet_id="",
            last_cv_id="",
            blocking_findings=[],
            increment_attempt=True,
        )
        stage_contract = stage_contract_for_plan_item(goal_dir, args.plan_item_id)
        verdict = compute_verdict(goal_dir)
    except MobiusError as exc:
        json_print(command_result("loop-start-stage", ok=False, goal_dir=goal_dir, errors=[str(exc)]))
        return 2
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("loop-start-stage", goal_dir, errors, updated_files=["loop.csv"], data={"row": row}))
        return 2
    json_print(
        loop_command_result(
            "loop-start-stage",
            root,
            args.session_id,
            args.goal_slug,
            updated_files=["loop.csv", "verdict.csv"],
            data={"row": row, "stage_contract": stage_contract, "verdict": verdict},
        )
    )
    return 0


def cmd_continue(args: argparse.Namespace) -> int:
    root = project_root(args)
    goal_dir = load_goal_dir(root, args.session_id, args.goal_slug)
    terminal = terminal_verdict(goal_dir)
    if terminal:
        json_print(loop_command_result("continue", root, args.session_id, args.goal_slug, data={"next_plan_item_id": ""}))
        return 0
    errors = validate_contract_dir(goal_dir)
    if errors:
        json_print(command_contract_error("continue", goal_dir, errors))
        return 2
    locked_result = locked_contract_command_result("continue", goal_dir)
    if locked_result is not None:
        json_print(locked_result)
        return 2
    json_print(
        loop_command_result(
            "continue",
            root,
            args.session_id,
            args.goal_slug,
        )
    )
    return 0


def cmd_status(args: argparse.Namespace) -> int:
    root = project_root(args)
    mobius_dir = root / ".mobius"
    if not mobius_dir.exists():
        json_print(command_result("status", ok=False, errors=["not initialized"], next_required_action="init_run"))
        return 1
    goals: list[dict[str, Any]] = []
    runs_dir = mobius_dir / "runs"
    if runs_dir.exists():
        for run_path in sorted(runs_dir.iterdir()):
            if not run_path.is_dir() or not run_path.name.startswith("codex-session-"):
                continue
            session_id = run_path.name.removeprefix("codex-session-")
            for goal_dir in sorted(path for path in run_path.iterdir() if path.is_dir() and (path / "goal.csv").exists()):
                state = read_single_csv(goal_dir / "goal.csv") or {}
                try:
                    audit = ledger_audit_data(root, session_id, goal_dir.name)
                except Exception:
                    audit = {"next_required_action": "ledger_audit_failed", "terminal_verdict": "", "loop_gate": "unknown"}
                goals.append(
                    {
                        "session_id": session_id,
                        "goal_slug": goal_dir.name,
                        "goal_id": state.get("goal_id", ""),
                        "status": state.get("status", ""),
                        "terminal_verdict": audit.get("terminal_verdict", ""),
                        "loop_gate": audit.get("loop_gate", ""),
                        "next_required_action": audit.get("next_required_action", ""),
                    }
                )
    active_goals = [goal for goal in goals if goal.get("status") in {"planning", "active"}]
    terminal_goals = [goal for goal in goals if goal.get("status") in {"accepted", "blocked"}]
    next_action = "continue_active_goal" if active_goals else "create_or_select_goal"
    json_print(
        command_result(
            "status",
            next_required_action=next_action,
            data={"mobius_dir": str(mobius_dir), "active_goals": active_goals, "terminal_goals": terminal_goals, "goals": goals},
        )
    )
    return 0


def plugin_root() -> Path:
    return Path(__file__).resolve().parents[1]


def cmd_hook_health(args: argparse.Namespace) -> int:
    root = plugin_root()
    hooks_file = root / "hooks" / "hooks.json"
    script = root / "scripts" / "mobius.py"
    events: list[str] = []
    errors: list[str] = []
    if not hooks_file.exists():
        errors.append("hooks/hooks.json is missing")
    else:
        try:
            hooks_data = json.loads(hooks_file.read_text(encoding="utf-8"))
            events = sorted((hooks_data.get("hooks") or {}).keys())
        except json.JSONDecodeError as exc:
            errors.append(f"hooks/hooks.json is invalid JSON: {exc.msg}")
    if events != ["PreToolUse", "Stop"]:
        errors.append("expected hook events are PreToolUse and Stop")
    if not script.exists():
        errors.append("scripts/mobius.py is missing")
    if shutil.which("python3") is None:
        errors.append("python3 is not on PATH")
    location = "installed_cache" if "/.codex/plugins/cache/" in str(root) else "source"
    ok = not errors
    json_print(
        command_result(
            "hook-health",
            ok=ok,
            errors=errors,
            gate="ready" if ok else "blocked",
            next_required_action="review_changed_hooks_with_/hooks" if ok else "repair_plugin_install",
            data={
                "plugin_root": str(root),
                "run_location": location,
                "hooks_file": str(hooks_file),
                "hook_file_present": hooks_file.exists(),
                "expected_hook_events": ["PreToolUse", "Stop"],
                "detected_hook_events": events,
                "launcher_can_dispatch": script.exists() and shutil.which("python3") is not None,
                "trust_review_warning": "Codex hook trust is external state; review changed Mobius hooks with /hooks in a new thread after reinstall.",
            },
        )
    )
    return 0 if ok else 2


def read_stdin_text() -> str:
    if sys.stdin.isatty():
        return ""
    return sys.stdin.read()


def hook_payload() -> dict[str, Any]:
    text = read_stdin_text()
    if not text.strip():
        return {}
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        return {"_raw": text}
    return parsed if isinstance(parsed, dict) else {"_value": parsed}


def collect_strings(value: Any) -> list[str]:
    if isinstance(value, str):
        return [value]
    if isinstance(value, dict):
        strings: list[str] = []
        for item in value.values():
            strings.extend(collect_strings(item))
        return strings
    if isinstance(value, list):
        strings: list[str] = []
        for item in value:
            strings.extend(collect_strings(item))
        return strings
    return []


def collect_command_values(value: Any, key: str = "") -> list[Any]:
    command_keys = {
        "command",
        "cmd",
        "argv",
        "args",
        "arguments",
        "tool_input",
        "toolInput",
        "input",
    }
    values: list[Any] = []
    if key in command_keys and isinstance(value, (str, list, dict)):
        values.append(value)
    if isinstance(value, dict):
        for child_key, child_value in value.items():
            values.extend(collect_command_values(child_value, child_key))
    elif isinstance(value, list):
        for item in value:
            values.extend(collect_command_values(item, key))
    return values


def command_tokens(value: Any) -> list[str]:
    if isinstance(value, str):
        try:
            return shlex.split(value)
        except ValueError:
            return value.split()
    if isinstance(value, list):
        tokens: list[str] = []
        for item in value:
            if isinstance(item, str):
                tokens.append(item)
        return tokens
    if isinstance(value, dict):
        tokens: list[str] = []
        for key in ("command", "cmd"):
            if isinstance(value.get(key), str):
                tokens.extend(command_tokens(value[key]))
        for key in ("argv", "args", "arguments"):
            if isinstance(value.get(key), list):
                tokens.extend(command_tokens(value[key]))
        return tokens
    return []


def hook_project_root(args: argparse.Namespace, payload: dict[str, Any]) -> Path:
    for key in ("project_root", "projectRoot", "workspace_root", "workspaceRoot", "cwd"):
        value = payload.get(key)
        if isinstance(value, str) and value:
            return Path(value).expanduser().resolve()
    arg_value = getattr(args, "project_root", None)
    if isinstance(arg_value, str) and arg_value:
        return Path(arg_value).expanduser().resolve()
    return Path.cwd().resolve()


def first_nested_string(value: Any, keys: set[str]) -> str | None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in keys and isinstance(child, str) and child:
                return child
        for child in value.values():
            found = first_nested_string(child, keys)
            if found:
                return found
    elif isinstance(value, list):
        for child in value:
            found = first_nested_string(child, keys)
            if found:
                return found
    return None


def nested_strings_for_keys(value: Any, keys: set[str]) -> list[str]:
    matches: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            if key in keys and isinstance(child, str) and child:
                matches.append(child)
            matches.extend(nested_strings_for_keys(child, keys))
    elif isinstance(value, list):
        for child in value:
            matches.extend(nested_strings_for_keys(child, keys))
    return matches


def payload_session_id(payload: dict[str, Any]) -> str:
    value = first_nested_string(payload, {"session_id", "codex_session_id", "sessionId", "codexSessionId"})
    if value:
        return value
    return os.environ.get("CODEX_SESSION_ID", "")


def hook_explicit_target(payload: dict[str, Any]) -> tuple[str | None, str | None, str | None]:
    session_id = payload_session_id(payload) or None
    goal_slug = first_nested_string(payload, {"goal_slug", "goalSlug", "goal-slug"})
    goal_id = first_nested_string(payload, {"goal_id", "goalId"})
    return session_id, goal_slug, goal_id


def command_token_sets(payload: dict[str, Any]) -> list[list[str]]:
    return [tokens for tokens in (command_tokens(value) for value in collect_command_values(payload)) if tokens]


def hook_target(payload: dict[str, Any]) -> tuple[str | None, str | None, str | None]:
    return hook_explicit_target(payload)


def hook_path_candidates(payload: dict[str, Any]) -> list[str]:
    path_keys = {
        "path",
        "paths",
        "file",
        "files",
        "file_path",
        "filePath",
        "filepath",
        "target_path",
        "targetPath",
        "target_file",
        "targetFile",
    }
    values: list[str] = []

    def walk(value: Any, key: str = "") -> None:
        if isinstance(value, str):
            if key in path_keys:
                values.append(value)
            return
        if isinstance(value, dict):
            for child_key, child_value in value.items():
                walk(child_value, child_key)
        elif isinstance(value, list):
            for child in value:
                walk(child, key)

    walk(payload)
    for tokens in command_token_sets(payload):
        for token in tokens:
            if ".mobius/" in token or ".mobius\\\\" in token:
                values.append(token)
    return values


def clean_relative_mobius_prefix(prefix: str) -> str:
    prefix = prefix.replace("\\", "/")
    for delimiter in ("'", '"', "=", ">", "<", "|", "&", ";", "(", ")", "{", "}", "[", "]"):
        if delimiter in prefix:
            prefix = prefix.rsplit(delimiter, 1)[-1]
    return prefix.rstrip("/")


def mobius_path_binding(value: str, default_root: Path) -> dict[str, Any] | None:
    normalized = value.replace("\\", "/")
    marker = "/.mobius/runs/"
    if marker in normalized:
        marker_index = normalized.find(marker)
        prefix = normalized[:marker_index]
        first_slash = prefix.find("/")
        root_text = prefix[first_slash:] if first_slash >= 0 else prefix
        root = Path(root_text or "/").expanduser().resolve()
        rest = normalized[marker_index + len(marker) :]
    else:
        relative_marker = ".mobius/runs/"
        marker_index = normalized.find(relative_marker)
        if marker_index < 0:
            return None
        prefix = clean_relative_mobius_prefix(normalized[:marker_index])
        root = (default_root / prefix).resolve() if prefix not in {"", "."} else default_root.resolve()
        rest = normalized[marker_index + len(relative_marker) :]

    parts = [part for part in rest.split("/") if part]
    if len(parts) < 2:
        return None
    run_name, goal_slug = parts[0], parts[1]
    if not run_name.startswith("codex-session-"):
        return None
    if len(parts) == 2 and goal_slug == "run.csv":
        session_id = run_name.removeprefix("codex-session-")
        run_path = root / ".mobius" / "runs" / run_name
        return {
            "root": root,
            "goal_dir": run_path,
            "session_id": session_id,
            "goal_slug": "",
            "filename": "run.csv",
        }
    session_id = run_name.removeprefix("codex-session-")
    filename = re.split(r"[^A-Za-z0-9._-]", parts[2], maxsplit=1)[0] if len(parts) > 2 else ""
    goal_dir = root / ".mobius" / "runs" / run_name / goal_slug
    return {
        "root": root,
        "goal_dir": goal_dir,
        "session_id": session_id,
        "goal_slug": goal_slug,
        "filename": filename,
    }


def path_mentions_file(value: str, filename: str) -> bool:
    normalized = value.replace("\\", "/")
    return bool(re.search(rf"(?:^|/){re.escape(filename)}(?=$|[^A-Za-z0-9._-])", normalized))


def protected_ledger_candidate(value: str) -> bool:
    normalized = value.replace("\\", "/")
    protected = (
        "run.csv",
        "goal.csv",
        "plan.csv",
        "acceptance.csv",
        "evidence.csv",
        "packets.csv",
        "cv.csv",
        "loop.csv",
        "review_attempts.csv",
        "verdict.csv",
    )
    return ".mobius/" in normalized and any(path_mentions_file(normalized, filename) for filename in protected)


def command_redirects_to_protected_ledger(tokens: list[str]) -> bool:
    redirection = re.compile(r"^(?:(?:\d*|&)(?:>>?|>\||<>))(.*)$")
    for index, token in enumerate(tokens):
        match = redirection.match(token)
        if not match:
            continue
        target = match.group(1) or (tokens[index + 1] if index + 1 < len(tokens) else "")
        if protected_ledger_candidate(target):
            return True
    return False


def command_reads_protected_ledger(tokens: list[str]) -> bool:
    if not tokens or not any(protected_ledger_candidate(token) for token in tokens):
        return False
    if command_redirects_to_protected_ledger(tokens):
        return False
    if any(re.search(r"[|&;<>()[\]{}]", token) for token in tokens[1:]):
        return False
    shell_control = {"|", "||", "&&", ";", "&", "(", ")", "{", "}"}
    if any(token in shell_control for token in tokens):
        return False
    command = Path(tokens[0]).name
    read_commands = {
        "cat",
        "head",
        "tail",
        "wc",
        "sha1sum",
        "sha256sum",
        "shasum",
        "stat",
        "file",
        "grep",
        "rg",
        "sed",
        "cut",
        "less",
        "more",
        "nl",
    }
    if command == "sed" and any(token == "-i" or token.startswith("-i") or token == "--in-place" or token.startswith("--in-place=") for token in tokens[1:]):
        return False
    if command == "git":
        return len(tokens) >= 2 and tokens[1] == "check-ignore"
    return command in read_commands


def command_touches_protected_ledger(tokens: list[str]) -> bool:
    return bool(tokens and any(protected_ledger_candidate(token) for token in tokens))


def structured_write_tool(payload: dict[str, Any]) -> bool:
    names = " ".join(nested_strings_for_keys(payload, {"tool_name", "toolName", "tool"})).lower()
    if any(marker in names for marker in ("write", "edit", "multiedit", "apply_patch")):
        return True
    return not command_token_sets(payload) and any(protected_ledger_candidate(candidate) for candidate in hook_path_candidates(payload))


def hook_block(message: str) -> int:
    print(message, file=sys.stderr)
    return 2


def hook_pre_tool_use(args: argparse.Namespace, payload: dict[str, Any]) -> int:
    root = hook_project_root(args, payload)

    candidates = hook_path_candidates(payload)
    if not candidates:
        return 0
    token_sets = command_token_sets(payload)
    unknown_protected_commands = [
        tokens for tokens in token_sets if command_touches_protected_ledger(tokens) and not command_reads_protected_ledger(tokens)
    ]
    if not structured_write_tool(payload) and not unknown_protected_commands:
        return 0
    for candidate in candidates:
        if ".mobius/" not in candidate.replace("\\", "/"):
            continue
        binding = mobius_path_binding(candidate, root)
        if binding is None:
            continue
        filename = str(binding["filename"])
        path = Path(binding["goal_dir"]) / filename if filename else Path(binding["goal_dir"])
        if filename == "verdict.csv":
            return hook_block(f"mobius:state-write-blocked:{path}: verdict.csv is derived state; use Mobius recorded review or verdict-compute")
        if filename:
            return hook_block(f"mobius:state-write-blocked:{path}: {filename} is protected state; use Mobius CLI commands")
    return 0


def accepted_verdict_exists(root: Path, session_id: str | None = None, goal_slug: str | None = None, goal_id: str | None = None) -> bool:
    verdicts: list[Path]
    if not goal_slug and not goal_id:
        return False
    if session_id and goal_slug:
        verdicts = [run_dir(root, session_id) / goal_slug / "verdict.csv"]
    elif session_id:
        verdicts = list((run_dir(root, session_id)).glob("*/verdict.csv"))
    elif goal_id:
        verdicts = list((root / ".mobius" / "runs").glob("codex-session-*/*/verdict.csv"))
    else:
        return False
    if goal_id:
        filtered: list[Path] = []
        for verdict in verdicts:
            row = read_single_csv(verdict)
            if row and row.get("goal_id") == goal_id:
                filtered.append(verdict)
        verdicts = filtered
    if goal_id and len(verdicts) != 1:
        return False
    for verdict in verdicts:
        row = read_single_csv(verdict)
        if row and row.get("overall") == "accepted":
            return True
    return False


def hook_final_text(payload: dict[str, Any]) -> str:
    keys = {
        "final_response",
        "finalResponse",
        "assistant_response",
        "assistantResponse",
        "message",
        "text",
        "content",
        "output",
    }
    strings = nested_strings_for_keys(payload, keys)
    if strings:
        return "\n".join(strings)
    return "\n".join(collect_strings(payload))


def text_mentions_target(text: str, goal_slug: str | None, goal_id: str | None) -> bool:
    normalized = text.lower()
    return bool((goal_slug and goal_slug.lower() in normalized) or (goal_id and goal_id.lower() in normalized))


def hook_stop(args: argparse.Namespace, payload: dict[str, Any]) -> int:
    root = hook_project_root(args, payload)
    if not (root / ".mobius").exists():
        return 0
    text = hook_final_text(payload)
    normalized_text = text.lower()
    completion_claim = any(
        marker in normalized_text
        for marker in (
            "accepted",
            "已完成",
            "完成了",
            "目标已完成",
            "done",
            "complete",
            "completed",
        )
    )
    session_id, goal_slug, goal_id = hook_target(payload)
    if not completion_claim or not (goal_slug or goal_id) or not text_mentions_target(text, goal_slug, goal_id):
        return 0
    if not accepted_verdict_exists(root, session_id, goal_slug, goal_id):
        print("mobius:completion-blocked: no accepted verdict.csv found for claimed goal", file=sys.stderr)
        return 2
    return 0


def cmd_hook(args: argparse.Namespace) -> int:
    payload = hook_payload()
    if args.action == "pre-tool-use":
        return hook_pre_tool_use(args, payload)
    if args.action == "stop":
        return hook_stop(args, payload)
    print(f"mobius:unknown-hook:{args.action}", file=sys.stderr)
    return 2


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="mobius", description="Mobius local CSV ledger utilities")
    parser.add_argument("--project-root", default=".", help="Project root that owns .mobius")
    sub = parser.add_subparsers(dest="command", required=True)

    init = sub.add_parser("init")
    init.add_argument("--session-id", required=True)
    init.set_defaults(func=cmd_init)

    goal = sub.add_parser("goal-start")
    goal.add_argument("--session-id", required=True)
    goal.add_argument("--slug", required=True)
    goal.add_argument("--title", required=True)
    goal.add_argument("--user-goal", required=True)
    goal.add_argument("--latest-user-request")
    goal.add_argument("--non-goal", action="append")
    goal.set_defaults(func=cmd_goal_start)

    contract_stage = sub.add_parser("contract-add-stage")
    contract_stage.add_argument("--session-id", required=True)
    contract_stage.add_argument("--goal-slug", required=True)
    contract_stage.add_argument("--id", required=True)
    contract_stage.add_argument("--title", required=True)
    contract_stage.add_argument("--description", required=True)
    contract_stage.add_argument("--depends-on-json")
    contract_stage.add_argument("--scope-json")
    contract_stage.add_argument("--work-json", required=True)
    contract_stage.add_argument("--gate-json")
    contract_stage.add_argument("--recovery-json")
    contract_stage.add_argument("--budget-json")
    contract_stage.add_argument("--acceptance-json", required=True)
    contract_stage.add_argument("--contract-defaults", choices=["none", "local"], default="none")
    contract_stage.add_argument("--revision", default="1")
    contract_stage.add_argument("--optional", action="store_true")
    contract_stage.set_defaults(func=cmd_contract_add_stage)

    contract_supersede = sub.add_parser("contract-supersede-stage")
    contract_supersede.add_argument("--session-id", required=True)
    contract_supersede.add_argument("--goal-slug", required=True)
    contract_supersede.add_argument("--supersedes-id", required=True)
    contract_supersede.add_argument("--change-reason", required=True)
    contract_supersede.add_argument("--id", required=True)
    contract_supersede.add_argument("--title", required=True)
    contract_supersede.add_argument("--description", required=True)
    contract_supersede.add_argument("--depends-on-json")
    contract_supersede.add_argument("--scope-json")
    contract_supersede.add_argument("--work-json", required=True)
    contract_supersede.add_argument("--gate-json")
    contract_supersede.add_argument("--recovery-json")
    contract_supersede.add_argument("--budget-json")
    contract_supersede.add_argument("--acceptance-json", required=True)
    contract_supersede.add_argument("--contract-defaults", choices=["none", "local"], default="none")
    contract_supersede.add_argument("--revision", default="1")
    contract_supersede.add_argument("--optional", action="store_true")
    contract_supersede.set_defaults(func=cmd_contract_supersede_stage)

    evidence = sub.add_parser("evidence-add")
    evidence.add_argument("--session-id", required=True)
    evidence.add_argument("--goal-slug", required=True)
    evidence.add_argument("--type", choices=sorted(EVIDENCE_TYPES), required=True)
    evidence.add_argument("--summary", required=True)
    evidence.add_argument("--supports", action="append", required=True)
    evidence.add_argument("--artifact")
    evidence.add_argument("--artifact-json")
    evidence.add_argument("--created-by", default="main_agent")
    evidence.set_defaults(func=cmd_evidence_add)

    validate = sub.add_parser("contract-validate")
    validate.add_argument("--session-id")
    validate.add_argument("--goal-slug")
    validate.set_defaults(func=cmd_validate_contract)

    lock = sub.add_parser("contract-lock")
    lock.add_argument("--session-id", required=True)
    lock.add_argument("--goal-slug", required=True)
    lock.add_argument("--locked-by", default="main_agent")
    lock.set_defaults(func=cmd_contract_lock)

    packet = sub.add_parser("packet-create")
    packet.add_argument("--session-id", required=True)
    packet.add_argument("--goal-slug", required=True)
    packet.add_argument("--review-mode", choices=["exit_review", "delta_review"], default="exit_review")
    packet.add_argument("--acceptance-id", action="append", help="Limit a delta review packet to one acceptance id; repeatable")
    packet.set_defaults(func=cmd_packet_create)

    packet_read = sub.add_parser("packet-read")
    packet_read.add_argument("--session-id", required=True)
    packet_read.add_argument("--goal-slug", required=True)
    packet_read.add_argument("--review-mode", choices=["exit_review", "delta_review"], required=True)
    packet_read.add_argument("--packet-id")
    packet_read.set_defaults(func=cmd_packet_read)

    verdict = sub.add_parser("verdict-compute")
    verdict.add_argument("--session-id", required=True)
    verdict.add_argument("--goal-slug", required=True)
    verdict.set_defaults(func=cmd_verdict_compute)

    loop_status = sub.add_parser("loop-status")
    loop_status.add_argument("--session-id", required=True)
    loop_status.add_argument("--goal-slug", required=True)
    loop_status.set_defaults(func=cmd_loop_status)

    ledger_audit = sub.add_parser("ledger-audit")
    ledger_audit.add_argument("--session-id", required=True)
    ledger_audit.add_argument("--goal-slug", required=True)
    ledger_audit.set_defaults(func=cmd_ledger_audit)

    loop_start = sub.add_parser("loop-start-stage")
    loop_start.add_argument("--session-id", required=True)
    loop_start.add_argument("--goal-slug", required=True)
    loop_start.add_argument("--plan-item-id", required=True)
    loop_start.add_argument("--status", choices=sorted(PUBLIC_LOOP_START_STATUSES), default="running")
    loop_start.set_defaults(func=cmd_loop_start_stage)

    cont = sub.add_parser("continue")
    cont.add_argument("--session-id", required=True)
    cont.add_argument("--goal-slug", required=True)
    cont.set_defaults(func=cmd_continue)

    status = sub.add_parser("status")
    status.set_defaults(func=cmd_status)

    hook_health = sub.add_parser("hook-health")
    hook_health.set_defaults(func=cmd_hook_health)

    hook = sub.add_parser("hook")
    hook.add_argument("action", choices=["pre-tool-use", "stop"])
    hook.add_argument("--project-root", default=argparse.SUPPRESS, help="Project root that owns .mobius")
    hook.add_argument("--session-id")
    hook.set_defaults(func=cmd_hook)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
