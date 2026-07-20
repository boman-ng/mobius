# Copilot Transition Drafts and Confirmation Preview

Load this reference only after fresh typed state identifies one legal Copilot mutation. Every JSON
block is machine checked against the live MCP schema: `required` preserves schema order and
`template` contains exactly one payload with those fields. Replace placeholders with complete
typed values; never submit placeholder text.

The outer request is:

```text
project_root | project_id | expected_heads(project, objective) | request_id | command
```

Activation and revision may add the five-field top-level presentation-only `interaction` sibling.
It is not part of `command`, the confirmed ObjectiveSpec, or Core request identity.

## activate_objective

```json
{"command":"activate_objective","required":["objective_spec","confirmation"],"template":{"activate_objective":{"objective_spec":"<complete ObjectiveSpec>","confirmation":"<confirmation containing the same complete ObjectiveSpec and exact zero/live heads>"}}}
```

## revise_objective

```json
{"command":"revise_objective","required":["objective_spec","confirmation"],"template":{"revise_objective":{"objective_spec":"<complete revised ObjectiveSpec>","confirmation":"<confirmation containing the same complete revised ObjectiveSpec and exact live heads>"}}}
```

## abandon

```json
{"command":"abandon","required":["reason","confirmation"],"template":{"abandon":{"reason":"<exact reason shown to the human>","confirmation":"<confirmation binding the same reason, Objective, project, and exact live heads>"}}}
```

## install_map

```json
{"command":"install_map","required":["map","initial_routes","cover","carry"],"template":{"install_map":{"map":"<complete Initial or SpecRevised MapRevision>","initial_routes":"<empty typed Route map>","cover":"<complete CoverJudgment>","carry":"<total carry map over exactly eligible Stages>"}}}
```

Copilot installs only `Mapping(Initial|SpecRevised)` and always uses empty `initial_routes`. Copy the
current ObjectiveSpec identity, MappingReason, previous Map, and structural carry eligibility from
live typed state; do not reconstruct them from interaction prose or invocation history.

## Exact confirmation preview

Before activation, revision, or abandonment, render one preview containing:

```text
Project: <exact project identity and canonical root>
Objective: <exact stable identity; proposed revision where applicable>
Action: <activate | revise | abandon>
Heads: <expected_project_seq, expected_objective_seq>
Canonical typed payload: <the complete payload, not a summary>
Payload digest display: sha256:…<last 7 lowercase hex of the full digest>
Interpretation summary: <short human explanation>
```

`mobius.canonical-json.v1` uses UTF-8 JSON, bytewise-sorted object keys, typed integer/string/bool/
list/object/null values, no insignificant whitespace, and no floating-point values. Compute the
full SHA-256, but show only its last seven hex characters here. This display hint is neither
identity nor authentication; Core validates the full typed confirmation, payload, state, and heads.

The human confirms the complete action and typed payload. Any payload field, head, Objective,
project, action, or intervening fact-changing action invalidates the preview and confirmation.
Rebuild and show the complete preview again; never patch only the digest display or heads. The submitted
`confirmed_payload` must equal the displayed ObjectiveSpec field for field, and abandon reason must
equal the displayed reason.

## Typed extraction, initialization, and recovery

Maintain an ephemeral field ledger for schema identity, binding, both heads, typed state, exact
subject, selected template, and confirmation status. Each value comes from the current supported
read or a newly shaped typed object. Missing, extra, wrong-kind, ambiguous, stale, or unverified
input closes the fence. Never persist this ledger.

Call `mobius_project_init` only for explicit activation after the binding read is definitively
`absent`. Once its receipt is known during the current entry, do not call it again. Idempotency is
for response-loss/crash recovery, not normal control flow. After compaction or uncertainty, re-read
binding rather than assuming initialization failed.

On `invalid_tool_input`, discard the command and request id and rebuild from the selected block. On
stale head, discard the payload, semantic decision, confirmation, request id, and fence. On any
read, hook, path, schema, or binding failure, set `State=unknown` and `Fence=closed`, run standalone
host discovery/doctor checks, and submit no mutation until a complete supported cockpit is rebuilt.
Never guess heads, binding, wrapper, subject, or Context.

Retain an accepted activation/revision `interaction_path` as `exact_path` in the handoff cockpit.
If absent or lost, record `unavailable`. A later zero-match or ambiguous exact Objective/revision
lookup stays unavailable; never select the newest file or replay the accepted business transition.
