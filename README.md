# Mobius

Mobius is a local-only Codex plugin for explicit plan-and-loop work. It creates a locked goal
contract with `mobius-plan`, then `mobius-loop` drives each stage through objective evidence,
recorded review, and final exit review before acceptance.

Mobius is meant for work where "done" should be backed by a small local ledger instead of agent
confidence. It does not send project state to a hosted Mobius service.

For explicitly targeted goals, Mobius makes the plan, evidence, feedback, review, stop conditions,
and blind-spot checks visible so completion depends on recorded gates rather than an agent's
confidence narrative.

## What It Includes

- `mobius-plan`: creates, validates, and locks an explicit goal contract.
- `mobius-loop`: executes a locked plan through evidence, packet, review, and verdict gates.
- `mobius-cv`: a bundled stdio MCP server that records MobiusCV delta and exit reviews.
- Lifecycle hooks: local guardrails for protected Mobius state and false completion claims.

## Requirements

- Codex with plugin, skill, MCP, and hook support.
- Python 3.11 or newer.
- `uv` available on `PATH`, or `MOBIUS_CV_UV` set to an absolute `uv` executable.
- Kimi CLI access only for review policies that require Kimi, including strict exit review.

## Install From This Repository

Add the repository marketplace, then install the plugin:

```bash
codex plugin marketplace add boman-ng/mobius --ref v0.2.0 --sparse .agents/plugins --sparse plugins --sparse LICENSE
codex plugin add mobius@mobius
```

After installing or updating, start a new Codex thread so bundled skills, MCP config, and hooks are
loaded from the installed plugin cache.

## Local Development

From a source checkout, add the checkout as a marketplace source:

```bash
codex plugin marketplace add /path/to/mobius
codex plugin add mobius@mobius
```

## Trust And Enablement

Installing the plugin makes its skills and MCP config available, but Codex approval settings still
apply. Plugin-bundled hooks are non-managed hooks and must be reviewed and trusted through Codex
before they run. Use `/hooks` to inspect changed hook definitions after install or upgrade.

The bundled MCP server runs locally through `./scripts/mobius_cv_mcp_server.sh`. Codex starts it
from the installed plugin root. The launcher uses `PLUGIN_DATA` for its `uv` environment when
available, and otherwise falls back to a user cache path; see
`plugins/mobius/references/mobiuscv-mcp.md`.

## Basic Usage

Create and lock a contract:

```text
$mobius:mobius-plan <goal description>
```

Run the full locked plan by passing the Mobius `plan.csv` path for that goal:

```text
$mobius:mobius-loop <path-to-plan.csv>
```

Mobius records project-local execution state outside source control. This repository ignores that
state because it is local evidence and ledger data, not release source.

## Review And Evidence Guarantees

Mobius keeps one normal path for a goal:

1. Lock a plan and acceptance matrix.
2. Record objective evidence with compact structured metadata.
3. Create a frozen packet index from local ledgers.
4. Run a stateless MobiusCV review for each stage.
5. Run strict exit review before final acceptance.

Packets are compact indexes, not evidence archives. They contain ledger refs and short hash tails;
full command output, diffs, file contents, and full hashes remain local. Command and test evidence
can include replay metadata such as command string, argv, cwd, selected environment metadata, exit
code, duration, output refs, `recorded_at`, and optional `validity_scope` values for `stage`,
`final`, or `historical` proof.

MobiusCV fails closed. Missing, invalid, unchecked, degraded, or unavailable reviewer output is not
recorded as a passing CV judgment. Reviewer infrastructure failures remain retryable review
attempts when retryable and do not consume a packet as reviewed; non-retryable reviewer
infrastructure failures are surfaced as compact `review_attempts.csv` diagnostics. Repairable final
review blockers such as stale `file_ref` hashes or generated Python artifacts route back to final
evidence refresh instead of making the goal terminal. Passing review rows store compact raw-output
hash metadata by default; non-pass rows retain local `raw_reviews/*.json` artifacts for audit.

## Verification

From a checkout:

```bash
bash scripts/verify.sh
```

The verification script checks Python syntax, regression tests, plugin manifest shape, MCP launcher
self-check, hook health, marketplace metadata, and ignored local Mobius state.

## Troubleshooting

If the MCP self-check cannot find `uv`, install `uv` and make sure it is on `PATH`, or set
`MOBIUS_CV_UV` to the absolute path of the executable. From a source checkout, maintainers can run:

```bash
MOBIUS_CV_UV=/absolute/path/to/uv \
  PLUGIN_DATA="$(mktemp -d)" \
  plugins/mobius/scripts/mobius_cv_mcp_server.sh --self-check
```

If Codex reports changed hooks after install or upgrade, run `/hooks`, review the Mobius hook
definitions, and trust only the version you intend to run. The hooks are expected to guard local
Mobius state and false completion claims; they should not ask for project secrets or external
credentials.

If `mobius-loop` reaches a review gate and reports that Kimi is required, configure Kimi CLI access
or choose a Mobius review policy that does not require Kimi before rerunning the gate. Strict exit
review cannot pass by treating a missing reviewer as success.

If the MCP server still fails after `uv` is available, rerun the source-checkout self-check with a
clean plugin data directory and inspect the generated error:

```bash
PLUGIN_DATA="$(mktemp -d)" plugins/mobius/scripts/mobius_cv_mcp_server.sh --self-check
```

## Current Limits

- Official public Plugin Directory publishing is not self-serve yet; this repository uses a repo
  marketplace for distribution.
- Exit review requires all policy-required reviewers to return valid non-degraded results.
- Mobius hooks guard Mobius state only. They do not replace tests, CI, code review, or repository
  security controls.

## License

Apache-2.0. See `LICENSE`.
