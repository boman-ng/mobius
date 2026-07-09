# Mobius

Mobius is a local-only Codex plugin for explicit objective-and-loop work. It creates a locked
Objective contract with `mobius-plan`, then `mobius-loop` drives Work Items through Route Runs,
objective Evidence, Review Targets, recorded Review Judgments, and a derived Verdict.

Mobius is for work where "done" should be backed by local ledgers instead of agent confidence. It
does not send project state to a hosted Mobius service.

## What It Includes

- `mobius-plan`: creates, validates, and locks an explicit Objective contract.
- `mobius-loop`: executes the locked Objective through Route Runs, Evidence, Review Targets, and
  recorded Review Judgments.
- `mobius-review`: a bundled stdio MCP server that records checkpoint and exit Review Judgments.
- Lifecycle hooks: narrow local guardrails for protected Mobius ledgers and false terminal claims.

## Requirements

- Codex with plugin, skill, MCP, and hook support.
- Python 3.11 or newer.
- `uv` available on `PATH`, or `MOBIUS_REVIEW_UV` set to an absolute `uv` executable.

## Install From This Repository

```bash
codex plugin marketplace add boman-ng/mobius --ref v0.5.0 --sparse .agents/plugins --sparse plugins --sparse LICENSE
codex plugin add mobius@mobius
```

After installing or updating, start a new Codex thread so bundled skills, MCP config, and hooks are
loaded from the installed plugin cache.

## Basic Usage

Create and lock an Objective contract:

```text
$mobius:mobius-plan <objective description>
```

Run the full loop:

```text
$mobius:mobius-loop <objective-slug>
```

Mobius records local execution state outside source control. This repository ignores `.mobius/`
because it is local evidence and ledger data, not release source.

## Model

Mobius keeps one normal path:

1. Lock an Objective with ordered Work Items and Criteria.
2. Select a Route and start a Route Run for the next Work Item.
3. Record objective Evidence for each Criterion.
4. Create a one-shot Review Target from current ledgers.
5. Record a checkpoint Review Judgment as feedback for the Route Run.
6. Record an exit Review Judgment before the final Verdict can become `accepted`.

Review feedback is not the unit of failure. A Route Run can keep receiving feedback until its
Timebox expires, no viable Route remains, a required tool or reviewer is unavailable, or the user
stops the loop. `budget.csv` records metered harness-internal time separately from external or
detached work. Codex session imports preserve the finest defensible timing precision in the
`source` cell instead of inventing missing durations.

## Verification

From a checkout:

```bash
python -m pip install -r requirements-dev.txt
PYTHONDONTWRITEBYTECODE=1 python -m pytest
```

The pytest suite checks ledger contracts, Review Target/Judgment recording, time accounting,
manifest shape, MCP launcher self-check, hook health, generated-file hygiene, and ignored local
Mobius state.

## Troubleshooting

If the Review MCP self-check cannot find `uv`, install `uv` or set `MOBIUS_REVIEW_UV` to the
absolute executable path:

```bash
MOBIUS_REVIEW_UV=/absolute/path/to/uv \
  PLUGIN_DATA="$(mktemp -d)" \
  plugins/mobius/scripts/mobius_review_mcp_server.sh --self-check
```

If Codex reports changed hooks after install or upgrade, run `/hooks`, inspect the Mobius hook
definitions, and trust only the version you intend to run.

## Current Limits

- Official public Plugin Directory publishing is not self-serve yet; this repository uses a repo
  marketplace for distribution.
- Exit review requires a valid non-degraded Review Judgment.
- Mobius hooks guard Mobius state only. They do not replace tests, CI, code review, or repository
  security controls.

## License

Apache-2.0. See `LICENSE`.
