# Mobius Plugin Development Instructions

This file extends higher-level Codex instructions for this repository. It is for developing,
testing, and releasing the Mobius Codex plugin source only. Do not treat it as end-user behavior,
skill content, marketplace metadata, or runtime policy after installation.

## Project Map

- Plugin source: `plugins/mobius/`.
- Plugin manifest: `plugins/mobius/.codex-plugin/plugin.json`.
- Marketplace entry: `.agents/plugins/marketplace.json`.
- Skills: `plugins/mobius/skills/mobius-plan/SKILL.md` and
  `plugins/mobius/skills/mobius-loop/SKILL.md`.
- Local state engine and hook health: `plugins/mobius/scripts/mobius.py`.
- MobiusCV MCP server and launcher: `plugins/mobius/scripts/mobius_cv_mcp.py` and
  `plugins/mobius/scripts/mobius_cv_mcp_server.sh`.
- Hook launcher and hook definitions: `plugins/mobius/scripts/mobius_hook_launcher.sh` and
  `plugins/mobius/hooks/hooks.json`.
- Durable plugin references: `plugins/mobius/references/`.
- Test configuration: `pyproject.toml` and `requirements-dev.txt`.
- Regression tests: `tests/mobius_regression_tests.py`.
- Release bundle tests: `tests/test_release_bundle.py`.
- CI release gate: `.github/workflows/ci.yml`.
- Release docs and repository-level files: `README.md`, `docs/`, `CHANGELOG.md`,
  `CONTRIBUTING.md`, `SECURITY.md`, `LICENSE`, `.github/`, and `.gitignore`.

## Source Of Truth

- `plugins/mobius/scripts/mobius.py` owns Mobius local ledger semantics, plan locking, stage
  transitions, hook checks, and acceptance conditions.
- `plugins/mobius/scripts/mobius_cv_mcp.py` owns the MobiusCV MCP contract and review recording
  behavior.
- `plugins/mobius/.codex-plugin/plugin.json` owns plugin identity, version, display metadata,
  skills path, and MCP entrypoint.
- `plugins/mobius/.mcp.json` owns the plugin-bundled MCP launch command. Keep paths relative to
  the installed plugin root.
- `plugins/mobius/hooks/hooks.json` owns plugin-bundled hook registration. Keep hooks local to
  Mobius state and completion-claim guardrails.
- `plugins/mobius/references/` documents behavior that skills, hooks, MCP, and tests rely on.

Before changing plugin behavior, update the owning source and the relevant reference or release
documentation in the same change when the public contract changes.

## Release Contract

- Public repository coordinates are `boman-ng/mobius`.
- The plugin name is `mobius`; keep the marketplace entry and manifest aligned.
- Keep release URLs under `https://github.com/boman-ng/mobius`.
- Do not commit personal usernames, personal home paths, local cache paths, machine-specific
  absolute paths, generated virtual environments, bytecode, or local Mobius ledger state.
- `.mobius/` is project-local execution evidence and must remain ignored by Git.
- The release source must install from a repository marketplace until an official public plugin
  directory flow is available.
- Development and verification files must stay outside the installed plugin bundle unless they are
  required by runtime.

## Development Rules

- Keep plugin paths relative and portable. Installed plugins run from a cache location that differs
  from the development checkout.
- Keep Mobius local-only. Do not add hosted service assumptions, external telemetry, or network
  dependencies for core plan, loop, hook, or review-recording behavior.
- Fail closed at review gates. Missing, invalid, unchecked, or degraded reviewer output must not be
  converted into success.
- Preserve explicit user intent. Mobius skills should activate only for explicitly targeted Mobius
  goals, not ordinary Codex tasks.
- Keep hooks narrow. Hooks may protect Mobius state and false completion claims, but must not
  become a general repository policy engine.
- Prefer updating the existing owner over adding parallel scripts, aliases, shims, fallback paths,
  or duplicate state paths.
- Keep generated runtime files out of the plugin source tree. Use temporary directories or
  `PLUGIN_DATA` for launcher checks.

## Commit Rules

- Use Conventional Commits; each commit must be atomic and contain no more than four distinct changes.

## Verification Matrix

| Change type | Required checks |
|---|---|
| Manifest, marketplace, hooks, MCP config, skills, scripts, or release docs | `PYTHONDONTWRITEBYTECODE=1 python -m pytest`; `PYTHONPYCACHEPREFIX="$(mktemp -d)" python -m py_compile plugins/mobius/scripts/mobius.py plugins/mobius/scripts/mobius_cv_mcp.py tests/mobius_regression_tests.py tests/test_release_bundle.py`; `PYTHONDONTWRITEBYTECODE=1 python plugins/mobius/scripts/mobius.py --project-root "$PWD" hook-health`; `git diff --check` |
| Python script change | `PYTHONDONTWRITEBYTECODE=1 python -m pytest`; `PYTHONPYCACHEPREFIX="$(mktemp -d)" python -m py_compile plugins/mobius/scripts/mobius.py plugins/mobius/scripts/mobius_cv_mcp.py tests/mobius_regression_tests.py tests/test_release_bundle.py` |
| Plugin manifest shape only | `PYTHONDONTWRITEBYTECODE=1 python -m pytest tests/test_release_bundle.py` |
| Documentation only | Review rendered Markdown when tables, links, or command blocks change; run `PYTHONDONTWRITEBYTECODE=1 python -m pytest tests/test_release_bundle.py` when release commands, paths, or repository coordinates change |

Use the narrowest meaningful check first, but run the full pytest suite plus syntax, hook-health,
and Git hygiene checks before treating a release-facing change as complete.

## Review Before Finishing

- Inspect diffs for accidental scope expansion, personal paths, stale repository coordinates,
  hidden local state, and runtime behavior that depends on the development checkout.
- Confirm `plugins/mobius/.codex-plugin/plugin.json`, `plugins/mobius/.mcp.json`,
  `plugins/mobius/hooks/hooks.json`, and `.agents/plugins/marketplace.json` still describe one
  coherent install path.
- Confirm no change makes `AGENTS.md` part of the Mobius runtime contract; it is contributor
  guidance for this repository only.
