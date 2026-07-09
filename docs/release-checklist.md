# Release Checklist

Use this checklist before tagging `v0.5.0`.

## Verify

Run the release gate from a clean checkout:

```bash
python -m pip install -r requirements-dev.txt
PYTHONDONTWRITEBYTECODE=1 python -m pytest
PYTHONPYCACHEPREFIX="$(mktemp -d)" python -m py_compile \
  plugins/mobius/scripts/mobius.py \
  plugins/mobius/scripts/mobius_review_mcp.py \
  tests/mobius_regression_tests.py \
  tests/test_release_bundle.py
python plugins/mobius/scripts/mobius.py --project-root "$PWD" hook-health
git diff --check && git status --short --ignored=no
```

All commands must pass. Inspect the final status output before tagging: no local Mobius ledger state
should appear, and all release files should be intentionally staged or committed.

## Review Release Contents

- Confirm `plugins/mobius/.codex-plugin/plugin.json` has version `0.5.0`, repository metadata,
  `Apache-2.0` license metadata, `./skills/`, and `./.mcp.json`.
- Confirm `.agents/plugins/marketplace.json` points to `./plugins/mobius` with installation
  `AVAILABLE` and authentication `ON_INSTALL`.
- Confirm `README.md`, `SECURITY.md`, `CONTRIBUTING.md`, `CHANGELOG.md`, and
  `docs/official-docs-basis.md` match the release.
- Review `/hooks` after installing from the tag. Mobius hooks must remain local guardrails for
  protected Mobius ledgers and false terminal claims.
- Run the Review MCP launcher self-check with a clean `PLUGIN_DATA` directory.

## Review Coverage

- Confirm regression tests cover Objective creation, Work Item locking, Criterion evidence,
  Review Target one-shot behavior, Review Judgment recording, Route Run time accounting, and exit
  Verdict derivation.
- Confirm regression tests cover Codex session timing precision, mixed tool accounting, and
  Review Feedback routing.
- Confirm pytest covers manifest/marketplace validation, Review MCP launcher self-check, generated
  file checks, release-source hygiene, and ignored local Mobius state.
- Confirm release-facing docs describe the canonical v0.5 model and `budget.csv`.
- Confirm no `.mobius` ledger state, generated environments, bytecode, local cache paths, personal
  home paths, or secrets are visible in `git status --short --ignored=no`.

## Tag

```bash
git tag -a v0.5.0 -m "Mobius v0.5.0"
git push origin v0.5.0
```

## Install Or Refresh

```bash
codex plugin marketplace add boman-ng/mobius --ref v0.5.0 --sparse .agents/plugins --sparse plugins --sparse LICENSE
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
- Exit review requires a valid non-degraded Review Judgment.
- Mobius does not replace repository tests, CI, code review, or secret scanning.
