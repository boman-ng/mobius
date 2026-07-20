# Loop Transition Drafts

Load this reference only after a fresh cockpit identifies one legal Loop mutation. Each block is a
machine-checked index entry: `required` preserves live MCP schema order and `template` contains one
command payload with exactly those fields. Replace every placeholder from current typed state or a
new complete typed object; never submit placeholder text.

The outer request has one normal shape:

```text
project_root | project_id | expected_heads(project, objective) | request_id | command
```

Build `command` from exactly one entry below. Add no wrapper inside the selected command payload.

## install_map

```json
{"command":"install_map","required":["map","initial_routes","cover","carry"],"template":{"install_map":{"map":"<complete MapRevision>","initial_routes":"<typed Route map; normally empty>","cover":"<complete CoverJudgment>","carry":"<total carry map over exactly eligible Stages>"}}}
```

Use this only for `Mapping(Remap|WaitRevealedDrift)`. Copy the current ObjectiveSpec identity and
previous Map from live state. Compute structural eligibility mechanically, then let the main Agent
give an explicit `valid` or `invalid` semantic carry judgment for every and only eligible Stage.

## add_route

```json
{"command":"add_route","required":["route"],"template":{"add_route":{"route":"<complete Route with current Stage and exact StructuralContext>"}}}
```

## select_route

```json
{"command":"select_route","required":["route"],"template":{"select_route":{"route":"<exact current available Route identity>"}}}
```

## start_attempt

```json
{"command":"start_attempt","required":["attempt"],"template":{"start_attempt":{"attempt":"<complete Attempt with selected Route and current AcceptanceContext>"}}}
```

Load `risk-gate.md` first. The Attempt bound and current AcceptanceContext are new typed input and
exact live state respectively; neither comes from prose or a prior Attempt.

## record_evidence

```json
{"command":"record_evidence","required":["evidence"],"template":{"record_evidence":{"evidence":"<complete Evidence for the exact current subject and Context>"}}}
```

Load `evidence-bundle.md` first for any externally dependent observation. Only a coherent,
currently applicable bundle or a fully self-contained intrinsic observation may support a claim.

## seal_attempt

```json
{"command":"seal_attempt","required":["attempt","seal_reason"],"template":{"seal_attempt":{"attempt":"<exact current Attempt identity>","seal_reason":"<submitted | bound_reached | interrupted>"}}}
```

Callers never supply Packet, Evidence selection, or Trail prefix. Re-run the pre-Seal freshness
gate before fencing this restricted command.

## decision

```json
{"command":"decision","required":["decision"],"template":{"decision":{"decision":"<complete main-Agent ReviewDecision over exact current Packet and Criterion domain>"}}}
```

Copy Packet identity only from current `Reviewing` state after `review-read.md` closes every exact
identity and `evidence-bundle.md` classifies applicability at freshly re-read heads.

## check_wait

```json
{"command":"check_wait","required":["wait_condition","evidence","judgment"],"template":{"check_wait":{"wait_condition":"<exact current WaitCondition identity>","evidence":"<complete newly admitted Evidence map>","judgment":"<complete main-Agent WaitJudgment over the exact full set>"}}}
```

Use only the same-snapshot, all-or-none set from `wait-read.md` plus complete new Evidence. Never
reconstruct Wait identity, Context, or evidence_set from a prior batch.

## request_remap

```json
{"command":"request_remap","required":["reason"],"template":{"request_remap":{"reason":"<specific current Map or accepted-proof invalidation reason>"}}}
```

Use the proof-impact gate from `evidence-bundle.md`. A changed surface with unknown impact is not an
unaffected result.

## Typed extraction and fence ledger

For the selected entry, keep one ephemeral ledger in current context:

```text
Field | authoritative live source | expected typed identity/value | copied value | verified
Heads | schema/project/objective reads | exact pair | exact pair | yes/no
Subject/context | compact state + exact object | exact ids/bytes | exact ids/bytes | yes/no
Closure/batch | state-specific recipe | complete/all-or-none | result | yes/no/not-applicable
Draft | selected template | exact required fields | result | yes/no
```

Missing, extra, duplicate, wrong-kind, identity-mismatched, artifact-mismatched, stale, truncated, or
unverifiable input closes the fence. Keep the ledger ephemeral; it is neither Trail nor a report.

`structural_context`, AcceptanceContext, Packet identity, Decision identity, and WaitCondition
identity must come from the exact current typed object/state. Never infer them from prose, an error
message, another Stage, a previous Attempt, or remembered context.

On `invalid_tool_input`, discard the complete command and request id, select the named entry again,
and rebuild every field before a fresh head fence. On stale head, discard command, semantic
decision, closure/batch, and fence. Never retry an unchanged payload or patch only heads.
