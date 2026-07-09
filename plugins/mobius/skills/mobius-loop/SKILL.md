---
name: mobius-loop
description: "Run one locked Mobius Objective through Route Runs, Evidence, Review Targets, and recorded Review Judgments."
---

# Mobius Loop

Use this skill only for a locked, explicitly targeted Mobius Objective. Do not use it for ordinary
Codex tasks, ad hoc implementation loops, or non-Mobius review gates. By default, run the full
locked Objective until Mobius reaches a terminal gate or a real stop condition.

Hooks may block direct CSV edits or explicit false terminal claims. Hooks never advance loop state,
run reviewers, or accept an Objective.

## Full Loop

1. Resolve the Mobius plugin root from this skill path, then run all Mobius CLI commands through
   that absolute script path.
2. Read status and ask for the next required action:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> explain \
  --session-id <codex-session-id> \
  --objective-slug <objective-slug>

python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> continue \
  --session-id <codex-session-id> \
  --objective-slug <objective-slug>
```

3. Treat `loop` as the controller:
   - Continue automatically while `loop.agent_must_continue=true`.
   - Stop only when `loop.agent_must_stop=true`, `ok=false`, a required tool/reviewer is
     unavailable, the user interrupts, or the Verdict is `accepted` or `blocked`.
   - Prefer `loop.next_argv` and `loop.next_actions` over reconstructing commands from prose.

4. When `next_required_action=start_route_run`, start the returned Work Item route:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> route-run-start \
  --session-id <codex-session-id> \
  --objective-slug <objective-slug> \
  --work-item-id <W1>
```

5. Implement only the selected Work Item scope.
6. Record objective Evidence for the linked Criteria:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> evidence-add \
  --session-id <codex-session-id> \
  --objective-slug <objective-slug> \
  --type command_result \
  --summary "<what the evidence proves>" \
  --supports <C1> \
  --artifact-json '{"type":"command_result","name":"<command name>","command":"<command>","exit_code":0}'
```

7. When Evidence is present, create the Review Target:

```bash
python3 <mobius-plugin-root>/scripts/mobius.py --project-root <project-root> review-target-create \
  --session-id <codex-session-id> \
  --objective-slug <objective-slug> \
  --review-mode checkpoint_review \
  --work-item-id <W1>
```

8. Use Mobius Review MCP:
   - `mobius_review_build_subagent_prompt` builds the host reviewer prompt from the Review Target.
   - Run the host Codex subagent with that prompt.
   - Pass its raw stateless result to `mobius_review_record_checkpoint_judgment` or
     `mobius_review_record_exit_judgment`.
   - Close the completed host subagent after the review is recorded or after a visible failure.

9. After any recorded Review Judgment with `ok=true` and `persisted=true`, immediately call
   `continue` and execute the returned loop action.

10. When `continue` reports `create_exit_review_target`, create an exit Review Target and record an
    exit Review Judgment. Completion is allowed only when the recorded exit result leads to
    `gate=accepted` or the loop returns terminal `accepted`.

## Rules

- Use Mobius only for explicitly targeted Objective work.
- Implement exactly the returned Work Item scope and leave unrelated paths alone.
- Review Feedback is loop work, not Route failure budget.
- Do not mark Criteria passed from agent confidence or self-review.
- Do not reuse a Review Target for a second Review Judgment.
- Do not create Review Targets, record Review Judgments, add Evidence, or start Route Runs after a
  terminal Verdict.
- During repair, prune obsolete wrong-path artifacts instead of preserving compatibility aliases,
  broad fallbacks, or glue layers.
- If the contract itself must change, return to `mobius-plan` and create a new Objective contract.
