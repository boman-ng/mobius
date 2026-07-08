#!/usr/bin/env python3
"""MobiusCV MCP server.

MobiusCV is stateless by design: every review call must include a frozen index packet and any
host-mediated Codex subagent result explicitly in the input.
"""

from __future__ import annotations

import json
import os
import re
import selectors
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from mcp.server.fastmcp import FastMCP

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))
import mobius


SERVER_VERSION = "0.4.0"
RESULT_SCHEMA = "mobius.cv_result"
REVIEWER_SCHEMA = "mobius.cv_reviewer_result"
VALID_REVIEW_MODES = {"delta_review", "exit_review"}
VALID_LEVELS = {1, 2}
REVIEWER_RESULT_START = "MOBIUS_CV_REVIEWER_RESULT"
REVIEWER_RESULT_END = "END_MOBIUS_CV_REVIEWER_RESULT"
VALID_REVIEWER_VERDICTS = {"pass", "fail", "unknown", "blocked"}
KIMI_CONNECTIVITY_PROMPT = "Reply hi"
KIMI_CONNECTIVITY_PATTERN = re.compile(r"\bhi\b", re.IGNORECASE)
KIMI_CHILD_ENV = "MOBIUS_CV_KIMI_CHILD"
KIMI_STARTUP_SMOKE_TIMEOUT_SECONDS = 45
KIMI_FIRST_EVENT_TIMEOUT_SECONDS = 180
KIMI_ACTIVITY_IDLE_TIMEOUT_SECONDS = 300
KIMI_HARD_TIMEOUT_SECONDS = 900
AUTH_UNAVAILABLE_PATTERNS = (
    "OAuth provider",
    "failed to fetch an access token",
    "auth.kimi.com",
    "Unauthorized",
    "Forbidden",
    "invalid_grant",
    "login required",
    "token expired",
)
AUTH_STATUS_PATTERN = re.compile(
    r"\b(?:http|status|status_code|code|response|error|auth|authorization)[^\n]{0,40}\b(?:401|403)\b|\b(?:401|403)\b[^\n]{0,40}\b(?:unauthorized|forbidden|auth|authorization)\b",
    re.IGNORECASE,
)

INSTRUCTIONS = (
    "MobiusCV provides stateless external review gates for explicitly targeted Mobius goals. "
    "Use MobiusCV only for an explicitly targeted Mobius goal. Pass a frozen JSON index packet "
    "with local file references every time. Do not pass prior review chat as scope. "
    "Delta reviews only check changed claims; exit reviews must check the full acceptance matrix. "
    "Reviewers audit evidence quality, assumptions, blind spots, disconfirmation, Goodhart risk, "
    "contract drift, staleness, and pruning concerns. Missing, unchecked, invalid, ambiguous, or "
    "degraded reviewer output is not a pass."
)

mcp = FastMCP(name="mobius-cv", instructions=INSTRUCTIONS)
STARTED_AT = datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def now_iso() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def env_enabled(name: str, default: bool = True) -> bool:
    value = os.environ.get(name)
    if value is None:
        return default
    return value.strip().lower() not in {"0", "false", "no", "off"}


def compact_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def terminate_process_group(pgid: int | None, grace_seconds: float = 0.3) -> None:
    if pgid is None:
        return
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return
    except PermissionError:
        return
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(pgid, sig)
        except ProcessLookupError:
            return
        except PermissionError:
            return
        time.sleep(grace_seconds)


def run_command(
    args: list[str],
    timeout: int = 20,
    input_text: str | None = None,
    cwd: str | None = None,
    env: dict[str, str] | None = None,
    isolate_process_group: bool = False,
    cleanup_process_group: bool = False,
) -> dict[str, Any]:
    pgid: int | None = None
    try:
        process = subprocess.Popen(
            args,
            stdin=subprocess.PIPE if input_text is not None else None,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=cwd,
            env=env,
            start_new_session=isolate_process_group,
        )
        if isolate_process_group:
            pgid = os.getpgid(process.pid)
        stdout, stderr = process.communicate(input=input_text, timeout=timeout)
    except FileNotFoundError:
        return {"status": "missing_cli", "args": args, "exit_code": None, "stdout": "", "stderr": ""}
    except subprocess.TimeoutExpired as exc:
        terminate_process_group(pgid)
        stdout = exc.stdout or ""
        stderr = exc.stderr or ""
        try:
            more_stdout, more_stderr = process.communicate(timeout=2)
            stdout = more_stdout or stdout
            stderr = more_stderr or stderr
        except subprocess.TimeoutExpired:
            terminate_process_group(pgid, grace_seconds=0.1)
        return {
            "status": "timeout",
            "args": args,
            "exit_code": None,
            "stdout": stdout,
            "stderr": stderr,
        }
    finally:
        if cleanup_process_group:
            terminate_process_group(pgid)
    return {"status": "ok" if process.returncode == 0 else "error", "args": args, "exit_code": process.returncode, "stdout": stdout, "stderr": stderr}


def kimi_child_env() -> dict[str, str]:
    env = os.environ.copy()
    env[KIMI_CHILD_ENV] = "1"
    env["MOBIUS_CV_STARTUP_SMOKE"] = "0"
    return env


def run_kimi_command(args: list[str], timeout: int = 20, input_text: str | None = None) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="mobius-cv-kimi-cwd-") as cwd:
        return run_command(
            args,
            timeout=timeout,
            input_text=input_text,
            cwd=cwd,
            env=kimi_child_env(),
            isolate_process_group=True,
            cleanup_process_group=True,
        )


def is_auth_unavailable(text: str) -> bool:
    haystack = text.lower()
    return any(pattern.lower() in haystack for pattern in AUTH_UNAVAILABLE_PATTERNS) or bool(AUTH_STATUS_PATTERN.search(text))


def classify_kimi_failure(stdout: str = "", stderr: str = "", status: str = "command_failed") -> tuple[str, bool]:
    if is_auth_unavailable(f"{stdout}\n{stderr}"):
        return "auth_unavailable", False
    if status in {"first_event_timeout", "activity_idle_timeout", "hard_timeout"}:
        return status, True
    if status in {"missing_cli", "no_prompt_mode", "startup_health_degraded", "workspace_unavailable"}:
        return status, False
    if status == "invalid_output":
        return status, True
    if status == "timeout":
        return "hard_timeout", True
    return "command_failed", True


