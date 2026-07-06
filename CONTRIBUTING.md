# Contributing

Mobius changes should preserve the local-only, explicit-goal state model.

## Development Setup

1. Install Python 3.11 or newer.
2. Install `uv` or set `MOBIUS_CV_UV` to an absolute `uv` executable path.
3. Work from the repository root.

## Verification

Run the narrow checks for your change, and run the release gate before opening a release PR:

```bash
python3 tests/mobius_regression_tests.py
python3 -m py_compile plugins/mobius/scripts/mobius.py plugins/mobius/scripts/mobius_cv_mcp.py tests/mobius_regression_tests.py
PLUGIN_DATA="$(mktemp -d)" plugins/mobius/scripts/mobius_cv_mcp_server.sh --self-check
bash scripts/verify.sh
```

## Rules For Changes

- Do not hand-edit local Mobius ledgers.
- Do not commit local Mobius state, virtual environments, caches, or bytecode.
- Keep skill workflows focused and explicit.
- Keep MCP and hook behavior local, deterministic, and documented.
- Document any user-visible behavior or trust-boundary change.

## Pull Requests

Explain the user-facing change, evidence recorded, verification commands, and any residual risk.
Security-sensitive changes should include the relevant threat model or failure mode.
