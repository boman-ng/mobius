---
name: mobius-plan
description: "Create, validate, and lock an explicit Mobius Objective contract. Use only for an explicitly targeted Mobius Objective."
---

# Mobius Plan

Use this skill only when the user explicitly asks for Mobius or provides an existing Mobius
Objective target. Do not use it for ordinary planning, todo lists, implementation plans, or review
checklists that are not explicitly Mobius Objective work. Do not hand-edit CSV ledgers in the
normal workflow.

Planning creates the explicit `session_id + objective_slug` state that Mobius hooks can protect.
Hooks stay no-op for ordinary work and do not replace this planning path.

## Contract Shape

Use one canonical model:

- Objective: user-visible outcome.
- Work Item: ordered unit of work.
- Criterion: observable condition required by a Work Item.
- Route: implementation path.
- Route Run: one execution of a Route.
- Timebox: harness-internal time available to a Route Run.
- Evidence: objective support for Criteria.
- Review Target: frozen one-shot review input.
- Review Judgment: recorded reviewer decision and feedback action.
- Verdict: derived terminal adjudication.

Review Feedback is advisory loop input. It is not failure budget. Route Run failure is a timebox
expiry, no-viable-route classification, required tool/reviewer unavailability, or explicit user
stop.

## Workflow

1. Resolve the plugin root from this skill path.
2. Create the Objective:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> objective-start \
  --session-id <codex-session-id> \
  --slug <objective-slug> \
  --title "<objective title>" \
  --user-request "<user-visible objective>"
```

3. Add each Work Item with linked Criteria:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> contract-add-work-item \
  --session-id <codex-session-id> \
  --objective-slug <objective-slug> \
  --id W1 \
  --title "<work item title>" \
  --description "<verifiable scope>" \
  --depends-on-json '[]' \
  --scope-json '{"allowed_paths":["src/**","tests/**"],"forbidden_paths":[".mobius/**"],"non_claims":["unrelated behavior"],"invariants":["tests pass"],"side_effect_level":"local"}' \
  --work-json '{"target_refs":["src/**"],"deliverables":["implemented behavior"],"deleted_paths":[]}' \
  --gate-json '{"entry":["contract locked"],"exit":["criterion evidence exists"],"verifiers":["command_result","checkpoint_review"],"review_focus":[{"question":"Does evidence prove user outcome rather than process motion?","blind_spot":"proxy evidence","counterevidence":"behavior still fails","expected_signal":"command_result or test_result covers the boundary"}]}' \
  --recovery-json '{"rollback_boundary":"selected files","restart_rule":"start a new Route Run within the Timebox","escalation_rule":"surface only impossible contract, no viable Route, unavailable tool/reviewer, or user stop"}' \
  --timebox-json '{"route_run_timebox_ms":14400000,"budget_axis":"harness_internal_time"}' \
  --criteria-json '[{"id":"C1","requirement":"Tests pass","observable_outcome":"test command exits 0","evidence_required":[{"type":"test_result","name":"test command"}],"verifier":[{"type":"test_result","name":"test command"},{"type":"checkpoint_review"},{"type":"exit_review"}],"review_focus":[{"question":"What would falsify this Criterion?","blind_spot":"happy-path-only proof","counterevidence":"boundary test fails","expected_signal":"test_result exit_code 0 and relevant coverage"}],"required":true}]'
```

4. Validate and lock:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> contract-validate \
  --session-id <codex-session-id> \
  --objective-slug <objective-slug>

python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> contract-lock \
  --session-id <codex-session-id> \
  --objective-slug <objective-slug> \
  --locked-by main_agent
```

5. Continue with `mobius-loop`.

## Rules

- Use Mobius only for explicitly targeted Objective work.
- Treat command JSON as the source for ids, gates, errors, and next actions.
- Every required non-root Work Item needs dependencies.
- Every Work Item needs scope, work, gate, recovery, Timebox, and linked Criteria.
- `evidence_required` may only use `change_set_scope`, `file_ref`, `command_result`,
  `test_result`, or `human_assertion`.
- Review verifiers are `checkpoint_review` and `exit_review`.
- Use `timebox_json.route_run_timebox_ms`; do not use retry-count budgeting.
- Do not change locked structural fields in place. Create a new Objective contract when the
  contract itself is wrong.
