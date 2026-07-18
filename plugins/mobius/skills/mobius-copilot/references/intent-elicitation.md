# Intent Elicitation

Use this recipe only while preparing an activation or revision contract.

## Work from a small Working Set

Keep only:

- intended outcome;
- observable criteria;
- boundaries and excluded claims;
- verified facts with short sources;
- unresolved tensions;
- deferred Route notes.

This Working Set exists only in the current conversation. Do not create a Core state, Trail object,
database row, draft file, or per-turn transcript for it.

## Clarify as an expert

1. Inspect available code, documentation, configuration, and current Mobius facts before asking.
2. Classify each human statement as an outcome, fact, constraint, preference, or candidate solution.
3. Identify the single unresolved issue most likely to change the ObjectiveSpec or Map.
4. State the current interpretation, why the issue matters, and a recommendation; then ask one
   important question. Offer two or three options only when they make the trade-off clearer.
5. Update the Working Set and repeat only while a contract or Map blocker remains.

Challenge contradictions, unverifiable assumptions, missing completion criteria, and tactics that
prematurely narrow the outcome. Explain the evidence and trade-off, and let the human correct the
interpretation. Do not turn a clear request into a questionnaire.

## Know when to stop

Move to the interpretation summary when:

- the intended result is explicit;
- every Criterion is observable and has a verification rule;
- material boundaries and excluded claims are explicit;
- major contradictions are resolved; and
- a complete Map can be designed.

Stop asking when the remaining uncertainty concerns only how to execute a Route. Human technical
preferences remain advisory Route Notes; every Route is designed later by the `$mobius-loop` main
agent.

## Prepare the presentation summary

Before exact confirmation, show a concise interpretation summary and accept corrections. In the
normal Copilot path, send the complete five-field `interaction` beside `command` in the same
activation or revision call:

```json
{
  "project_root": "<canonical-project-root>",
  "project_id": "<project-id>",
  "expected_heads": {
    "expected_project_seq": 0,
    "expected_objective_seq": 0
  },
  "request_id": "<id for this exact payload>",
  "command": {
    "<activate_objective-or-revise_objective>": {
      "objective_spec": {},
      "confirmation": {}
    }
  },
  "interaction": {
    "interpreted_intent": "Markdown summary of the agreed outcome",
    "confirmed_boundaries": "Markdown summary of contract boundaries and non-goals",
    "verified_facts": "Markdown list of verified facts and short sources",
    "challenges_and_resolutions": "Markdown summary of material challenges and resolutions",
    "route_notes": "Markdown notes for later Route design"
  }
}
```

Use concise Markdown, not a transcript. Put durable acceptance requirements in ObjectiveSpec;
reserve `route_notes` for implementation preferences, rejected approaches, and hypotheses that the
Route designer must reverify.
