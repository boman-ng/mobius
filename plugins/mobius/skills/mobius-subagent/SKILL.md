---
name: mobius-subagent
description: Coordinate bounded delegated work through the current host's native Subagent workflow. Use when a user requests delegation or the main agent determines that delegation is useful and needs a scout, researcher, driver, verifier, or advisory judge with self-contained task, result, effect, and freshness semantics.
---

# Mobius Subagent

Use this skill when delegation is requested or the main agent determines that delegation is useful. Keep the workflow instruction-first: the host Runtime owns execution, while the main agent owns task construction, interpretation, verification, and every later submission.

## Preserve the ownership boundary

- Treat one delegation as one bounded task with one final result.
- Treat native agent/thread, turn, item, tool, permission, model, status, and usage objects as the only execution facts.
- Do not create a worker ledger, queue, scheduler, registry, heartbeat, memory, transport, or Runtime mirror.
- Keep workers free of cross-task business continuity. Create a new task when the role, objective, authorization, baseline, or frozen material changes.
- Treat every returned observation, effect, artifact, recommendation, and provenance item as a candidate for main-agent inspection, never as an automatically accepted downstream fact.
- Check baseline and material freshness before consuming a result.
- Use a cooperative instruction boundary. Do not require caller attestation, role-specific tool hiding, or a custom sandbox.

## Select one role

| Role | Work mode | Default effect boundary | Recommended execution policy |
|---|---|---|---|
| `scout` | Inspect local files, code, logs, tests, and data | Read-only | A current host-available fast coding model / `medium` |
| `researcher` | Investigate authoritative open-world sources | Read-only workspace; authorized network reads | A current host-available research-capable model / `medium` |
| `driver` | Continue the main agent's work on one bounded change | Only effects explicitly authorized by the task | Let the host Runtime resolve model and effort |
| `verifier` | Independently reproduce, test, observe, and compare | Read-only by default; declared temporary test effects | A current host-available reliable coding model / `high` |
| `judge` | Challenge frozen materials and return advisory findings | Read-only | Internal: a current host-available strong reasoning model / `medium`; external: an available independent model |

Treat this capability-based matrix as advisory execution policy, not an availability guarantee or a fixed model catalog. Resolve it against the host's current supported models. Confirm an explicitly requested model and effort before use and select the closest currently available configuration when necessary. Never pin a Driver-specific model or effort.

Select a role by its work function, never by inferring identity from the model that actually runs it.

Read [role profiles](references/role-profiles.md) after selecting a role. Load the selected role's complete input/output template and rules; do not ask the worker to find or read this reference.

## Build the task envelope

Provide all correctness-critical context inside the task. Use natural language, structured Markdown, or JSON; the following object defines required semantics rather than a new transport protocol:

```json
{
  "role": "scout | researcher | driver | verifier | judge",
  "background": {
    "why_now": "reason for delegating now",
    "current_state": ["relevant current conditions and completed work"],
    "confirmed_facts": [
      {"id": "BF1", "fact": "a fact already checked by the main agent", "evidence": []}
    ],
    "materials": [
      {"id": "BM1", "locator": "accessible material", "purpose": "why it is supplied"}
    ],
    "assumptions_to_check": [
      {"id": "BA1", "assumption": "a premise that still needs evaluation"}
    ]
  },
  "objectives": [
    {"id": "O1", "objective": "one observable result", "priority": "must | should"}
  ],
  "boundaries": {
    "forbidden": [
      {"id": "F1", "rule": "prohibited action or target", "reason": "why it is prohibited"}
    ],
    "focus": [
      {"id": "FO1", "target": "primary entry point", "purpose": "why to start here"}
    ]
  },
  "role_input": {},
  "output_format": {
    "representation": "structured_markdown | json",
    "template": "the complete common result plus the selected role output",
    "constraints": ["evidence granularity, length, locator, and redaction requirements"],
    "result_budget": {
      "max_public_result_bytes": 8192
    }
  },
  "done_when": [
    {"id": "D1", "condition": "an observable return condition", "evidence_required": []}
  ]
}
```

Apply these construction rules:

- Keep `objectives` and `done_when` non-empty and within one coherent task.
- Include `boundaries.forbidden` even when its list is empty. Treat `focus`, roots, and listed paths as starting points, not exhaustive access lists.
- Omit `allowed` by default. Add a narrow, exhaustive positive allowlist only for the corresponding high-risk action when safety, privacy, compliance, production impact, external people, or irreversibility requires it. Give every supplied item an `id`, `action`, `target`, and `constraints`; an unmatched high-risk action is prohibited.
- Use only main-agent-verified items in `confirmed_facts`; put unverified premises in `assumptions_to_check`.
- Fill `role_input` with the selected role template.
- Inline the complete public result, selected `role_output`, and, when effects are possible, the effect item below. Never send a reference the worker cannot access.
- Set `max_public_result_bytes` to one finite positive integer sized for the task. It bounds the whole serialized public result, including the selected `role_output`, inventories, and locators. Keep raw logs and large details in authorized artifacts or native Runtime items and return only checkable locators.
- State return conditions for success, partial completion, and a clear blocker. Do not let the worker redefine completion.

## Require one public result

Require exactly one public result envelope and exactly one selected `role_output`:

