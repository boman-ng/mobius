"""Release-facing validation for the Mobius plugin bundle."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
PLUGIN_ROOT = REPO_ROOT / "plugins" / "mobius"


def rel(path: Path) -> str:
    return path.relative_to(REPO_ROOT).as_posix()


def read_json(path: Path) -> dict[str, object]:
    data = json.loads(path.read_text(encoding="utf-8"))
    assert isinstance(data, dict), f"{rel(path)} must contain a JSON object"
    return data


def release_text_paths() -> list[Path]:
    excluded_parts = {".git", ".mobius", ".venv", "__pycache__"}
    paths: list[Path] = []
    for path in REPO_ROOT.rglob("*"):
        if not path.is_file():
            continue
        rel_parts = path.relative_to(REPO_ROOT).parts
        if any(part in excluded_parts for part in rel_parts):
            continue
        try:
            path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        paths.append(path)
    return paths


def public_text_paths() -> list[Path]:
    roots = [
        README.md if False else REPO_ROOT / "README.md",
        REPO_ROOT / "CHANGELOG.md",
        REPO_ROOT / "docs",
        PLUGIN_ROOT / "skills",
        PLUGIN_ROOT / "references",
        PLUGIN_ROOT / "scripts",
        PLUGIN_ROOT / ".codex-plugin" / "plugin.json",
        PLUGIN_ROOT / ".mcp.json",
        PLUGIN_ROOT / "pyproject.toml",
    ]
    paths: list[Path] = []
    for root in roots:
        if root.is_file():
            paths.append(root)
        elif root.is_dir():
            paths.extend(path for path in root.rglob("*") if path.is_file())
    return paths


def test_plugin_source_tree_has_no_generated_runtime_artifacts() -> None:
    generated_dirs = [path for path in PLUGIN_ROOT.rglob("*") if path.is_dir() and path.name in {".venv", "__pycache__"}]
    generated_files = [path for path in PLUGIN_ROOT.rglob("*.pyc") if path.is_file()]
    assert generated_dirs == []
    assert generated_files == []


def test_required_plugin_bundle_files_exist() -> None:
    required_paths = [
        ".codex-plugin/plugin.json",
        ".mcp.json",
        "hooks/hooks.json",
        "skills/mobius-plan/SKILL.md",
        "skills/mobius-loop/SKILL.md",
        "skills/mobius-plan/agents/openai.yaml",
        "skills/mobius-loop/agents/openai.yaml",
        "scripts/mobius.py",
        "scripts/mobius_review_mcp.py",
        "scripts/mobius_review_mcp_server.sh",
        "scripts/mobius_hook_launcher.sh",
        "pyproject.toml",
        "uv.lock",
        "references/data-contracts.md",
        "references/review-mcp.md",
        "references/hooks.md",
    ]
    missing = [path for path in required_paths if not (PLUGIN_ROOT / path).is_file()]
    assert missing == []
    assert not (PLUGIN_ROOT / "scripts" / ("mobius_" + "cv_mcp.py")).exists()


def test_development_only_files_are_not_in_plugin_bundle() -> None:
    forbidden_plugin_source_paths = [
        "scripts/" + "verify.sh",
        "scripts/mobius_regression_tests.py",
        "tests/mobius_regression_tests.py",
        "tests/test_release_bundle.py",
        "requirements-dev.txt",
    ]
    present = [path for path in forbidden_plugin_source_paths if (PLUGIN_ROOT / path).exists()]
    assert present == []


def test_plugin_manifest_marketplace_mcp_and_hooks_shape() -> None:
    expected_owner = "boman-ng"
    expected_slug = f"{expected_owner}/mobius"
    expected_repo_url = f"https://github.com/{expected_slug}"

    manifest = read_json(PLUGIN_ROOT / ".codex-plugin/plugin.json")
    assert manifest.get("name") == "mobius"
    assert manifest.get("version") == "0.5.0"
    assert manifest.get("license") == "Apache-2.0"
    assert manifest.get("skills") == "./skills/"
    assert manifest.get("mcpServers") == "./.mcp.json"
    assert manifest.get("repository") == expected_repo_url
    assert manifest.get("homepage") == f"{expected_repo_url}#readme"

    mcp = read_json(PLUGIN_ROOT / ".mcp.json")
    mcp_servers = mcp.get("mcpServers")
    assert isinstance(mcp_servers, dict)
    server = mcp_servers.get("mobius-review")
    assert isinstance(server, dict)
    assert server.get("cwd") == "."
    assert server.get("command") == "/bin/bash"
    assert server.get("args") == ["./scripts/mobius_review_mcp_server.sh"]

    hooks = read_json(PLUGIN_ROOT / "hooks/hooks.json").get("hooks")
    assert isinstance(hooks, dict)
    for event in ("PreToolUse", "Stop"):
        entries = hooks.get(event)
        assert isinstance(entries, list) and entries
        rendered = json.dumps(entries)
        assert "mobius_hook_launcher.sh" in rendered
        assert "PLUGIN_ROOT missing" in rendered
        command = entries[0]["hooks"][0]["command"]
        with tempfile.TemporaryDirectory() as tmp:
            env = os.environ.copy()
            env["PLUGIN_ROOT"] = str(Path(tmp) / ".codex" / "plugins" / "cache" / "mobius" / "mobius" / "0.5.0")
            stale = subprocess.run(command, input="{}", text=True, shell=True, executable="/bin/bash", capture_output=True, env=env, check=False)
        assert stale.returncode == 0
        assert "hook-unavailable: installed plugin cache missing" in stale.stderr

    marketplace = read_json(REPO_ROOT / ".agents/plugins/marketplace.json")
    plugins = marketplace.get("plugins")
    assert marketplace.get("name") == "mobius"
    assert isinstance(plugins, list)
    entry = next((item for item in plugins if isinstance(item, dict) and item.get("name") == "mobius"), None)
    assert isinstance(entry, dict)
    assert entry.get("source", {}).get("path") == "./plugins/mobius"
    assert entry.get("policy", {}).get("installation") == "AVAILABLE"


def test_plugin_bundle_exposes_doctor_and_explicit_skill_activation() -> None:
    uv_path = shutil.which("uv")
    assert uv_path, "uv is required for the Review MCP doctor readiness check"
    env = os.environ.copy()
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env["MOBIUS_REVIEW_UV"] = uv_path
    result = subprocess.run(
        ["python3", str(PLUGIN_ROOT / "scripts/mobius.py"), "doctor"],
        text=True,
        capture_output=True,
        env=env,
        check=False,
    )
    payload = json.loads(result.stdout)
    assert payload["command"] == "doctor"
    assert payload["mcp"]["server"] == "mobius-review"
    assert payload["mcp"]["uv_required"] is True
    assert result.returncode == 0
    assert payload["mcp"]["start_ready"] is True
    assert payload["mcp"]["self_check"]["status"] == "ready"

    bad_env = {**env, "MOBIUS_REVIEW_UV": "/bin/false"}
    bad_result = subprocess.run(
        ["python3", str(PLUGIN_ROOT / "scripts/mobius.py"), "doctor"],
        text=True,
        capture_output=True,
        env=bad_env,
        check=False,
    )
    bad_payload = json.loads(bad_result.stdout)
    assert bad_result.returncode == 2
    assert bad_payload["mcp"]["start_ready"] is False
    assert any("Review MCP self-check failed" in error for error in bad_payload["errors"])

    plan_skill = (PLUGIN_ROOT / "skills/mobius-plan/SKILL.md").read_text(encoding="utf-8")
    loop_skill = (PLUGIN_ROOT / "skills/mobius-loop/SKILL.md").read_text(encoding="utf-8")
    for text in (plan_skill, loop_skill):
        assert "Use this skill only" in text
        assert "ordinary" in text
        assert "Objective" in text


def test_public_v0_5_surface_uses_canonical_terms() -> None:
    required_terms = [
        "Objective",
        "Work Item",
        "Criterion",
        "Route",
        "Route Run",
        "Timebox",
        "Evidence",
        "Review Target",
        "Review Judgment",
        "Review Feedback",
        "Verdict",
    ]
    public_text = "\n".join(path.read_text(encoding="utf-8") for path in public_text_paths())
    for term in required_terms:
        assert term in public_text

    help_text = subprocess.run(
        ["python3", str(PLUGIN_ROOT / "scripts/mobius.py"), "--help"],
        text=True,
        capture_output=True,
        check=True,
    ).stdout
    assert "objective-start" in help_text
    assert "contract-add-work-item" in help_text
    assert "review-target-create" in help_text
    assert "review-judgment-record" in help_text

    def command(name: str) -> str:
        return rf"\b{name}\b"

    def csv_name(name: str) -> str:
        return rf"\b{name}\.csv\b"

    forbidden_patterns = [
        command("go" + "al-" + "start"),
        command("contract-add-" + "st" + "age"),
        command("pack" + "et-" + "create"),
        command("delta_" + "review"),
        command("goal_" + "slug"),
        command("plan_item_" + "id"),
        csv_name("accept" + "ance"),
        csv_name("pack" + "ets"),
        csv_name("c" + "v"),
        csv_name("review_" + "attempts"),
        command("Mobius" + "C" + "V"),
        command("mobius-" + "c" + "v"),
        command("MOBIUS_" + "CV_REVIEWER_RESULT"),
        "mobius_" + "cv_mcp",
    ]
    searchable = public_text + "\n" + help_text
    for pattern in forbidden_patterns:
        assert re.search(pattern, searchable) is None, pattern


def test_release_text_has_no_forbidden_local_or_stale_tokens() -> None:
    expected_owner = "boman-ng"
    expected_slug = f"{expected_owner}/mobius"
    linux_home_prefix = "/" + "home" + "/"
    mac_home_prefix = "/" + "Users" + "/"
    win_home_prefix = "C:" + "\\" + "Users" + "\\"
    personal_path_patterns = [
        (re.compile(re.escape(linux_home_prefix) + r"[A-Za-z0-9_.-]+"), "Linux home path"),
        (re.compile(re.escape(mac_home_prefix) + r"[A-Za-z0-9_.-]+"), "macOS home path"),
        (re.compile(re.escape(win_home_prefix) + r"[A-Za-z0-9_.-]+"), "Windows home path"),
    ]

    stale_owner = "wu" + "bw"
    forbidden_tokens = [
        f"github.com/{stale_owner}",
        f"{stale_owner}/mobius",
        linux_home_prefix + stale_owner,
        ".codex/plugins/cache/" + "personal",
        "0.1.0+" + "codex",
        "bash scripts/" + "verify.sh",
        "scripts/" + "verify.sh",
    ]
    marketplace_add = "codex plugin " + "marketplace add "

    for path in release_text_paths():
        text = path.read_text(encoding="utf-8")
        path_rel = rel(path)
        for forbidden in forbidden_tokens:
            assert forbidden not in text, f"{path_rel} contains forbidden token {forbidden}"
        for pattern, label in personal_path_patterns:
            assert pattern.search(text) is None, f"{path_rel} contains a hardcoded {label}"
        for match in re.finditer(re.escape(marketplace_add) + r"([^\s]+)", text):
            target = match.group(1)
            assert target.startswith("/") or target == expected_slug, (
                f"{path_rel} contains unexpected marketplace coordinate {target}"
            )
        for match in re.finditer(r"https://github\.com/([A-Za-z0-9_.-]+)(?:/([A-Za-z0-9_.-]+))?", text):
            owner = match.group(1)
            repo_name = match.group(2)
            assert owner == expected_owner and (repo_name is None or repo_name == "mobius"), (
                f"{path_rel} contains unexpected GitHub URL owner or repository"
            )


def test_local_mobius_state_is_ignored_by_git() -> None:
    gitignore = (REPO_ROOT / ".gitignore").read_text(encoding="utf-8")
    assert ".mobius/" in gitignore
