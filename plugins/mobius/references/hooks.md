# Mobius Hooks

Mobius hooks are deterministic guardrails around explicit `.mobius/` state. They do not plan,
advance Route Runs, call reviewers, validate ordinary reads, or decide the Verdict.

The plugin ships one hook configuration:

```text
hooks/hooks.json
scripts/mobius_hook_launcher.sh
```

Hook commands launch Mobius through `${PLUGIN_ROOT}` so installed plugin cache paths work without
user-specific assumptions.

## Hook Commands

```text
/bin/sh -c ... "${PLUGIN_ROOT}" hook pre-tool-use
  Blocks direct writes to protected Mobius ledgers.
  Allows read-only inspection of those ledgers.

/bin/sh -c ... "${PLUGIN_ROOT}" hook stop
  Blocks terminal claims only when the final text explicitly targets the same Objective and the
  local Verdict is not accepted.
```

Protected ledgers are:

```text
run.csv
objective.csv
work_items.csv
criteria.csv
routes.csv
route_runs.csv
budget.csv
evidence.csv
review_targets.csv
review_judgments.csv
review_runs.csv
verdict.csv
```

Hooks are no-op for ordinary work even if the plugin is enabled or `.mobius/` exists. Contract
validation, Review Target creation, Review Judgment recording, and Verdict derivation remain owned
by the Mobius CLI and Review MCP.

## Health

```bash
python3 <mobius-plugin-root>/scripts/mobius.py doctor
python3 <mobius-plugin-root>/scripts/mobius.py --project-root /tmp hook-health
```

`doctor` reports plugin root, Review MCP launcher readiness, hook file presence, and `uv`
discovery. `hook-health` checks only hook registration shape and protected-ledger names.
