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

## Agentic Contract Shape

Shape the contract from first principles before adding stages. A Mobius plan should make the
following explicit in existing stage and acceptance fields:

- Goal: the user-visible outcome the locked plan must produce.
- Constraints: path, policy, dependency, data, and interaction limits.
- Inputs and outputs: what the stage may consume and what it must leave behind.
- Invariants: behavior, security, state, and public-contract facts that must remain true.
- Risks, assumptions, and known unknowns: what could make the plan wrong or incomplete.
- Minimum sufficient evidence: the smallest objective proof that can satisfy each acceptance row.
- Disconfirming observation: what evidence would prove the requirement is not met.
- Blind spot to inspect: the likely hidden failure mode, such as happy-path-only proof, stale refs,
  unchecked absence claims, or proxy metrics that do not represent the user outcome.
- Feedback signal: command, test, file, change-set, human assertion, or MobiusCV review signal that
  changes the next loop action.
- Stop condition: the Programic terminal or intervention gate, not Agent confidence.

Express these checks through the current contract shape: `scope_json.non_goals`,
`scope_json.invariants`, `gate_json.review_focus`, `recovery_json`, `budget_json.stop_condition`,
and acceptance `review_focus`. Do not create a second planning format, cognition ledger, or
parallel proof model.

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
Objective:
Constraints:
Inputs:
Outputs:
Invariants:
Risks:
Observable outcome:
Evidence required: change_set_scope|file_ref|command_result|test_result|human_assertion
Verifier: change_set_scope|file_ref|command_result|test_result|human_assertion|mobiuscv_delta|mobiuscv_exit
Evidence-add command that can satisfy the required evidence:
Assumptions:
Known unknowns:
Minimum sufficient evidence:
Disconfirming observation:
Blind spot to inspect:
Feedback signal:
Stop condition:
Reviewer focus:
```

MobiusCV is a verifier, not objective evidence. Do not put `mobiuscv_delta` or
`mobiuscv_exit` in `evidence_required`. For review-only claims such as "no business
behavior was added", require a `change_set_scope` evidence row with coverage for tracked, staged,
untracked, and intent-to-add changes, then use `mobiuscv_delta` as the verifier. Do not copy diff
output into the packet.

When helpful, make `review_focus` entries structured objects instead of generic strings:

```json
{
  "question": "What evidence would falsify this acceptance?",
  "blind_spot": "happy-path-only evidence",
  "counterevidence": "negative or boundary case fails",
  "expected_signal": "test_result or command_result covers the boundary"
}
```

5. Add each executable stage and its linked proof obligations in one command:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> contract-add-stage \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug> \
  --id P1 \
  --title "<stage title>" \
  --description "<verifiable implementation scope>" \
  --depends-on-json '[]' \
  --scope-json '{"allowed_paths":["src/**","tests/**"],"forbidden_paths":[".mobius/**"],"non_goals":["Do not change unrelated behavior"],"invariants":["tests pass","public contract is preserved"],"side_effect_level":"local"}' \
  --work-json '{"target_refs":["src/**"],"deliverables":["implemented behavior"],"deleted_paths":[]}' \
  --gate-json '{"entry":["contract locked"],"exit":["tests pass"],"verifiers":["command_result","mobiuscv_delta"],"review_focus":[{"question":"Does evidence satisfy the user outcome rather than a process proxy?","blind_spot":"process metric passes while behavior regresses","counterevidence":"user-facing behavior or boundary test fails","expected_signal":"command_result or test_result covers the boundary"}]}' \
  --recovery-json '{"rollback_boundary":"revert stage files","restart_rule":"restart selected stage","escalation_rule":"surface blocker"}' \
  --budget-json '{"retry_limit":2,"max_stage_attempts":3,"stop_condition":"loop reports accepted, terminal blocked, agent_must_stop, unavailable required reviewer/tool, or explicit user interruption"}' \
  --acceptance-json '[{"id":"A1","requirement":"Tests pass","observable_outcome":"test command exits 0","evidence_required":[{"type":"command_result","name":"test command","exit_code":0}],"verifier":[{"type":"command_result","name":"test command"},{"type":"mobiuscv_delta"}],"review_focus":[{"question":"What would falsify this passing claim?","blind_spot":"only the happy path was tested","counterevidence":"test command exits nonzero or omits the changed boundary","expected_signal":"command_result exit_code 0 and relevant test coverage"}],"required":true}]'
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
Use `status` for goal discovery and `explain --session-id ... --goal-slug ...` for a concise
diagnostic summary instead of reading CSV ledgers directly during routine planning or handoff.

## Rules

- Use Mobius only for an explicitly targeted Mobius goal.
- Treat command JSON as the source for `goal_slug`, ids, gates, errors, and next actions.
- Every required non-root stage needs dependencies; every stage needs scope, work, gate, recovery,
  budget, and linked proof obligations.
- `evidence_required` may only use evidence-add types: `change_set_scope`, `file_ref`,
  `command_result`, `test_result`, or `human_assertion`.
- Acceptance planning must name assumptions, known unknowns, blind spots, disconfirming
  observations, minimum sufficient evidence, feedback signals, and stop conditions before lock.
- Path refs are only valid for `file_ref` evidence. Change-set scope evidence must declare
  coverage for tracked, staged, untracked, and intent-to-add changes.
- `mobiuscv_delta` and `mobiuscv_exit` belong in `verifier`, never in `evidence_required`.
- Keep Agentic checks in existing JSON fields and `review_focus`; do not add a cognition ledger,
  reviewer result field, or alternate contract format.
- Do not change locked structural fields in place.
- If the contract must change, use `contract-supersede-stage` with a new plan id, new acceptance
  ids, `--supersedes-id`, and `--change-reason`, then validate and lock again.
- A locked contract is required before implementation review.
