---
name: mobius-copilot
description: Manage one explicitly requested Mobius Objective contract and Map in the project-local Core. Use only when the user explicitly asks Mobius to activate a new Objective, revise or abandon the active Objective, or continue its pending initial or revised Map installation; ordinary coordination requests must not trigger this skill.
---

# Mobius Copilot

Guide the user through one exact human-authorized Objective contract action and any Map it requires.
Keep Core as the only business-state owner and keep the main agent as the only semantic owner of
every submitted transition. Leave execution of an active Objective to `$mobius-loop`.

## Enforce the invocation gate

Proceed only when the user explicitly requests Mobius, identifies the intended Objective, and asks
to activate, revise, abandon, or continue its pending initial or revised Map installation. If any
condition is absent, handle the request normally without initializing or changing Mobius. A prior
message is never evidence that continuation is valid; only the current typed Core state is.

Resolve the canonical project root and use only these Core MCP tools:

- `mobius_project_init`
- `mobius_read`
- `mobius_apply_transition`
- `mobius_audit`

Do not use CLI, direct files, SQL, or another ledger to create or change business state. Never
request, open, parse, or use a report, view, or CSV as Objective input. Human reports are derived
presentation only.

## Establish the live baseline

1. Initialize the project binding only for an activation when it does not exist and the user
   explicitly selected this project for the Mobius Objective.
2. Read typed status, current heads, and next actions from Core.
3. Match the requested action to live state: activation requires no active Objective; revision or
   abandonment requires the selected non-terminal Objective to be active; continuation requires
   the selected Objective to be in `Mapping` with `MappingReason` `Initial` or `SpecRevised` and
   `InstallMap` among its reported next actions.
4. Stop if another Objective is active. Do not replace, revise, or abandon it without the user's
   separate explicit intent.
5. Pin the Objective identity, project head, Objective head, and every material version used to
   shape the requested payload. Re-read before every submission.

Treat Core responses and current tool schemas as authoritative. Do not infer state from prose,
prior messages, delegated work, or presentation output.

## Shape the requested contract change

For activation or revision, translate the user's intended outcome into one typed Objective
specification. Make success observable by giving every Criterion a concrete statement,
verification rule, and scope. Preserve the user's boundaries and excluded claims; surface
ambiguity before submission instead of hiding it in a broad Criterion. A revision must preserve the
stable Objective identity and use a fresh ObjectiveSpec revision.

Shape one Map for that ObjectiveSpec with a single normal path:

- cover every current Criterion and assign it exactly one owner;
- give every Stage an outcome, output, and complete contract;
- keep dependencies acyclic and priorities total;
- include the required final-integration work;
- make each initial Route a falsifiable hypothesis for its Stage and current structural context.

Use the typed MCP schema for the actual objects. Do not restate or simulate reducer rules in this
skill; Core performs the final mechanical admission. For abandonment, shape only the exact reason
payload and do not create an ObjectiveSpec or Map.

## Obtain human confirmation

Before a new activation, revision, or abandonment transition:

1. Re-read live project and Objective heads.
2. Show the user the complete typed action and payload, including any Objective specification or
   reason that belongs to the action.
3. Ask for explicit confirmation of that exact action and payload.
4. Construct the typed confirmation bound to the current project, Objective, both heads, action,
   and complete payload only after the user confirms.

Never treat a prior general approval, a CLI flag, a worker result, or the main agent's own prose as
human confirmation. If the payload or either head changes, discard the confirmation and ask again.
After confirmation, execute exactly one branch below; the common confirmation step never submits a
transition itself. Continuing an already durable `Mapping` state does not repeat the accepted
contract transition or ask the user to reconfirm it.

## Execute exactly one contract branch

Use `MappingReason` as the Map-installation ownership boundary. Submit `InstallMap` only for
`Initial` after activation or `SpecRevised` after revision. `$mobius-loop` owns Map installation for
`Remap` and `WaitRevealedDrift`.

### Continue an accepted Mapping state

Use this as the single Map-installation path both immediately after an accepted contract transition
and after interruption. Re-read Core and require the selected live Objective to be in `Mapping`,
the reported next actions to permit `InstallMap`, and the reason to be exactly one of:

- `Initial`: shape and install the Map for the current initial ObjectiveSpec revision;
- `SpecRevised`: shape and install the replacement Map for the current revised ObjectiveSpec.

Treat the contract transition that produced this state as durable. From this continuation entry,
do not submit `ActivateObjective` or `ReviseObjective`, and do not ask the user to reconfirm either
accepted transition. Shape the Map only from the current typed ObjectiveSpec and heads read from
Core, submit its complete installation inputs, and re-read after acceptance. If the live reason is
`Remap` or `WaitRevealedDrift`, hand control to `$mobius-loop` instead.

### Activate a new Objective

Submit `ActivateObjective` only from the idle live state with the exact confirmed ObjectiveSpec.
Re-read Core, require the reported `MappingReason` to be `Initial`, then enter the accepted Mapping
continuation above. Re-read after each accepted transition.

### Revise the active Objective

Submit `ReviseObjective` only for the selected active Objective with the exact confirmed fresh
ObjectiveSpec. Never submit activation as a prelude to revision. Re-read the resulting `Mapping`
state, require the reported `MappingReason` to be `SpecRevised`, then enter the accepted Mapping
continuation above. Re-read again.

### Abandon the active Objective

Submit `Abandon` only for the selected active Objective with the exact confirmed reason. Never
submit activation or a Map as a prelude to abandonment. Re-read the terminal `Abandoned` state and
stop.

The Copilot is the sole Composition owner of these three human-authorized contract actions. On a
stale head, retain no assumed success and read the new baseline. Obtain a new confirmation before
retrying a changed or unaccepted human-authorized contract transition. For a stale `InstallMap`
continuation, rebuild against the live Mapping state and heads without reconfirming the already
accepted contract transition. Serialize every submission.

Finish by reading status, current context, and next actions. Report the Objective identity,
authorized outcome, current typed state, and unresolved user decisions. Do not claim that the
Objective itself is complete merely because a contract action succeeded, and do not emit the
Mobius completion marker.