def run_kimi_review_command(
    args: list[str],
    *,
    cwd: str | None = None,
    first_event_timeout_seconds: int = KIMI_FIRST_EVENT_TIMEOUT_SECONDS,
    activity_idle_timeout_seconds: int = KIMI_ACTIVITY_IDLE_TIMEOUT_SECONDS,
    hard_timeout_seconds: int = KIMI_HARD_TIMEOUT_SECONDS,
) -> dict[str, Any]:
    stdout_parts: list[str] = []
    stderr_parts: list[str] = []
    start = time.monotonic()
    pgid: int | None = None
    process: subprocess.Popen[str] | None = None
    timed_status: str | None = None
    temp_cwd: tempfile.TemporaryDirectory[str] | None = None
    try:
        run_cwd = cwd
        if run_cwd is None:
            temp_cwd = tempfile.TemporaryDirectory(prefix="mobius-cv-kimi-cwd-")
            run_cwd = temp_cwd.name
        process = subprocess.Popen(
            args,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=run_cwd,
            env=kimi_child_env(),
            start_new_session=True,
            bufsize=1,
        )
        pgid = os.getpgid(process.pid)
        selector = selectors.DefaultSelector()
        if process.stdout is not None:
            selector.register(process.stdout, selectors.EVENT_READ, "stdout")
        if process.stderr is not None:
            selector.register(process.stderr, selectors.EVENT_READ, "stderr")
        first_event_seen = False
        last_event = start

        while True:
            now = time.monotonic()
            if process.poll() is not None:
                break
            if now - start >= hard_timeout_seconds:
                timed_status = "hard_timeout"
                break
            if not first_event_seen and now - start >= first_event_timeout_seconds:
                timed_status = "first_event_timeout"
                break
            if first_event_seen and now - last_event >= activity_idle_timeout_seconds:
                timed_status = "activity_idle_timeout"
                break

            events = selector.select(timeout=0.25)
            for key, _mask in events:
                stream = key.fileobj
                line = stream.readline()
                if line == "":
                    try:
                        selector.unregister(stream)
                    except Exception:
                        pass
                    continue
                if key.data == "stdout":
                    stdout_parts.append(line)
                else:
                    stderr_parts.append(line)
                first_event_seen = True
                last_event = time.monotonic()

        if timed_status:
            terminate_process_group(pgid)

        try:
            more_stdout, more_stderr = process.communicate(timeout=2)
        except subprocess.TimeoutExpired:
            terminate_process_group(pgid, grace_seconds=0.1)
            more_stdout, more_stderr = "", ""
        stdout_parts.append(more_stdout or "")
        stderr_parts.append(more_stderr or "")
    except FileNotFoundError:
        return {
            "status": "missing_cli",
            "args": args,
            "exit_code": None,
            "stdout": "",
            "stderr": "",
            "duration_seconds": round(time.monotonic() - start, 3),
        }
    finally:
        terminate_process_group(pgid)
        if temp_cwd is not None:
            temp_cwd.cleanup()

    stdout = "".join(stdout_parts)
    stderr = "".join(stderr_parts)
    if timed_status:
        status = timed_status
        exit_code = None
    else:
        exit_code = process.returncode if process is not None else None
        status = "ok" if exit_code == 0 else "error"
    return {
        "status": status,
        "args": args,
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "duration_seconds": round(time.monotonic() - start, 3),
    }


def redact_check(check: dict[str, Any], command: str | None = None) -> dict[str, Any]:
    stdout = str(check.get("stdout", ""))
    stderr = str(check.get("stderr", ""))
    return {
        "command": command or " ".join(check.get("args", [])[-2:]),
        "status": check.get("status"),
        "exit_code": check.get("exit_code"),
        "stdout_preview": stdout[:600],
        "stderr_preview": stderr[:600],
    }


def discover_kimi(deep: bool = False) -> dict[str, Any]:
    binary = shutil.which("kimi")
    result: dict[str, Any] = {
        "id": "kimi-code",
        "kind": "cli",
        "command": "kimi",
        "path": binary,
        "available": bool(binary),
        "checks": [],
        "commands": [],
        "supports": {"prompt": False, "stream_json": False},
    }
    if not binary:
        result["status"] = "missing_cli"
        return result

    help_check = run_kimi_command([binary, "--help"], timeout=20)
    result["checks"].append(redact_check(help_check))
    help_text = f"{help_check.get('stdout', '')}\n{help_check.get('stderr', '')}"
    result["supports"]["prompt"] = "--prompt" in help_text or "-p" in help_text
    result["supports"]["stream_json"] = "stream-json" in help_text
    result["commands"] = sorted(set(re.findall(r"^\s{2}([a-z][a-z0-9|-]*)\s", help_text, flags=re.MULTILINE)))

    if deep and result["supports"]["prompt"]:
        smoke = kimi_connectivity_check(binary, timeout=KIMI_STARTUP_SMOKE_TIMEOUT_SECONDS)
        result["checks"].append(
            {
                "command": "connectivity_prompt",
                "status": smoke["status"],
                "prompt": KIMI_CONNECTIVITY_PROMPT,
                "signal": smoke["signal"],
                "stdout_preview": smoke.get("stdout_preview", ""),
                "stderr_preview": smoke.get("stderr_preview", ""),
            }
        )
        result["smoke_status"] = smoke["status"]
        result["startup_connectivity"] = {
            "status": smoke["status"],
            "valid": smoke["valid"],
            "prompt": KIMI_CONNECTIVITY_PROMPT,
            "signal": smoke["signal"],
        }

    if help_check["status"] != "ok":
        result["status"] = help_check["status"]
    elif not result["supports"]["prompt"]:
        result["status"] = "no_prompt_mode"
    elif deep and result.get("smoke_status") != "ok":
        result["status"] = "startup_health_degraded"
    else:
        result["status"] = "ready"
    return result


def packet_envelope_json(packet: dict[str, Any]) -> str:
    return json.dumps(packet, ensure_ascii=False, sort_keys=True, indent=2)


def load_packet(packet: dict[str, Any] | None) -> tuple[dict[str, Any] | None, str, list[str]]:
    if packet is None:
        return None, "", []
    if not isinstance(packet, dict):
        return None, "", ["packet JSON object is required"]
    if packet.get("schema") == "mobius.command_result" and isinstance(packet.get("packet"), dict):
        loaded = packet["packet"]
        return loaded, packet_envelope_json(loaded), []
    return packet, packet_envelope_json(packet), []


def review_contract_view_from_input(packet: dict[str, Any] | None) -> dict[str, Any] | None:
    if not isinstance(packet, dict):
        return None
    view = packet.get("review_contract_view")
    if isinstance(view, dict):
        return view
    return None


def extract_required_acceptance_ids(packet: dict[str, Any] | None, explicit: list[str] | None) -> list[str]:
    if isinstance(explicit, list):
        return [str(item) for item in explicit]
    if not isinstance(packet, dict):
        return []
    coverage = packet.get("coverage")
    if isinstance(coverage, dict):
        return [str(item) for item in coverage]
    return []


def path_is_within(path: Path, root: Path) -> bool:
    try:
        path.resolve().relative_to(root.resolve())
    except ValueError:
        return False
    return True


def reviewer_workspace_path(context: dict[str, Any] | None) -> tuple[Path | None, list[str]]:
    if not isinstance(context, dict) or not context.get("project_root"):
        return None, ["project_root is required for Kimi reviewer workspace"]
    root = Path(str(context["project_root"])).expanduser().resolve()
    if not root.exists():
        return None, [f"Kimi reviewer workspace is missing: {root}"]
    if not root.is_dir():
        return None, [f"Kimi reviewer workspace is not a directory: {root}"]
    return root, []


def reviewer_workspace_context(root: Path, goal_dir: Path, packet: dict[str, Any] | None) -> dict[str, Any]:
    return {
        "type": "project_root",
        "cwd": str(root),
        "ledger_abs_root": str(goal_dir),
        "ledger_root": str((packet or {}).get("ledger", {}).get("root", "")),
        "visibility": "project_root_relative",
    }


