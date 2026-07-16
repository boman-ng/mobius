---
name: mobius-loop
description: Continue one explicitly selected Mobius Objective through typed Core state, optional native delegated work, evidence admission, review, and verified completion. Use only when the user explicitly asks to run or continue a named Mobius Objective; ordinary implementation or review work must not trigger this skill.
---

# Mobius Loop

Run one Objective from its current Core state. Let Core own facts and guards; let the main agent
own work selection, candidate interpretation, Evidence translation, formal Judgment, and every
mutation submission.

## Enforce the invocation gate

Proceed only when the user explicitly requests Mobius and identifies the Objective to run. Read
the project binding, typed status, current heads, context, review material, and next actions before
doing work. Stop on a missing, different, terminal, or ambiguously selected Objective rather than
guessing the target.

Use only these Core MCP tools:

- `mobius_read`
- `mobius_capture_artifact`
- `mobius_read_artifact`
- `mobius_apply_transition`
- `mobius_audit`

Only `mobius_apply_transition` submits business transitions. Do not use CLI, direct files, SQL, or
another state path. Never request, open, parse, or use a report, view, or CSV as work, Evidence,
Judgment, proof, or completion input.

## Run the state-driven loop

Repeat this sequence until Core reports a terminal state or the work is honestly blocked:

1. Read live heads, current context, relevant objects or review material, and typed next actions.
2. Choose work from the current model need; do not choose a transition because of a worker role.
3. Perform the work directly or use the delegated lane below.
4. Inspect actual observations, effects, artifacts, counterevidence, unknowns, and cleanup.
5. Translate only verified candidates into complete typed model input.
6. Re-read the live baseline, then submit at most one transition.
7. Read the accepted state before selecting the next action.

Keep Core submissions strictly serial. A stale head, changed subject, changed Acceptance Context,
or changed frozen material invalidates the old candidate for admission. Preserve it only as a lead;
still inspect and clean up any real-world effects that already occurred.

## Install execution remaps

When Core reports `Mapping`, read its typed `MappingReason` before selecting the next action.

- For `Remap` or `WaitRevealedDrift`, own the replacement Map installation in this loop. Shape the
  complete Map, initial Routes, cover judgment, and carry judgment from the current typed context;
  submit `InstallMap` only when the reported next action permits it, then re-read Core.
- For `Initial` or `SpecRevised`, do not submit `InstallMap`. Hand control to `$mobius-copilot`,
  because that installation belongs to its confirmed activation or revision branch.

Never infer the reason from prose or a prior state. On a missing or unknown reason, stop without
submitting a transition.

## Use the delegated lane

Use `$mobius-subagent` and the current host's native Subagent workflow when delegation is useful.
Do not create another runtime, queue, registry, role protocol, or persistent task state.
Do not require a separate user invocation merely to select this optional delegation path. Preserve
the user's authorization, approval, and effect boundaries for every delegated task.

Choose only the bounded tasks justified by the current missing observation, effect, verification,
or counterargument. Use a Driver only when an authorized external effect is needed; use Scout,
Researcher, or Verifier tasks for distinct investigation needs; use zero or more independent Judges
when an advisory challenge would materially improve the main agent's decision. Do not impose a
fixed role sequence or worker count.

Run independent read-only tasks concurrently when the host supports it. Keep overlapping effects
serial, start Verifier work only after the relevant effect is stable, and freeze exact materials
before each Judge task. Use the existing role envelopes unchanged and keep every Judge advisory.

Add both of these explicit forbidden rules to every Mobius delegation envelope:

- Do not call any Mobius Core MCP method, including any of the six `mobius_*` tools.
- Do not read or write `.mobius/` managed state.

Check both rules before spawning. Reject and rebuild an envelope when either rule is absent; never
send a partially bounded Mobius delegation.

The rules apply even when the Runtime exposes the same tools and filesystem permissions to a child
thread. Do not pass a Core handle, mutation instruction, database content, report, view, or CSV to
a worker. A boundary violation, spawn/configuration/runtime/permission failure, partial effect,
unauthorized effect, or pending cleanup cannot advance Core state.

The direct lane follows the same candidate-consumption and Core-admission path without spawning
workers. Delegation is optional work production, not a second state-machine path.

## Consume candidates

For every native result, check the pinned baseline, all objective and done-condition results,
boundary compliance, material versions, actual effect scope, provenance, artifacts, uncertainties,
and cleanup. Verify the affected world rather than accepting a summary or role disposition.

For Evidence, identify the current subject and Acceptance Context, freeze the observation inline or
with `mobius_capture_artifact`, define the claims domain and provenance, and construct the complete
typed Evidence as the main agent. A worker result, effect record, artifact locator, or successful
Runtime status is never Evidence by itself.

For formal Judgment, read the current Core review material and required dependency view, inspect
the complete Packet, Evidence, counterevidence, and unknowns, and independently construct the typed
Decision or wait Judgment. Judge advice, votes, model count, and recommendations never become the
formal Judgment automatically. Packet materialization for sealing remains Core-owned.

## Hand off Objective contract changes

`$mobius-copilot` is the sole Composition owner of Objective activation, revision, abandonment, and
the `Initial` or `SpecRevised` Map installation those actions require. This loop does not submit
those contract transitions. It requires an already active Objective, so activation is outside its
entry contract. If the user requests revision or abandonment, or the work exposes a need for
either, re-read the live baseline, report the exact contract decision required, and hand control to
`$mobius-copilot`. Do not turn the handoff itself into a mutation or inferred human confirmation.

## Gate the completion claim

Treat Objective setup, successful delegated work, any number of favorable advisory reviews, an
accepted review, and an empty local task list as non-terminal signals. Completion exists only when
a fresh `mobius_read` for the selected Objective reports `Achieved`.

After that read succeeds, include exactly this standalone line in the final response:

```text
MOBIUS_OBJECTIVE_ACHIEVED: <objective-id>
```

Use the exact selected Objective identity. Do not emit the marker for waiting, abandoned, blocked,
degraded, unreadable, stale, or otherwise non-Achieved state. Report those states truthfully and
leave the marker absent.
