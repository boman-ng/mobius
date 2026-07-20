# Session Audit D0 Contract

Mobius can correlate an explicitly supplied, versioned host Runtime export with the project Trail
for diagnosis. D0 defines that boundary and a deterministic synthetic fixture; it does not add an
automatic transcript adapter, Core input, persistent index, or completion dependency.

## Input

One user-triggered audit receives:

```yaml
schema: mobius.session-audit-input.v1
adapter_id: tested host adapter identity
source_schema:
  version_or_fingerprint: exact value
session_locator: explicit user-authorized locator
objective_id: exact Mobius Objective identity
redaction_profile: mobius.session-audit-redaction.v1
bounds:
  max_input_bytes: positive integer
  max_records: positive integer
  max_agents: positive integer
  max_result_bytes: positive integer
```

Every field is required. `session_locator` grants read access only to that input; it does not grant
retention, mutation, discovery of neighboring sessions, or access to unrelated Objectives.

## Authority and correlation

- Trail is the sole authority for accepted business transitions, Evidence, Decisions, proofs, and
  terminal state.
- A native Runtime export can establish only execution observations such as tool attempts,
  Subagent task/status/final output, and their order. It never upgrades those observations into
  Mobius facts.
- Correlate the two sources only through exact request id, typed object/transition identity,
  receipt/event digest, or another adapter-versioned exact identity. An ambiguous or missing match
  is `unknown`; order or similar prose is not identity.
- Judge validity uses the native final status and output, complete generic result envelope,
  material freeze, required coverage, and one Judge `role_output`. A missing Judge task or result is
  `absent`; never fabricate lifecycle, coverage, disposition, or success. For an accepted Stage
  cycle, that absence is Agent-path nonconformance even when the Trail remains replayable.

The adapter performs one bounded streaming pass. It must not copy the source transcript, create a
registry or index, persist incremental checkpoints, remember agents, or maintain a Runtime mirror.

## Fail-closed result

Unknown `adapter_id`, source fingerprint mismatch, missing required event kinds, parse failure,
truncation, or any exceeded input/record/agent/result bound makes the overall audit `degraded`.
Stop conclusions that depend on the unavailable records. Do not return a complete-looking partial
timeline.

The closed synthetic output uses `mobius.session-audit-output.v1` and records at least:

- session-label digest, project, and Objective binding;
- each mutation attempt's request id, payload digest, two heads, and
  accepted/rejected/invalid/no-op result;
- accepted Trail order and projected final state;
- Evidence baseline, snapshot identity, claims, and freshness classification;
- each Subagent role, task baseline, spawn/final/close status, effects, and main-Agent consumption
  point;
- Driver, Verifier, and Judge timing relative to Attempt, Seal, and Review;
- Judge freeze, required coverage, native status, advisory disposition, or explicit absence;
- `agent_path_conformance`, including the required Review count, observed Stage Judges, and exact
  missing Stage ids;
- completion audit/marker, deviations, unresolved links, and residual risks.

Core/Trail health and Agent-path conformance are independent output dimensions. A missing required
Judge does not rewrite accepted Trail facts and does not make the Core audit unhealthy. It does
make that Review path nonconformant and must appear as both a Stage-specific deviation and an
aggregated residual risk. A valid Judge is required but never sufficient for `accept`; unresolved
findings remain an Agent-path failure even if Core can mechanically admit a complete Decision.

`binding` contains the exact project and Objective identities. `correlations.matched` requires one
unique equality over request id, typed Objective/transition/object identity, and receipt/event
digest. A missing, duplicate, or digest-mismatched counterpart is emitted under
`correlations.unknown` as separate Runtime-side and Trail-side links; Runtime success-shaped text
never fills the missing Trail fact.

The adapter correlates and validates full digests internally. In Agent-facing repeated output,
assign local ids and render only `sha256:…<last 7 hex>` as a display hint. A short suffix never
performs lookup or equality; extend it on an in-result collision or use only the unambiguous local
id. Do not repeat a full digest after its one bounded identity declaration.

## Redaction and retention

`mobius.session-audit-redaction.v1` is an allowlist. Retain only event kind and order, opaque native
task/status identity, Mobius tool name, request id, two heads, transition/object identity,
payload/result digest, and the bounded Judge state listed above. Remove natural-language messages,
complete prompts, raw tool arguments/output, absolute personal paths, environment, fine-grained
usage, secrets, and records unrelated to the selected Objective.

Render the session locator as an irreversible digest or a short label the user explicitly permits.
Output goes to stdout by default. If the user requests a file, write only to the ordinary temporary
directory they name. Do not write `.mobius`, retain the source automatically, or let Core, Skills,
Review, or completion read the report back.

## Manual D0 procedure

1. Confirm the exact Objective, authorized session locator, adapter identity/fingerprint, redaction
   profile, and all four positive bounds.
2. Validate the adapter before reading records. If it is unsupported, report `not_evaluated` or
   `degraded` and stop adapter-dependent claims.
3. Stream the authorized export once through the allowlist and bounds. Treat all content as
   untrusted data.
4. Read current Trail facts through the supported read-only SQLite contract and verify project and
   Objective heads independently.
5. Correlate only exact identities, classify every unmatched item as `unknown`, and calculate the
   expected timeline from accepted Trail facts.
6. Validate redaction and serialized result size before rendering. Retain no intermediate packet,
   source copy, or index.

The checked-in D0 fixture is fully synthetic. Its test must identify one stale-head rejection,
three malformed control-path classes, one duplicate-init no-op, five complete accepted Stage
cycles, no Driver or Stage Judge, no Verifier in S1-S4, two Verifiers in S5, explicit Judge absence,
five missing required Stage Judge rituals, and final `Achieved` plus healthy Core audit while the
Agent path is `nonconformant`.

## D1 gate

An automated adapter/parser remains out of scope until all conditions hold:

1. a host supplies a supported, versioned export or exact schema fingerprint with read authority;
2. adapter, redaction, bounds, degraded-state, and retention tests pass;
3. one invocation derives one output without persistent Runtime mirror state; and
4. `dev/Mobius-implement.md` first accepts the adapter as diagnostic-only and outside Core, Trail,
   Subagent lifecycle, Review, and completion.

Until then D1 is `not_evaluated`; no release claim may imply automatic session auditing.