def representative_file_ref_errors(root: Path, packet: dict[str, Any] | None) -> list[str]:
    refs = (packet or {}).get("refs")
    if not isinstance(refs, dict):
        return ["packet refs are required for reviewer workspace preflight"]
    errors: list[str] = []
    for evidence_id, ref in sorted(refs.items()):
        if not isinstance(ref, list) or len(ref) < 2 or str(ref[0]) != "file_ref":
            continue
        label = str(ref[1])
        if Path(label).is_absolute():
            errors.append(f"file_ref {evidence_id} must be project-root-relative: {label}")
            continue
        candidate = (root / label).resolve()
        if not path_is_within(candidate, root):
            errors.append(f"file_ref {evidence_id} escapes project root: {label}")
            continue
        if not candidate.is_file():
            errors.append(f"file_ref {evidence_id} is not visible from reviewer workspace: {label}")
    return errors


def preflight_exit_review_workspace(
    root: Path,
    goal_dir: Path,
    packet: dict[str, Any] | None,
    policy: dict[str, Any],
) -> list[str]:
    errors: list[str] = []
    if "kimi-code" in policy.get("required_reviewers", []):
        binary = shutil.which("kimi")
        if not binary:
            errors.append("Kimi reviewer preflight failed: kimi CLI is not installed or not on PATH")
        capability = discover_kimi(deep=False) if binary else {"status": "missing_cli", "supports": {}}
        supports = capability.get("supports") if isinstance(capability.get("supports"), dict) else {}
        if capability.get("status") == "no_prompt_mode" or not supports.get("prompt"):
            errors.append("Kimi reviewer preflight failed: kimi CLI does not expose prompt mode")
        elif capability.get("status") not in {"ready", "startup_health_degraded"}:
            errors.append(f"Kimi reviewer preflight failed: capability status {capability.get('status')}")

    ledger_root = str((packet or {}).get("ledger", {}).get("root", ""))
    if not ledger_root:
        errors.append("exit reviewer preflight failed: packet ledger.root is missing")
    elif Path(ledger_root).is_absolute():
        errors.append("exit reviewer preflight failed: packet ledger.root must be project-root-relative")
    else:
        ledger_abs = (root / ledger_root).resolve()
        if not path_is_within(ledger_abs, root):
            errors.append("exit reviewer preflight failed: packet ledger.root escapes project root")
        elif ledger_abs != goal_dir.resolve():
            errors.append("exit reviewer preflight failed: packet ledger.root does not match goal directory")
        elif not ledger_abs.is_dir():
            errors.append("exit reviewer preflight failed: packet ledger.root is not visible from reviewer workspace")

    for name in ("goal.md", "goal.csv", "plan.csv", "acceptance.csv", "evidence.csv", "packets.csv"):
        if not (goal_dir / name).is_file():
            errors.append(f"exit reviewer preflight failed: ledger file is not visible: {name}")
    errors.extend(representative_file_ref_errors(root, packet))
    return errors


def missing_ids(required_ids: list[str], checked_ids: Any) -> list[str]:
    checked = {str(item) for item in checked_ids} if isinstance(checked_ids, list) else set()
    return [item for item in required_ids if item not in checked]


def reviewer_context_block(context: dict[str, Any] | None) -> str:
    if not context:
        return ""
    material = {
        key: value
        for key, value in context.items()
        if key
        in {
            "project_root",
            "ledger_root",
            "ledger_abs_root",
            "goal_slug",
            "session_id",
        }
    }
    if not material:
        return ""
    return f"""
Reviewer local path context:
```json
{json.dumps(material, ensure_ascii=False, sort_keys=True, indent=2)}
```
Use this context only to resolve packet-local paths. The frozen Mobius packet JSON above remains the
review object of record.
"""


def review_contract_view_block(review_contract_view: dict[str, Any] | None) -> str:
    if not isinstance(review_contract_view, dict):
        return ""
    return f"""
Derived review contract view:
```json
{json.dumps(review_contract_view, ensure_ascii=False, sort_keys=True, indent=2)}
```
This view is generated from Mobius ledgers and the frozen packet. It is not a ledger, packet schema,
verdict engine, or acceptance source. If it conflicts with the indexed packet or local ledgers, use
the packet/ledgers as authoritative evidence and return unknown or blocked rather than pass.
"""


