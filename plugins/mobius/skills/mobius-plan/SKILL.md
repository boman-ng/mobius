---
name: mobius-plan
description: "Create, validate, and lock an explicit Mobius goal contract. Use only for an explicitly targeted Mobius goal."
---

# Mobius Plan

Use this skill only when the user explicitly asks for Mobius or provides an existing Mobius goal
target. The planning path is programic: create the goal, add executable stage contracts, validate,
then lock. Do not hand-edit CSV files in the normal workflow.

Planning creates the explicit `session_id + goal_slug` state that Mobius hooks can protect. Hooks
stay no-op for ordinary work and do not replace this planning path.

## Skill/MCP/CLI/Hook Responsibility Boundary

| Surface | Owns | Must Not Own |
| --- | --- | --- |
| `mobius-plan` skill | User-facing workflow for creating, validating, and locking a complete contract. | Hidden policy decisions, CSV writes outside CLI, or alternate contract semantics. |
| `mobius-loop` skill | User-facing workflow for executing one locked stage, recording evidence, creating packets, and calling recorded review. | Manual interpretation of reviewer prose, manual loop advancement, or final acceptance. |
| MobiusCV MCP | Prompt building, host reviewer result normalization, Kimi adapter execution/parsing, reviewer result assembly, and calling CLI persistence APIs. | Direct CSV writes, terminal verdict computation, bypassing CLI validation, or relaxing packet/contract errors. |
| `mobius.py` CLI/ledger engine | CSV state transitions, contract validation, packet validation, one-shot packet enforcement, loop state, acceptance status, and verdict derivation. | Kimi process lifecycle or reviewer prompt/tool execution. |
| Hooks | Guardrails for direct protected-ledger writes and explicit false completion claims. | State mutation, reviewer execution, goal inference from ambient `.mobius` state, loop advancement, or verdict computation. |

## Workflow

1. Identify the project root, Codex session id, and goal slug.
2. Resolve the Mobius plugin root from this skill path, then run all Mobius CLI commands through
   that absolute script path.
3. Create the goal:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> goal-start \
  --session-id <codex-session-id> \
  --slug <short-goal-slug> \
  --title "<goal title>" \
  --user-goal "<user-visible objective>"
```

4. Before adding a stage, shape each acceptance row with this proof matrix:

```text
Acceptance ID:
Requirement:
Observable outcome:
Evidence required: change_set_scope|file_ref|command_result|test_result|human_assertion
Verifier: change_set_scope|file_ref|command_result|test_result|human_assertion|mobiuscv_delta|mobiuscv_exit
Evidence-add command that can satisfy the required evidence:
Reviewer focus:
```

MobiusCV is a verifier, not objective evidence. Do not put `mobiuscv_delta` or
`mobiuscv_exit` in `evidence_required`. For review-only claims such as "no business
behavior was added", require a `change_set_scope` evidence row with coverage for tracked, staged,
untracked, and intent-to-add changes, then use `mobiuscv_delta` as the verifier. Do not copy diff
output into the packet.

5. Add each executable stage and its linked proof obligations in one command:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> contract-add-stage \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug> \
  --id P1 \
  --title "<stage title>" \
  --description "<verifiable implementation scope>" \
  --depends-on-json '[]' \
  --scope-json '{"allowed_paths":["src/**","tests/**"],"forbidden_paths":[".mobius/**"],"non_goals":["Do not change unrelated behavior"],"invariants":["tests pass"],"side_effect_level":"local"}' \
  --work-json '{"target_refs":["src/**"],"deliverables":["implemented behavior"],"deleted_paths":[]}' \
  --gate-json '{"entry":["contract locked"],"exit":["tests pass"],"verifiers":["command_result","mobiuscv_delta"],"review_focus":["scope and proof"]}' \
  --recovery-json '{"rollback_boundary":"revert stage files","restart_rule":"restart selected stage","escalation_rule":"surface blocker"}' \
  --budget-json '{"retry_limit":2,"max_stage_attempts":3,"stop_condition":"recorded review blocks or passes"}' \
  --acceptance-json '[{"id":"A1","requirement":"Tests pass","observable_outcome":"test command exits 0","evidence_required":[{"type":"command_result","name":"test command","exit_code":0}],"verifier":[{"type":"command_result","name":"test command"},{"type":"mobiuscv_delta"}],"review_focus":["proof obligation is satisfied"],"required":true}]'
```

For small local stages, `--contract-defaults local` may fill omitted
`depends-on-json`, `scope-json`, `gate-json`, `recovery-json`, and `budget-json` cells before
storage. Prefer explicit JSON when the stage has nontrivial boundaries, risks, rollback needs, or
review policy.

6. Validate and lock the contract:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> contract-validate \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug>

python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> contract-lock \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug> \
  --locked-by main_agent
```

7. Continue with `mobius-loop`.

## Rules

- Use Mobius only for an explicitly targeted Mobius goal.
- Treat command JSON as the source for `goal_slug`, ids, gates, errors, and next actions.
- Every required non-root stage needs dependencies; every stage needs scope, work, gate, recovery,
  budget, and linked proof obligations.
- `evidence_required` may only use evidence-add types: `change_set_scope`, `file_ref`,
  `command_result`, `test_result`, or `human_assertion`.
- Path refs are only valid for `file_ref` evidence. Change-set scope evidence must declare
  coverage for tracked, staged, untracked, and intent-to-add changes.
- `mobiuscv_delta` and `mobiuscv_exit` belong in `verifier`, never in `evidence_required`.
- Do not change locked structural fields in place.
- If the contract must change, use `contract-supersede-stage` with a new plan id, new acceptance
  ids, `--supersedes-id`, and `--change-reason`, then validate and lock again.
- A locked contract is required before implementation review.