```json
{
  "status": "completed | partial | blocked | failed",
  "summary": "what actually happened and was found",
  "objective_results": [
    {
      "objective_id": "O1",
      "status": "achieved | partial | blocked | failed",
      "result": "actual result",
      "evidence": []
    }
  ],
  "assumption_results": [
    {
      "assumption_id": "BA1",
      "assessment": "confirmed | contradicted | inconclusive | not_evaluated",
      "impact": "effect on the result",
      "evidence": []
    }
  ],
  "done_when_results": [
    {
      "done_when_id": "D1",
      "status": "satisfied | unsatisfied | unknown | not_evaluated",
      "evidence": [],
      "reason": "reason when needed"
    }
  ],
  "boundary_compliance": {
    "status": "compliant | violation | unknown",
    "violations": [
      {
        "rule_ref": "F1, AL1, or an objective id for unrelated drift",
        "description": "actual or suspected violation",
        "effect_ids": ["E1"],
        "evidence": []
      }
    ]
  },
  "effects": [
    {
      "id": "E1",
      "target_ref": "a role-input target id",
      "target": "affected object",
      "operation": "created | modified | deleted | executed | external_action",
      "authorization": {
        "status": "authorized | unauthorized | ambiguous",
        "refs": ["O1", "a role target id", "AL1 when applicable"]
      },
      "status": "completed | partial | failed | rolled_back",
      "before": "available baseline or prior state",
      "after": "observed outcome",
      "provenance": ["command, tool call, or official external object identifier"],
      "verification": ["check result or accessible locator"],
      "unexpected": [],
      "residual_risks": [],
      "cleanup": {
        "status": "not_needed | completed | pending",
        "reason": "status reason",
        "responsible": "the main agent or a stable external owner",
        "evidence": []
      }
    }
  ],
  "artifacts": [
    {"id": "A1", "locator": "accessible location", "description": "what it contains"}
  ],
  "uncertainties": [
    {"subject": "remaining uncertainty", "reason": "why", "next_check": "optional next check"}
  ],
  "blockers": [
    {"subject": "blocking condition", "reason": "why", "needed": "what would unblock it"}
  ],
  "overflow": {
    "omitted_items": 0,
    "artifact_ids": [],
    "reason": "none | result_budget"
  },
  "role_output": {}
}
```

Require every objective, assumption, and done condition to receive a result, including unknown and not-evaluated outcomes. Keep `effects` and `artifacts` as the only authoritative inventories; role output may reference their IDs but must not duplicate them.

Return one de-duplicated synthesis. Each fact, finding, and locator appears once and may be referenced
by ID elsewhere. Record optional excess in `overflow`. The whole serialized envelope must remain
within `max_public_result_bytes`; a single item does not bypass that ceiling.
Never silently truncate correctness-critical results. If a critical result cannot fit and no
authorized stable locator is available, return `partial` or `blocked` instead of a success-shaped
summary.

Keep all IDs local to this task. The top-level `status` describes whether execution returned normally; it does not assert that every objective succeeded or that anything was accepted later.

For each attempted or actual effect, report successful, failed, partial, rolled-back, unexpected, unauthorized, ambiguously authorized, and pending-cleanup outcomes truthfully. Verify the observed result when possible. A one-shot worker cannot own later cleanup; transfer pending responsibility to the main agent or a stable external owner. Never include secrets or unrelated sensitive data.

An artifact locator only tells the main agent where to inspect content. It does not establish freshness or immutability. Separate direct observations from inferences and attach checkable locators to factual claims.

## Use the native lifecycle

Spawn through the current host's officially supported native Subagent workflow. Let the Runtime own thread creation, context assembly, tools, sandbox, permissions, status, usage, wait, follow-up, interrupt, and close behavior.

Consume the native final output, items, status, and usage directly; do not reserialize or persist a shadow copy of Runtime facts.

For Driver tasks, do not override model, reasoning effort, provider, sandbox, or approval settings. Report spawn, configuration, Runtime, and permission failures as failures. Do not switch to a custom worker, alternate transport, or background process.

Use follow-up only to clarify or complete the same envelope, baseline, and authorization. Create a new task when the objective, role, authorization, frozen material, or baseline changes. Close completed, failed, interrupted, or no-longer-needed executions through the host's normal lifecycle.

## Coordinate concurrency

- Run tasks concurrently only when inputs, work scopes, and possible effects are independent.
- Run independent read-only investigations and independent Judge reviews concurrently when useful.
- Give fanout a shared finite total result budget, divide it into child budgets, and ask each Judge for a distinct question, failure model, or counterargument. Produce one de-duplicated synthesis after inspection.
- Keep multiple Judge results independent. Never turn model count, votes, Runtime success, or recommendations into automatic acceptance.
- Serialize Drivers whose modifications overlap or compete for the same external object.
- Start a Verifier after the relevant effects have occurred and stabilized.
- Inspect independent results in parallel when useful, while serializing any later downstream acceptance or submission.
- Treat a result based on a changed baseline or frozen material as stale. Preserve it only as a lead or start a new task; still inspect and clean up effects that already happened.

## Consume the result

Before using a result:

1. Fix the final output to its native Runtime identity.
2. Check every objective, assumption, done condition, and boundary result for closure.
3. Recheck the delegation baseline and every material version used by a conclusion.
4. Inspect actual effects, provenance, artifacts, uncertainties, violations, and pending cleanup; do not rely on a summary alone.
5. Apply the selected role's coverage rules, including the Judge freeze gate.
6. Keep the result advisory and candidate-only. Independently decide what, if anything, to use or submit later.
7. Create a fresh task for a retry, changed role, changed authorization, or changed baseline instead of treating a worker thread as a persistent business actor.