def review_prompt(
    packet_json: str,
    review_mode: str,
    required_ids: list[str],
    reviewer: str,
    context: dict[str, Any] | None = None,
    review_contract_view: dict[str, Any] | None = None,
) -> str:
    scope = (
        "Exit review: inspect the full frozen index from scratch, including the required plan DAG and full acceptance matrix."
        if review_mode == "exit_review"
        else "Delta review: inspect only the target stage contract and linked proof obligations; do not issue final acceptance for the goal."
    )
    return f"""You are an independent, stateless MobiusCV reviewer.

Role and stateless rule:
- Review this request from scratch.
- Ignore prior review rounds, prior chat, and any unstated context.
- Treat this packet as one-shot input; do not rely on or request reuse of an older packet_id.
- Return only the required result block. Do not add prose before or after it.

Review scope:
- {scope}
- Audit claims, risks, acceptance obligations, and proof coverage.
- Treat packet refs as starting points, not exclusive evidence and not authoritative proof.
- Inspect plan stage contract fields: scope_json, work_json, gate_json, recovery_json, budget_json, and acceptance_ids_json when needed.
- Inspect linked acceptance fields: requirement, observable_outcome, evidence_required_json, verifier_json, and review_focus_json when needed.
- Inspect evidence rows for the linked acceptance ids and verify they satisfy evidence_required_json.
- Check assumptions, known unknowns, blind spots, counterevidence, and expected feedback signals when they appear in review_focus_json or contract fields.
- For each checked required acceptance id, identify a plausible disconfirming observation that
  would falsify the acceptance claim; if no such observation is considered, do not return pass.
- Check contract drift: implementation and review claims must stay inside locked scope_json, work_json, gate_json, recovery_json, budget_json, and linked acceptance ids.
- Treat missing evidence, unchecked acceptance rows, degraded tools, or ambiguity as unknown, not pass.
- Treat Agent confidence, self-review, stage completion, or process completion as insufficient for pass.
- Prefer concrete blockers over general advice.

Frozen index and tool boundary:
- The packet below is a frozen local index, not a full evidence archive.
- It must be generated from current Mobius ledgers for this review attempt.
- Start from packet.ledger, packet.coverage, and packet.refs.
- The reviewer may inspect local context needed to validate a referenced claim, path, command, or absence claim.
- Keep review activity read-only: do not modify files, write artifacts, install dependencies, start services, or run destructive commands.
- Audit sensor quality: refs, file hashes when practical, command and test exit semantics, evidence freshness, coverage limits, and whether the evidence actually measures the user outcome.
- Check Goodhart or proxy risk: a passing metric, command, or process must not replace the locked observable outcome.
- Check variety coverage when required: happy path, boundary, negative, state, and absence-claim evidence should match the acceptance risk.
- Before relying on an indexed file, compare its local hash when practical.
- If an indexed file is missing, unreadable, hash-mismatched, stale, or older than final source changes when final evidence is required, return VERDICT: unknown or VERDICT: blocked with a blocking finding.
- Flag compatibility, fallback, alias, glue, or history-preserving code when no locked contract names a real external user, data, or API contract that requires it.

Required acceptance ids:
{compact_json(required_ids)}

Checklist claims:
- C1: The indexed local files support the required plan stage contract and acceptance ids for this review scope.
- C2: Every required acceptance id is either checked against its proof obligations or explicitly listed as unchecked.
- C3: No degraded, missing, or ambiguous evidence is treated as pass.
- C4: The review mode is {review_mode} and the reviewer id is {reviewer}.
- C5: The review checked contract fields rather than relying on prose summaries.
- C6: Assumptions, blind spots, counterevidence, and Goodhart or proxy risk were checked against objective evidence.
- C7: Every checked required acceptance id was considered against a disconfirming observation that would falsify the claim.
- C8: Contract drift, stale refs, unchecked ids, and pruning concerns are surfaced as normal blocking findings or required revisions.

Frozen Mobius packet JSON:
```json
{packet_json}
```
{review_contract_view_block(review_contract_view)}
{reviewer_context_block(context)}

Self-check before returning:
- CHECKED_ACCEPTANCE_IDS, UNCHECKED_ACCEPTANCE_IDS, BLOCKING_FINDINGS, REQUIRED_REVISIONS, and EVIDENCE_CHECKED must be JSON arrays.
- Empty arrays must be [].
- Missing evidence is unknown, not pass.
- Stale evidence, unchecked ids, degraded reviewers, ambiguous evidence, or confidence-only completion is not pass.
- For delta_review, do not issue final acceptance.
- For exit_review, check the full active required plan DAG and acceptance matrix.

Minimal passing example values:
- VERDICT: pass
- CHECKED_ACCEPTANCE_IDS: {compact_json(required_ids)}
- UNCHECKED_ACCEPTANCE_IDS: []
- BLOCKING_FINDINGS: []
- REQUIRED_REVISIONS: []
- EVIDENCE_CHECKED: ["plan.csv","acceptance.csv","evidence.csv"]

Copy this final template exactly and replace placeholders only:
{REVIEWER_RESULT_START}
REVIEWER: {reviewer}
REVIEW_MODE: {review_mode}
VERDICT: <pass|fail|unknown|blocked>
CHECKED_ACCEPTANCE_IDS: <json array>
UNCHECKED_ACCEPTANCE_IDS: <json array>
BLOCKING_FINDINGS: <json array of strings>
REQUIRED_REVISIONS: <json array of strings>
EVIDENCE_CHECKED: <json array of strings>
NOTES: <short text>
{REVIEWER_RESULT_END}"""


ARRAY_FIELD_NAMES = {
    "checked_acceptance_ids",
    "unchecked_acceptance_ids",
    "blocking_findings",
    "required_revisions",
    "evidence_checked",
}


def parse_json_array_field(name: str, value: str, errors: list[str]) -> list[str]:
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError:
        errors.append(f"{name} must be a JSON array")
        return []
    if not isinstance(parsed, list):
        errors.append(f"{name} must be a JSON array")
        return []
    if any(not isinstance(item, str) for item in parsed):
        errors.append(f"{name} must contain only strings")
        return []
    return parsed


def invalid_reviewer_result(
    reviewer_id: str,
    review_mode: str,
    required_ids: list[str],
    errors: list[str],
    raw_text: str = "",
    *,
    retryable: bool = True,
) -> dict[str, Any]:
    raw_hash_tail = mobius.sha256_tail(mobius.sha256_text(raw_text)) if raw_text else ""
    result = {
        "schema": REVIEWER_SCHEMA,
        "reviewer_id": reviewer_id,
        "review_mode": review_mode,
        "status": "invalid_output",
        "verdict": "unknown",
        "checked_acceptance_ids": [],
        "unchecked_acceptance_ids": list(required_ids),
        "blocking_findings": [f"invalid reviewer output: {'; '.join(errors)}"],
        "required_revisions": [],
        "evidence_checked": [],
        "retryable": retryable,
        "raw_ref": "",
        "raw_hash_tail": raw_hash_tail,
        "stateless": True,
    }
    if raw_text:
        result["_raw_text"] = raw_text
    return result


def parse_reviewer_result_block(
    text: str,
    expected_reviewer_id: str,
    expected_review_mode: str,
) -> tuple[dict[str, Any] | None, list[str]]:
    stripped = text.strip()
    errors: list[str] = []
    start_count = len(re.findall(rf"(?m)^{re.escape(REVIEWER_RESULT_START)}$", stripped))
    end_count = len(re.findall(rf"(?m)^{re.escape(REVIEWER_RESULT_END)}$", stripped))
    if start_count != 1:
        errors.append(f"expected exactly one {REVIEWER_RESULT_START} block")
    if end_count != 1:
        errors.append(f"expected exactly one {REVIEWER_RESULT_END} block")
    if errors:
        return None, errors
    pattern = rf"^{re.escape(REVIEWER_RESULT_START)}\n(.*?)\n{re.escape(REVIEWER_RESULT_END)}$"
    match = re.fullmatch(pattern, stripped, flags=re.S)
    if not match:
        return None, ["reviewer result must contain no prose before or after the result block"]

    fields: dict[str, str] = {}
    for line in match.group(1).splitlines():
        if not line.strip():
            continue
        if ":" not in line:
            errors.append(f"invalid result line: {line}")
            continue
        key, value = line.split(":", 1)
        normalized_key = key.strip().lower()
        if normalized_key in fields:
            errors.append(f"duplicate field: {normalized_key}")
            continue
        fields[normalized_key] = value.strip()

    required_keys = {
        "reviewer",
        "review_mode",
        "verdict",
        "checked_acceptance_ids",
        "unchecked_acceptance_ids",
        "blocking_findings",
        "required_revisions",
        "evidence_checked",
    }
    optional_keys = {"notes"}
    unknown = sorted(set(fields) - required_keys - optional_keys)
    if unknown:
        errors.append("unknown fields: " + ",".join(unknown))
    missing = sorted(required_keys - set(fields))
    if missing:
        errors.append("missing required fields: " + ",".join(missing))

    reviewer = fields.get("reviewer", "")
    mode = fields.get("review_mode", "")
    verdict = fields.get("verdict", "").lower()
    if reviewer != expected_reviewer_id:
        errors.append(f"reviewer mismatch: expected {expected_reviewer_id}, got {reviewer}")
    if mode != expected_review_mode:
        errors.append(f"review_mode mismatch: expected {expected_review_mode}, got {mode}")
    if verdict not in VALID_REVIEWER_VERDICTS:
        errors.append("verdict must be pass, fail, unknown, or blocked")

    arrays = {
        name: parse_json_array_field(name, fields.get(name, ""), errors)
        for name in ARRAY_FIELD_NAMES
        if name in fields
    }
    if errors:
        return None, errors

    result = {
        "schema": REVIEWER_SCHEMA,
        "reviewer_id": reviewer,
        "review_mode": mode,
        "status": "completed",
        "verdict": verdict,
        "checked_acceptance_ids": arrays["checked_acceptance_ids"],
        "unchecked_acceptance_ids": arrays["unchecked_acceptance_ids"],
        "blocking_findings": arrays["blocking_findings"],
        "required_revisions": arrays["required_revisions"],
        "evidence_checked": arrays["evidence_checked"],
        "notes": fields.get("notes", ""),
        "retryable": False,
        "raw_ref": "",
        "raw_hash_tail": mobius.sha256_tail(mobius.sha256_text(stripped)),
        "stateless": True,
    }
    result["_raw_text"] = stripped
    return result, []


