# Release Checklist

Use this checklist before tagging `v0.3.0`.

## Verify

Run the release gate from a clean checkout:

```bash
python -m pip install -r requirements-dev.txt
PYTHONDONTWRITEBYTECODE=1 python -m pytest
PYTHONPYCACHEPREFIX="$(mktemp -d)" python -m py_compile \
  plugins/mobius/scripts/mobius.py \
  plugins/mobius/scripts/mobius_cv_mcp.py \
  tests/mobius_regression_tests.py \
  tests/test_release_bundle.py
python plugins/mobius/scripts/mobius.py --project-root "$PWD" hook-health
git diff --check && git status --short --ignored=no
```

All commands must pass. Inspect the final status output before tagging: no local Mobius
ledger state should appear, and all release files should be intentionally staged or committed.

## Review Release Contents

- Confirm `plugins/mobius/.codex-plugin/plugin.json` has version `0.3.0`, repository metadata,
  `Apache-2.0` license metadata, `./skills/`, and `./.mcp.json`.
- Confirm `.agents/plugins/marketplace.json` points to `./plugins/mobius` with installation
  `AVAILABLE` and authentication `ON_INSTALL`.
- Confirm `README.md`, `SECURITY.md`, `CONTRIBUTING.md`, `CHANGELOG.md`, and
  `docs/official-docs-basis.md` match the release.
- Review `/hooks` after installing from the tag. Mobius hooks must remain local guardrails for
  Mobius state and false completion claims.
- Run the MCP launcher self-check with a clean `PLUGIN_DATA` directory if the installed cache has
  changed.

## Review Hardening Coverage

- Confirm the regression suite covers reviewer workspace/preflight, canonical packet recording,
  retryable reviewer infrastructure failures, loop action output, evidence schema ergonomics,
  status/explain diagnostics, evidence validity scope, and raw review retention.
- Confirm pytest covers manifest/marketplace validation, MCP launcher self-check, generated-file
  checks, release-source hygiene, and ignored local Mobius state.
- Confirm CI runs Python syntax, pytest, hook health, and Git hygiene as direct workflow steps
  rather than through a shell verification wrapper.
- Confirm release-facing docs describe the compact packet model, replayable evidence metadata,
  fail-closed MobiusCV behavior, retryable/non-retryable reviewer infrastructure failures,
  repairable final-evidence refresh, and raw review retention policy.
- Confirm no `.mobius` ledger state, generated environments, bytecode, local cache paths, personal
  home paths, or secrets are visible in `git status --short --ignored=no`.

## Tag

```bash
git tag -a v0.3.0 -m "Mobius v0.3.0"
git push origin v0.3.0
```

## Install Or Refresh

Install from the pinned tag:

```bash
codex plugin marketplace add boman-ng/mobius --ref v0.3.0 --sparse .agents/plugins --sparse plugins --sparse LICENSE
codex plugin add mobius@mobius
```

To refresh an existing Git marketplace snapshot, run:

```bash
codex plugin marketplace upgrade
codex plugin remove mobius@mobius
codex plugin add mobius@mobius
```

Start a new Codex thread after install or refresh so skills, MCP config, and hooks are loaded from
the installed plugin cache.

## Residual Limits

- This release is distributed through a repository marketplace, not an official public Plugin
  Directory listing.
- Strict exit review requires Kimi CLI access when the active Mobius policy requires Kimi.
- Mobius does not replace repository tests, CI, code review, or secret scanning.
