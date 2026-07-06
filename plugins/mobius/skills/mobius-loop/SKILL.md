---
name: mobius-loop
description: "Run one locked Mobius goal through programic stage gates and recorded external review. Use only for an explicitly targeted Mobius goal."
---

# Mobius Loop

Use this skill only for a locked, explicitly targeted Mobius goal. By default, run the full locked
plan as a loop until Mobius reaches a terminal gate or a real stop condition. Each iteration
still executes exactly one stage contract, but a passed stage is not a stopping point; immediately
call `continue` again and consume the next `loop.next_required_action`.

Use one-stage mode only when the user explicitly asks for exactly one stage, a dry run, a status
check, or a pause after the current gate. The normal `$mobius-loop <goal>` experience is full-plan
Loop Engineering with minimal human-in-loop.

Hooks may block direct CSV edits or explicit false completion claims. Hooks
never advance loop state, run reviewers, or accept a goal.

## Skill/MCP/CLI/Hook Responsibility Boundary

| Surface | Owns | Must Not Own |
| --- | --- | --- |
| `mobius-plan` skill | User-facing workflow for creating, validating, and locking a complete contract. | Hidden policy decisions, CSV writes outside CLI, or alternate contract semantics. |
| `mobius-loop` skill | User-facing full-plan loop for repeatedly executing locked stages, recording evidence, creating packets, and calling recorded review until a real stop gate. | Manual interpretation of reviewer prose, manual loop advancement, or final acceptance. |
| MobiusCV MCP | Prompt building, host reviewer result normalization, Kimi adapter execution/parsing, reviewer result assembly, and calling CLI persistence APIs. | Direct CSV writes, terminal verdict computation, bypassing CLI validation, or relaxing packet/contract errors. |
| `mobius.py` CLI/ledger engine | CSV state transitions, contract validation, packet validation, one-shot packet enforcement, loop state, acceptance status, and verdict derivation. | Kimi process lifecycle or reviewer prompt/tool execution. |
| Hooks | Guardrails for direct protected-ledger writes and explicit false completion claims. | State mutation, reviewer execution, goal inference from ambient `.mobius` state, loop advancement, or verdict computation. |

## Full Plan Loop

1. Resolve the Mobius plugin root from this skill path, then run all Mobius CLI commands through
   that absolute script path.
2. Read status, audit the ledger, and ask for the next required action:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> loop-status \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug>

python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> ledger-audit \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug>

python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> continue \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug>
```

3. Treat `loop` as the loop controller:
   - Continue automatically while `loop.agent_must_continue=true`.
   - Stop only when `loop.agent_must_stop=true`, `ok=false`, a tool/reviewer is unavailable,
     the user interrupts, or the goal reaches `accepted` or `blocked`.
   - Do not send a final answer just because a delta review passed. A delta pass means call
     `continue` again in the same turn.
4. When `loop.next_required_action=start_next_stage`, start exactly that stage and read the
   returned `stage_contract` JSON:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> loop-start-stage \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug> \
  --plan-item-id <P1>
```

5. Implement only the selected stage.
6. Record objective evidence for the linked acceptance ids. Supported evidence types are
   `change_set_scope`, `file_ref`, `command_result`, `test_result`, and `human_assertion`. When
   the acceptance proof requires a concrete command, test, file, or change-set scope, include
   structured `artifact_json` metadata:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> evidence-add \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug> \
  --type command_result \
  --summary "<what the evidence proves>" \
  --supports <A1> \
  --artifact-json '{"type":"command_result","name":"<command name>","command":"<command>","exit_code":0}'
```

Use `--artifact <path>` only with `--type file_ref`. Use `change_set_scope` evidence for absence
and scope claims:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> evidence-add \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug> \
  --type change_set_scope \
  --summary "<what scope was checked>" \
  --supports <A1> \
  --artifact-json '{"type":"change_set_scope","paths":["src/**"],"allowed_change_classes":["source"],"forbidden_paths":[".mobius/**"],"coverage":{"tracked":true,"staged":true,"untracked":true,"intent_to_add":true}}'
```