def validate_reviewer_result_dict(result: dict[str, Any], reviewer_id: str, review_mode: str) -> list[str]:
    errors: list[str] = []
    if result.get("schema", REVIEWER_SCHEMA) != REVIEWER_SCHEMA:
        errors.append(f"schema must be {REVIEWER_SCHEMA}")
    if result.get("reviewer_id") != reviewer_id:
        errors.append(f"reviewer_id mismatch: expected {reviewer_id}, got {result.get('reviewer_id', '')}")
    if result.get("review_mode") != review_mode:
        errors.append(f"review_mode mismatch: expected {review_mode}, got {result.get('review_mode', '')}")
    status = result.get("status")
    if not isinstance(status, str) or not status:
        errors.append("status is required")
    verdict = result.get("verdict", "unknown")
    if verdict not in VALID_REVIEWER_VERDICTS:
        errors.append("verdict must be pass, fail, unknown, or blocked")
    if status == "completed":
        for name in ARRAY_FIELD_NAMES:
            if not isinstance(result.get(name), list):
                errors.append(f"{name} must be a JSON array")
    return errors


def normalize_reviewer_result(raw: Any, reviewer_id: str, review_mode: str, required_ids: list[str]) -> dict[str, Any]:
    if isinstance(raw, str):
        parsed, errors = parse_reviewer_result_block(raw, reviewer_id, review_mode)
        if errors or parsed is None:
            return invalid_reviewer_result(reviewer_id, review_mode, required_ids, errors, raw_text=raw)
        return parsed
    if isinstance(raw, dict):
        result = dict(raw)
        if "_raw_text" not in result:
            result["_raw_text"] = compact_json(raw)
    else:
        return invalid_reviewer_result(
            reviewer_id,
            review_mode,
            required_ids,
            [f"unsupported reviewer result type: {type(raw).__name__}"],
        )

    result.setdefault("schema", REVIEWER_SCHEMA)
    result.setdefault("status", "completed" if result.get("verdict") else "invalid_output")
    result.setdefault("verdict", "unknown")
    result.setdefault("checked_acceptance_ids", [])
    result.setdefault("unchecked_acceptance_ids", missing_ids(required_ids, result.get("checked_acceptance_ids", [])))
    result.setdefault("blocking_findings", [])
    result.setdefault("required_revisions", [])
    result.setdefault("evidence_checked", [])
    errors = validate_reviewer_result_dict(result, reviewer_id, review_mode)
    if errors:
        return invalid_reviewer_result(reviewer_id, review_mode, required_ids, errors, raw_text=compact_json(result))
    result["stateless"] = True
    return result


def assistant_text_from_stream_json(stdout: str) -> str:
    fragments: list[str] = []
    for line in stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict):
            if event.get("role") != "assistant":
                continue
            message = event.get("message")
            if isinstance(message, dict) and isinstance(message.get("content"), str):
                fragments.append(message["content"])
            for key in ("content", "text", "delta"):
                if isinstance(event.get(key), str):
                    fragments.append(event[key])
    return "\n".join(fragment for fragment in fragments if fragment).strip()


def kimi_connectivity_check(binary: str, timeout: int) -> dict[str, Any]:
    raw = run_kimi_command([binary, "-p", KIMI_CONNECTIVITY_PROMPT, "--output-format", "stream-json"], timeout=timeout)
    status, retryable = classify_kimi_failure(str(raw.get("stdout", "")), str(raw.get("stderr", "")), str(raw.get("status", "")))
    text = assistant_text_from_stream_json(str(raw.get("stdout", "")))
    signal = text.strip()
    valid = raw["status"] == "ok" and bool(KIMI_CONNECTIVITY_PATTERN.search(signal))
    return {
        "status": "ok" if valid else status,
        "valid": valid,
        "retryable": retryable,
        "signal": signal[:200],
        "stdout_preview": str(raw.get("stdout", ""))[:600],
        "stderr_preview": str(raw.get("stderr", ""))[:600],
    }


def degraded_kimi_result(
    status: str,
    review_mode: str,
    required_ids: list[str],
    blocking_finding: str,
    *,
    retryable: bool,
    raw: dict[str, Any] | None = None,
) -> dict[str, Any]:
    result = normalize_reviewer_result(
        {
            "schema": REVIEWER_SCHEMA,
            "reviewer_id": "kimi-code",
            "review_mode": review_mode,
            "status": status,
            "verdict": "unknown",
            "checked_acceptance_ids": [],
            "unchecked_acceptance_ids": required_ids,
            "blocking_findings": [blocking_finding],
            "required_revisions": [],
            "evidence_checked": [],
            "retryable": retryable,
        },
        "kimi-code",
        review_mode,
        required_ids,
    )
    if raw:
        result["cli_status"] = {
            "status": raw.get("status"),
            "exit_code": raw.get("exit_code"),
            "duration_seconds": raw.get("duration_seconds"),
        }
        assistant_text = assistant_text_from_stream_json(str(raw.get("stdout", "")))
        raw_material = str(raw.get("stdout", "")) + "\n" + str(raw.get("stderr", "")) + "\n" + assistant_text
        result["raw_ref"] = ""
        result["raw_hash_tail"] = mobius.sha256_tail(mobius.sha256_text(raw_material))
        result["_raw_text"] = raw_material
    return result


