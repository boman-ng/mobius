# Release Checklist

Use this checklist before tagging `v0.2.0`.

## Verify

Run the release gate from a clean checkout:

```bash
bash scripts/verify.sh
git diff --check && git status --short --ignored=no
```

The first command must pass. Inspect the second command output before tagging: no local Mobius
ledger state should appear, and all release files should be intentionally staged or committed.

## Review Release Contents

- Confirm `plugins/mobius/.codex-plugin/plugin.json` has version `0.2.0`, repository metadata,
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
- Confirm `scripts/verify.sh` runs the regression suite, syntax checks, manifest/marketplace
  validation, MCP launcher self-check, hook health, git hygiene, and generated-file checks with
  exact generated-artifact diagnostics.
- Confirm release-facing docs describe the compact packet model, replayable evidence metadata,
  fail-closed MobiusCV behavior, retryable/non-retryable reviewer infrastructure failures,
  repairable final-evidence refresh, and raw review retention policy.
- Confirm no `.mobius` ledger state, generated environments, bytecode, local cache paths, personal
  home paths, or secrets are visible in `git status --short --ignored=no`.

## Tag

```bash
git tag -a v0.2.0 -m "Mobius v0.2.0"
git push origin v0.2.0
```

## Install Or Refresh

Install from the pinned tag:

```bash
codex plugin marketplace add boman-ng/mobius --ref v0.2.0 --sparse .agents/plugins --sparse plugins --sparse LICENSE
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