Do not copy command output, diff output, or file contents into the packet.

MobiusCV review is a verifier, not evidence. If a stage needs review to confirm an absence claim,
such as "no business behavior was added", first record `change_set_scope` evidence, then use the
delta review gate. Packet refs are starting points for reviewer judgment, not exclusive evidence.

7. Create a JSON packet for the stage:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> packet-create \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug> \
  --review-mode delta_review \
  --acceptance-id <A1>
```

Each packet is one-shot review input. It must be a compact `mobius.packet` envelope with
`coverage` and `refs`, not a copied evidence archive. If a stage is revised or a review must be
rerun, create a new packet from the current Mobius ledgers. Do not reuse a previous `packet_id`.

8. Use MobiusCV:
   - `mobius_cv_build_subagent_prompt` to build the host subagent prompt from the packet JSON.
   - Run the host Codex subagent with that prompt and pass its raw stateless result back.
   - `mobius_cv_record_delta_review` to persist the stage review. Delta review defaults to
     `delta_light` with the host subagent only; use level 2 or `input_refs.review_policy` named
     `delta_kimi` when the stage needs the external Kimi reviewer.

9. After any recorded review result with `ok=true` and `persisted=true`, immediately call
   `continue` and execute the returned loop action. A repairable delta failure is normal loop
   work when the loop returns `repair_stage`, `record_missing_evidence`,
   `run_missing_command_evidence`, `create_new_packet`, or `retry_review`. Do not ask the user for
   permission between stages or repair attempts unless the user explicitly requested one-stage mode
   or the loop reports `agent_must_stop=true`.
10. When `continue` reports `create_exit_packet`, create an `exit_review` packet and call
   `mobius_cv_record_exit_review`. If the exit review records a repairable fail, immediately call
   `continue`; the loop will route the earliest affected stage back through normal repair work.
11. When `continue` reports `record_exit_review` or exit-review `retry_review`, retrieve the
   outstanding packet through `packet-read`, then call `mobius_cv_record_exit_review` with that
   packet:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> packet-read \
  --session-id <codex-session-id> \
  --goal-slug <yyyy-mm-dd-goal-slug> \
  --review-mode exit_review \
  --packet-id <packet_exit_001>
```

12. Completion is allowed only when the recorded exit result returns `gate=accepted`.
13. When a goal is `accepted` or `blocked`, stop the loop for that goal. Start a new
    goal for later changes unless an explicit reopen design exists.

## Rules

- Use Mobius only for an explicitly targeted Mobius goal.
- Follow command JSON fields, especially `loop`, `stage_contract`, `gate`,
  `next_required_action`, `packet`, and `updated_files`.
- Default to full-plan loop execution. Do not stop after a passed delta gate unless the user
  explicitly requested one-stage mode.
- Implement exactly the returned stage scope, work, gate, recovery, budget, and acceptance proof
  obligations. Do not infer stage boundaries from prose.
- Pass the packet JSON object to MobiusCV. Do not pass CSV packet ledger rows.
- Branch only on command/review fields: `loop.agent_must_continue`,
  `loop.agent_must_stop`, `ok`, `persisted`, `gate`, `next_required_action`,
  `blocking_findings`, and `errors`.
- Do not interpret Kimi output or reviewer prose manually.
- Do not require Kimi for every delta review; Kimi remains mandatory for `delta_kimi` and
  `exit_strict`.
- Do not edit Mobius CSV files by hand during the normal loop.
- Do not mark a stage passed from Agent confidence or self-review.
- Do not use a delta review as final acceptance.
- Do not pass prior review chat as scope for a new exit review.
- Do not reuse a packet or `packet_id` for a second recorded review.
- Do not create packets, record reviews, add evidence, or start stages after a terminal verdict.
- Treat `ok=false`, `persisted=false`, terminal verdicts, explicit loop stops, and unavailable
  reviewer/tool classes as stops. Treat missing evidence and concrete reviewer revisions as loop
  work unless the loop reports a stop.
- If the contract must change, return to `mobius-plan`.