def run_kimi_review(
    packet_json: str,
    review_mode: str,
    required_ids: list[str],
    timeout_seconds: int,
    context: dict[str, Any] | None = None,
    review_contract_view: dict[str, Any] | None = None,
) -> dict[str, Any]:
    binary = shutil.which("kimi")
    if not binary:
        return degraded_kimi_result(
            "missing_cli",
            review_mode,
            required_ids,
            "kimi CLI is not installed or not on PATH",
            retryable=False,
        )
    capability = discover_kimi(deep=False)
    if capability.get("status") == "missing_cli":
        return degraded_kimi_result(
            "missing_cli",
            review_mode,
            required_ids,
            "kimi CLI is not installed or not on PATH",
            retryable=False,
        )
    if capability.get("status") == "no_prompt_mode":
        return degraded_kimi_result(
            "no_prompt_mode",
            review_mode,
            required_ids,
            "kimi CLI does not expose a prompt mode",
            retryable=False,
        )
    if capability.get("status") not in {"ready", "startup_health_degraded"}:
        return degraded_kimi_result(
            "command_failed",
            review_mode,
            required_ids,
            "kimi capability check failed before review",
            retryable=True,
            raw={"status": capability.get("status"), "stdout": "", "stderr": compact_json(capability), "exit_code": None},
        )
    startup_kimi = STARTUP_HEALTH if isinstance(STARTUP_HEALTH, dict) else {}
    startup_connectivity = startup_kimi.get("startup_connectivity") if isinstance(startup_kimi.get("startup_connectivity"), dict) else {}
    workspace, workspace_errors = reviewer_workspace_path(context)
    if workspace_errors:
        return degraded_kimi_result(
            "workspace_unavailable",
            review_mode,
            required_ids,
            "; ".join(workspace_errors),
            retryable=False,
        )
    prompt = review_prompt(packet_json, review_mode, required_ids, "kimi-code", context, review_contract_view)
    hard_timeout = int(timeout_seconds or KIMI_HARD_TIMEOUT_SECONDS)
    first_event_timeout = min(KIMI_FIRST_EVENT_TIMEOUT_SECONDS, hard_timeout)
    activity_idle_timeout = min(KIMI_ACTIVITY_IDLE_TIMEOUT_SECONDS, hard_timeout)
    raw = run_kimi_review_command(
        [binary, "-p", prompt, "--output-format", "stream-json"],
        cwd=str(workspace),
        first_event_timeout_seconds=first_event_timeout,
        activity_idle_timeout_seconds=activity_idle_timeout,
        hard_timeout_seconds=hard_timeout,
    )
    if raw["status"] != "ok":
        failure_status, retryable = classify_kimi_failure(str(raw.get("stdout", "")), str(raw.get("stderr", "")), str(raw.get("status", "")))
        finding = (
            "kimi authentication unavailable; fix Kimi auth and rerun review"
            if failure_status == "auth_unavailable"
            else f"kimi review did not complete before producing a valid result: {failure_status}"
        )
        result = degraded_kimi_result(
            failure_status,
            review_mode,
            required_ids,
            finding,
            retryable=retryable,
            raw=raw,
        )
        result["startup_connectivity"] = startup_connectivity
        return result
    parsed = normalize_reviewer_result(assistant_text_from_stream_json(str(raw.get("stdout", ""))), "kimi-code", review_mode, required_ids)
    parsed["reviewer_id"] = "kimi-code"
    parsed["cli"] = {"command": "kimi", "output_format": "stream-json", "cwd": str(workspace)}
    parsed["startup_connectivity"] = startup_connectivity
    parsed["raw_ref"] = ""
    raw_material = str(raw.get("stdout", "")) + "\n" + str(raw.get("stderr", ""))
    parsed["raw_hash_tail"] = mobius.sha256_tail(mobius.sha256_text(raw_material))
    parsed["_raw_text"] = raw_material
    return parsed


def build_cv_result(
    review_mode: str,
    packet: dict[str, Any] | None,
    packet_json: str,
    required_ids: list[str],
    level: int,
    codex_subagent_result: dict[str, Any] | str,
    timeout_seconds: int,
    cv_id: str | None,
    input_refs: dict[str, Any] | None,
    goal_id: str,
    packet_id: str,
    review_contract_view: dict[str, Any] | None = None,
) -> dict[str, Any]:
    refs = dict(input_refs or {})
    policy = mobius.review_gate_policy(review_mode, refs.get("review_policy") or {"level": level})
    refs["review_policy"] = policy
    reviewers: list[dict[str, Any]] = []
    reviewers.append(normalize_reviewer_result(codex_subagent_result, "codex-subagent", review_mode, required_ids))

    if "kimi-code" in policy["required_reviewers"]:
        reviewers.append(run_kimi_review(packet_json, review_mode, required_ids, timeout_seconds, refs, review_contract_view))

    comparison = mobius.derive_cv_aggregate(reviewers, required_ids, review_mode, policy)
    suffix = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
    return {
        "schema": RESULT_SCHEMA,
        "cv_id": cv_id or f"cv_{review_mode.replace('_review', '')}_{suffix}",
        "goal_id": goal_id,
        "packet_id": packet_id,
        "review_mode": review_mode,
        "level": max(level, int(policy["minimum_level"])),
        "stateless": True,
        "reviewers": reviewers,
        "comparison": {
            "agreement": comparison["agreement"],
            "reviewer_verdicts": comparison["reviewer_verdicts"],
            "degraded_reviewers": comparison["degraded_reviewers"],
        },
        "result": {
            "overall": comparison["overall"],
            "checked_acceptance_ids": comparison["checked_acceptance_ids"],
            "unchecked_acceptance_ids": comparison["unchecked_acceptance_ids"],
            "blocking_findings": comparison["blocking_findings"],
            "required_revisions": comparison["required_revisions"],
        },
        "input_refs": refs,
        "returned_at": now_iso(),
    }


def recorded_error(
    review_mode: str,
    error: str,
    cv_result: dict[str, Any] | None = None,
    *,
    next_required_action: str = "fix_review_or_persistence_error",
) -> dict[str, Any]:
    return {
        "schema": "mobius.cv_recorded_result",
        "ok": False,
        "persisted": False,
        "goal_id": cv_result.get("goal_id", "") if isinstance(cv_result, dict) else "",
        "packet_id": str(cv_result.get("packet_id") or cv_result.get("packet") or "") if isinstance(cv_result, dict) else "",
        "cv_id": cv_result.get("cv_id", "") if isinstance(cv_result, dict) else "",
        "review_mode": review_mode,
        "gate": "error",
        "updated_files": [],
        "next_required_action": next_required_action,
        "blocking_findings": [error],
        "errors": [error],
        "error": error,
    }


def recorded_infra_failure(
    review_mode: str,
    goal_id: str,
    packet_id: str,
    cv_id: str,
    findings: list[str],
    loop: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema": "mobius.cv_recorded_result",
        "ok": True,
        "persisted": True,
        "goal_id": goal_id,
        "packet_id": packet_id,
        "cv_id": cv_id,
        "review_mode": review_mode,
        "gate": str(loop.get("loop_gate", "running")),
        "updated_files": ["review_attempts.csv"],
        "next_required_action": str(loop.get("next_required_action", "retry_review")),
        "blocking_findings": findings,
        "errors": [],
        "infra_failure": True,
        "loop": loop,
    }


def missing_codex_subagent_result_error() -> str:
    return "codex_subagent_result is required; call mobius_cv_build_subagent_prompt, run the host Codex subagent, then pass its stateless result back"


def reviewer_infra_failures(cv_result: dict[str, Any]) -> list[str]:
    reviewers = cv_result.get("reviewers")
    if not isinstance(reviewers, list):
        return ["reviewers are unavailable"]
    findings: list[str] = []
    for reviewer in reviewers:
        if not isinstance(reviewer, dict):
            findings.append("reviewer result is not an object")
            continue
        status = str(reviewer.get("status", ""))
        if status == "completed":
            continue
        reviewer_id = str(reviewer.get("reviewer_id", "unknown"))
        blocking = reviewer.get("blocking_findings")
        detail = "; ".join(str(item) for item in blocking) if isinstance(blocking, list) and blocking else status
        findings.append(f"{reviewer_id} infrastructure failure: {detail}")
    return findings


