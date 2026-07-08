# Mobius Hooks

Mobius is a local-only, opt-in Codex plugin. Its hooks protect explicit Mobius goal state without
claiming control over ordinary repository work.

Mobius hooks are deterministic guardrails around explicit `.mobius/` state. They do not plan,
advance stages, call reviewers, validate ordinary reads, or decide acceptance. The loop engine
remains the Mobius CLI plus MobiusCV recorded review.

The plugin ships one hook configuration:

```text
hooks/hooks.json
scripts/mobius_hook_launcher.sh
```

Codex discovers that file for enabled plugins. Hook commands launch Mobius through
`${PLUGIN_ROOT}` so installed plugin cache paths work without user-specific assumptions.

## Skill/MCP/CLI/Hook Responsibility Boundary

| Surface | Owns | Must Not Own |
| --- | --- | --- |
| `mobius-plan` skill | User-facing workflow for creating, validating, and locking a complete contract. | Hidden policy decisions, CSV writes outside CLI, or alternate contract semantics. |
| `mobius-loop` skill | User-facing workflow for executing one locked stage, recording evidence, creating packets, and calling recorded review. | Manual interpretation of reviewer prose, manual loop advancement, or final acceptance. |
| MobiusCV MCP | Prompt building, host reviewer result normalization, Kimi adapter execution/parsing, reviewer result assembly, and calling CLI persistence APIs. | Direct CSV writes, terminal verdict computation, bypassing CLI validation, or relaxing packet/contract errors. |
| `mobius.py` CLI/ledger engine | CSV state transitions, contract validation, packet validation, one-shot packet enforcement, loop state, acceptance status, and verdict derivation. | Kimi process lifecycle or reviewer prompt/tool execution. |
| Hooks | Guardrails for direct protected-ledger writes and explicit false completion claims. | State mutation, reviewer execution, goal inference from ambient `.mobius` state, loop advancement, or verdict computation. |

## Hook Commands

```text
/bin/sh -c ... "${PLUGIN_ROOT}" hook pre-tool-use
  Blocks direct edits to protected Mobius ledgers.

/bin/sh -c ... "${PLUGIN_ROOT}" hook stop
  Blocks completion claims only when the final answer explicitly targets a Mobius goal that lacks
  an accepted verdict.csv.
```

`hooks/hooks.json` contains only minimal event dispatch plus the bootstrap checks that must run
before the launcher is available: empty `${PLUGIN_ROOT}` and a missing installed cache directory.
`scripts/mobius_hook_launcher.sh` is the only full hook launch path. It is invoked through
`/bin/sh`, so installed plugin bundles do not depend on executable file mode. Hook startup failures
are classified as follows:

- empty `${PLUGIN_ROOT}` prints `mobius:hook-misconfigured: PLUGIN_ROOT missing` and exits 2;
- a missing installed cache directory under `~/.codex/plugins/cache/.../mobius/<version>/` prints
  `mobius:hook-unavailable: installed plugin cache missing` and exits 0;
- an existing plugin root without `scripts/mobius.py` prints `mobius:hook-corrupt-install` and exits
  2; if the launcher itself is missing, the rendered hook command fails closed before Mobius code can
  run;
- missing `python3` prints `mobius:hook-runtime-missing: python3` and exits 2.

Mobius does not ship a post-tool hook. Contract validation after state mutation is owned by the
Mobius CLI and MobiusCV MCP command handlers, not by an ambient lifecycle hook.

No hook creates state at startup, scans historical runs for ordinary work, or infers a goal from
"the only active goal".

## Activation Rule

Mobius hook behavior is active only when the current hook payload or structured tool command
targets Mobius explicitly:

- `pre-tool-use` sees a protected path field or command token under `.mobius/runs/...`;
- `stop` sees a completion claim and the final text explicitly mentions the same `goal_slug` or
  `goal_id`.

Mobius hooks are no-op for ordinary work even if the plugin is enabled, `.mobius/` exists, a prior
session has a goal, or the final answer contains generic completion words. Reading protected ledger
files is allowed when the command is a simple read-only inspection. Restricted pipelines are narrow:
the first segment may be a read command such as `cat`, `head`, `tail`, `wc`, `sha256sum`, `stat`,
`file`, `grep`, `rg`, or `nl`; later segments may only be stdout filters `sort`, `uniq`, or `wc`
using option-only stdout forms, without path operands, output flags, or file-reading flags. Redirects,
shell control operators, and write-capable inspection tools remain blocked. Path globs and brace
expansions under `.mobius/runs/...` that could expand to protected ledger filenames are treated as
protected ledger paths.

## Invariants Enforced

- Direct writes to Mobius authoritative CSV ledgers are blocked outside sanctioned Mobius command
  paths. This includes `run.csv`, `goal.csv`, `plan.csv`, `acceptance.csv`, `evidence.csv`,
  `packets.csv`, `cv.csv`, `loop.csv`, `review_attempts.csv`, and `verdict.csv`.
- `stop` requires an accepted `verdict.csv` only for the same explicit `session_id + goal_slug` or
  exact `goal_id`.
- Hooks never call reviewers, mark stages passed, validate touched goals after ordinary tools, or
  compute final acceptance from agent prose.

`contract-add-stage`, `packet-create`, `contract-lock`, loop advancement, verdict computation, and
MobiusCV recorded review validate the goal contract directly before returning success. Stage
creation writes plan and acceptance rows transactionally; final lock and review boundaries require
the complete contract.

## Target Binding

Hook project-root resolution uses the hook payload first:

```text
project_root, projectRoot, workspace_root, workspaceRoot, cwd
```

If none are present, Mobius uses the process current working directory.

Path-derived protected-ledger checks preserve the real root before `/.mobius/`:

```text
/repo/.mobius/runs/codex-session-s/goal/plan.csv
```

binds as:

```text
root=/repo
goal_dir=/repo/.mobius/runs/codex-session-s/goal
session_id=s
goal_slug=goal
filename=plan.csv
```

Relative `.mobius/runs/...` paths resolve against the hook project root. Narrative content that
mentions `.mobius/` but is not a path field or command token does not activate write protection.

## Health And Doctor

Run install diagnostics from source or an installed cache:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py doctor
```

`doctor` reports plugin root, source versus installed-cache location, POSIX platform support,
`python3`, `uv`/`MOBIUS_CV_UV`, hook files, MCP launcher self-check readiness, and optional
`PLUGIN_DATA` writability. Hook status is one of `active`, `inactive`, `stale_cache_path`, or
`unsupported_platform`. Missing `uv` or a failing launcher self-check fails doctor because MobiusCV
MCP cannot start, but it does not make hook-health fail by itself.

Run hook health when checking only hook registration shape:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root /tmp hook-health
```

The command reports hook file presence, detected events, launcher dispatch readiness, source versus
installed-cache location, and a warning that hook trust is Codex external state. Mobius never mutates
Codex hook trust state; review changed hooks with `/hooks`.