def review_and_record(
    review_mode: str,
    project_root: str,
    session_id: str,
    goal_slug: str,
    packet: dict[str, Any] | None,
    packet_id: str | None,
    level: int,
    codex_subagent_result: dict[str, Any] | str | None,
    required_acceptance_ids: list[str] | None,
    target_plan_item_id: str | None,
    target_acceptance_ids: list[str] | None,
    timeout_seconds: int,
    cv_id: str | None,
    input_refs: dict[str, Any] | None,
) -> dict[str, Any]:
    if level not in VALID_LEVELS:
        return recorded_error(review_mode, f"invalid level: {level}")
    input_review_contract_view = review_contract_view_from_input(packet)
    loaded_packet, loaded_text, errors = load_packet(packet)
    if errors:
        return recorded_error(review_mode, "; ".join(errors))
    root = Path(project_root).expanduser().resolve()
    goal_dir = mobius.load_goal_dir(root, session_id, goal_slug)
    if loaded_packet is None and packet_id:
        loaded_packet = mobius.packet_envelope_from_ledger(goal_dir, packet_id)
        if loaded_packet is None:
            return recorded_error(review_mode, f"packet_id is not recorded in packets.csv: {packet_id}")
        loaded_text = packet_envelope_json(loaded_packet)
    if loaded_packet is None:
        return recorded_error(review_mode, "packet JSON object or packet_id is required")
    ids = target_acceptance_ids or extract_required_acceptance_ids(loaded_packet, required_acceptance_ids)
    if review_mode == "delta_review" and not target_plan_item_id:
        return recorded_error(review_mode, "target_plan_item_id is required for delta_review", cv_result=loaded_packet)
    if review_mode == "delta_review" and not ids:
        return recorded_error(review_mode, "target_acceptance_ids or packet required_acceptance_ids are required for delta_review", cv_result=loaded_packet)
    terminal = mobius.terminal_verdict(goal_dir)
    if terminal:
        return recorded_error(review_mode, mobius.terminal_goal_error("record_cv_result", terminal), cv_result=loaded_packet)
    contract_errors = mobius.validate_contract_dir(goal_dir)
    if contract_errors:
        return recorded_error(review_mode, mobius.contract_error_text(contract_errors), cv_result=loaded_packet)
    try:
        mobius.require_locked_contract(goal_dir)
    except mobius.MobiusError as exc:
        return recorded_error(review_mode, str(exc), cv_result=loaded_packet)
    packet_id = str((loaded_packet or {}).get("packet", ""))
    if mobius.packet_has_recorded_review(goal_dir, packet_id):
        return recorded_error(review_mode, f"packet_id already has a recorded review: {packet_id}", cv_result=loaded_packet)
    expected_ids = [str(item) for item in ids] if review_mode == "delta_review" else None
    _normalized_packet, packet_errors = mobius.validate_packet_for_goal(goal_dir, loaded_packet or {}, review_mode, expected_ids)
    if packet_errors:
        action = "refresh_final_evidence" if review_mode == "exit_review" and any(
            mobius.REPAIRABLE_EXIT_BLOCKER_PATTERN.search(error) for error in packet_errors
        ) else "fix_review_or_persistence_error"
        return recorded_error(review_mode, "; ".join(packet_errors), next_required_action=action)
    if codex_subagent_result is None:
        return recorded_error(review_mode, missing_codex_subagent_result_error(), cv_result=loaded_packet)
    goal_state = mobius.read_single_csv(goal_dir / "goal.csv") or {}
    review_allowed_for_view = not mobius.packet_has_recorded_review(goal_dir, str((loaded_packet or {}).get("packet", "")))
    review_contract_view = input_review_contract_view
    if review_contract_view is None:
        try:
            review_contract_view = mobius.review_contract_view(
                goal_dir,
                loaded_packet or {},
                review_mode,
                [str(item) for item in ids],
                review_allowed=review_allowed_for_view,
            )
        except mobius.MobiusError:
            review_contract_view = None
    refs = dict(input_refs or {})
    refs.setdefault("project_root", str(root))
    refs.setdefault("ledger_abs_root", str(goal_dir))
    refs.setdefault("ledger_root", str((loaded_packet or {}).get("ledger", {}).get("root", "")))
    refs.setdefault("goal_slug", goal_slug)
    refs.setdefault("session_id", session_id)
    if review_mode == "delta_review" and target_plan_item_id:
        policy = mobius.review_policy_for_delta_targets(
            goal_dir,
            target_plan_item_id,
            [str(item) for item in ids],
            refs.get("review_policy") or {"level": level},
            level=level,
        )
    else:
        policy = mobius.review_gate_policy(review_mode, refs.get("review_policy") or {"level": level})
    refs["review_policy"] = policy
    refs.setdefault("review_workspace", reviewer_workspace_context(root, goal_dir, loaded_packet))
    if review_mode == "exit_review":
        preflight_errors = preflight_exit_review_workspace(root, goal_dir, loaded_packet, policy)
        if preflight_errors:
            return recorded_error(review_mode, "; ".join(preflight_errors), cv_result=loaded_packet)
    attempt_id = ""
    try:
        attempt_id = mobius.review_attempt_started(goal_dir, packet_id, review_mode)
    except mobius.MobiusError:
        attempt_id = ""
    try:
        cv_result = build_cv_result(
            review_mode,
            loaded_packet,
            loaded_text,
            ids,
            level,
            codex_subagent_result,
            timeout_seconds,
            cv_id,
            refs,
            str(goal_state.get("goal_id", "")),
            packet_id,
            review_contract_view,
        )
        infra_failures = reviewer_infra_failures(cv_result)
        if infra_failures:
            retryable = all(
                bool(reviewer.get("retryable", True))
                for reviewer in cv_result.get("reviewers", [])
                if isinstance(reviewer, dict) and reviewer.get("status") != "completed"
            )
            if attempt_id:
                mobius.review_attempt_finished(
                    goal_dir,
                    attempt_id,
                    "failed",
                    cv_id or str(cv_result.get("cv_id", "")),
                    failure_kind="reviewer_infra_failure",
                    retryable=retryable,
                    diagnostic_ref=cv_id or str(cv_result.get("cv_id", "")),
                )
            audit = mobius.ledger_audit_data(root, session_id, goal_slug)
            return recorded_infra_failure(
                review_mode,
                str(goal_state.get("goal_id", "")),
                packet_id,
                str(cv_result.get("cv_id", "")),
                infra_failures,
                audit["loop"],
            )
    except KeyboardInterrupt:
        if attempt_id:
            mobius.review_attempt_finished(goal_dir, attempt_id, "interrupted")
        raise
    except mobius.MobiusError as exc:
        if attempt_id:
            mobius.review_attempt_finished(goal_dir, attempt_id, "failed", failure_kind="mobius_error", retryable=False, diagnostic_ref=str(exc)[:160])
        return recorded_error(review_mode, str(exc), cv_result=loaded_packet)
    try:
        recorded = mobius.record_cv_result(
            root,
            session_id,
            goal_slug,
            cv_result,
            review_mode,
            target_plan_item_id=target_plan_item_id,
            target_acceptance_ids=[str(item) for item in ids] if review_mode == "delta_review" else None,
        )
        if attempt_id:
            mobius.review_attempt_finished(goal_dir, attempt_id, "recorded", str(recorded.get("cv_id", "")))
        return recorded
    except KeyboardInterrupt:
        if attempt_id:
            mobius.review_attempt_finished(goal_dir, attempt_id, "interrupted")
        raise
    except mobius.MobiusError as exc:
        if attempt_id:
            mobius.review_attempt_finished(goal_dir, attempt_id, "failed", failure_kind="mobius_error", retryable=False, diagnostic_ref=str(exc)[:160])
        return recorded_error(review_mode, str(exc), cv_result=cv_result)
    except OSError as exc:
        if attempt_id:
            mobius.review_attempt_finished(goal_dir, attempt_id, "failed", failure_kind="persistence_error", retryable=True, diagnostic_ref=str(exc)[:160])
        return recorded_error(review_mode, f"persistence error: {exc}", cv_result=cv_result)


@mcp.tool()
def mobius_cv_health(deep: bool = False, include_commands: bool = True) -> dict[str, Any]:
    """Check reviewer availability. Prompt smoke status is cached from MCP startup."""
    kimi = discover_kimi(deep=deep)
    if isinstance(STARTUP_HEALTH, dict):
        kimi["startup_connectivity"] = STARTUP_HEALTH.get("startup_connectivity", {})
        kimi["startup_status"] = STARTUP_HEALTH.get("status", "unknown")
    if not include_commands:
        kimi = dict(kimi)
        kimi.pop("commands", None)
        kimi.pop("checks", None)
    return {
        "schema": "mobius.cv_health",
        "server": {"name": "mobius-cv", "version": SERVER_VERSION},
        "stateless_reviews": True,
        "started_at": STARTED_AT,
        "checked_at": now_iso(),
        "startup_health": STARTUP_HEALTH,
        "reviewers": [
            {
                "id": "codex-subagent",
                "kind": "host_mediated",
                "status": "requires_host_subagent_bridge",
                "available": False,
                "reason": "MCP servers cannot directly invoke the current Codex host subagent tool.",
            },
            kimi,
        ],
        "status": "ready" if STARTUP_HEALTH.get("status") == "ready" else "degraded",
    }


@mcp.tool()
def mobius_cv_registry() -> dict[str, Any]:
    """Return registered reviewer adapters and supported review modes."""
    return {
        "schema": "mobius.cv_registry",
        "reviewers": [
            {
                "id": "codex-subagent",
                "kind": "host_mediated",
                "levels": [1, 2],
                "stateless_contract": REVIEWER_SCHEMA,
            },
            {
                "id": "kimi-code",
                "kind": "cli",
                "command": "kimi -p <prompt> --output-format stream-json",
                "levels": [2],
                "stateless_contract": REVIEWER_RESULT_START,
            },
        ],
        "review_modes": sorted(VALID_REVIEW_MODES),
        "review_policies": [
            mobius.review_gate_policy("delta_review", {"name": "delta_light"}),
            mobius.review_gate_policy("delta_review", {"name": "delta_kimi"}),
            mobius.review_gate_policy("exit_review", {"name": "exit_strict"}),
        ],
        "recorded_review_tools": ["mobius_cv_record_delta_review", "mobius_cv_record_exit_review"],
    }


@mcp.tool()
def mobius_cv_build_subagent_prompt(
    packet: dict[str, Any] | None = None,
    review_mode: str = "exit_review",
    required_acceptance_ids: list[str] | None = None,
) -> dict[str, Any]:
    """Build a stateless level-1 Codex subagent prompt from a frozen Mobius index packet."""
    if review_mode not in VALID_REVIEW_MODES:
        return {"schema": "mobius.cv_error", "ok": False, "error": f"invalid review_mode: {review_mode}"}
    loaded_packet, loaded_text, errors = load_packet(packet)
    if errors:
        return {"schema": "mobius.cv_error", "ok": False, "error": "; ".join(errors)}
    if loaded_packet is None:
        return {"schema": "mobius.cv_error", "ok": False, "error": "packet JSON object is required"}
    required_ids = extract_required_acceptance_ids(loaded_packet, required_acceptance_ids)
    review_contract_view = review_contract_view_from_input(packet)
    return {
        "schema": "mobius.cv_subagent_prompt",
        "review_mode": review_mode,
        "stateless": True,
        "prompt": review_prompt(loaded_text, review_mode, required_ids, "codex-subagent", review_contract_view=review_contract_view),
        "expected_result_schema": REVIEWER_SCHEMA,
    }

@mcp.tool()
def mobius_cv_record_delta_review(
    project_root: str,
    session_id: str,
    goal_slug: str,
    target_plan_item_id: str,
    target_acceptance_ids: list[str] | None = None,
    packet: dict[str, Any] | None = None,
    packet_id: str | None = None,
    level: int = 1,
    codex_subagent_result: dict[str, Any] | str | None = None,
    required_acceptance_ids: list[str] | None = None,
    timeout_seconds: int = KIMI_HARD_TIMEOUT_SECONDS,
    cv_id: str | None = None,
    input_refs: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Run a stateless delta review and persist its stage gate state through Mobius."""
    return review_and_record(
        "delta_review",
        project_root,
        session_id,
        goal_slug,
        packet,
        packet_id,
        level,
        codex_subagent_result,
        required_acceptance_ids,
        target_plan_item_id,
        target_acceptance_ids,
        timeout_seconds,
        cv_id,
        input_refs,
    )


@mcp.tool()
def mobius_cv_record_exit_review(
    project_root: str,
    session_id: str,
    goal_slug: str,
    packet: dict[str, Any] | None = None,
    packet_id: str | None = None,
    level: int = 2,
    codex_subagent_result: dict[str, Any] | str | None = None,
    required_acceptance_ids: list[str] | None = None,
    timeout_seconds: int = KIMI_HARD_TIMEOUT_SECONDS,
    cv_id: str | None = None,
    input_refs: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Run a stateless exit review, persist CV state, update acceptance fields, and compute verdict."""
    return review_and_record(
        "exit_review",
        project_root,
        session_id,
        goal_slug,
        packet,
        packet_id,
        level,
        codex_subagent_result,
        required_acceptance_ids,
        None,
        None,
        timeout_seconds,
        cv_id,
        input_refs,
    )


def startup_health() -> dict[str, Any]:
    if os.environ.get(KIMI_CHILD_ENV) == "1":
        return {
            "id": "kimi-code",
            "kind": "cli",
            "command": "kimi",
            "path": shutil.which("kimi"),
            "available": False,
            "status": "disabled_in_kimi_child",
            "checks": [],
            "commands": [],
            "supports": {"prompt": False, "stream_json": False},
            "startup_connectivity": {
                "status": "skipped",
                "valid": False,
                "prompt": KIMI_CONNECTIVITY_PROMPT,
                "signal": "disabled because MobiusCV was launched inside a Kimi child process",
            },
        }
    return discover_kimi(deep=env_enabled("MOBIUS_CV_STARTUP_SMOKE", default=False))


STARTUP_HEALTH = startup_health()


if __name__ == "__main__":
    mcp.run(transport="stdio")
