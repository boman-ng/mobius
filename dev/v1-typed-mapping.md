# Mobius v1 Typed Mapping

Status: implemented and covered by the v1 runtime. This document records the mechanical
boundary choices for `mobius-model.md`; it does not replace that model or declare compatibility
with a different persistence schema.

## Canonical Rust Owner

`plugins/mobius/runtime/src/domain/types.rs` is the mechanical type mapping. It depends only on the
Rust standard library and uses ordered collections so equality and traversal are deterministic.

| Model object | Rust type | Typed identity |
|---|---|---|
| Objective | `Objective` | `ObjectiveId` |
| ObjectiveSpec | `ObjectiveSpec` | `(ObjectiveId, revision)` |
| MapRevision | `MapRevision` | `(ObjectiveId, map revision)` |
| Stage | `Stage` | `StageId` |
| Criterion | `Criterion` | `CriterionId` |
| Route | `Route` | `RouteId` |
| Attempt | `Attempt` | `AttemptId` |
| Evidence | `Evidence` | `EvidenceId` |
| ReviewPacket | `ReviewPacket` | `ReviewPacketId` |
| ReviewDecision | `ReviewDecision` | `ReviewDecisionId` |
| WaitCondition | `WaitCondition` | `WaitConditionId` |

The model does not prescribe a byte-level identity algorithm for semantic IDs. The live admission
boundary must reject empty IDs and reject `IdentityConflict`: the same typed ID associated with
different structure. IDs must never be derived from timestamps, filesystem paths, database row
IDs, host threads, Runtime IDs, or presentation values.

## Canonical Collections And Values

- Mathematical sets use `BTreeSet`.
- Total or partial functions and identity-indexed object sets use `BTreeMap`.
- Sequence is used only where order is part of the value.
- Inline observations use `CanonicalValue`, which excludes floating point and represents objects
  with ordered string keys.
- Frozen observation has exactly two variants: `Inline(CanonicalValue)` and
  `CoreSnapshot(digest, size_bytes)`.
- SQLite rows, JSON object member order, insertion order, and report row order do not define
  theoretical identity.

## State And Lifecycle Mapping

`ObjectiveState` is the exact five-variant sum type `Idle | Mapping | Navigating | Achieved |
Abandoned`. `NavState` is the exact five-variant sum type `SeekingRoute | Ready | Attempting |
Reviewing | Waiting`.

Route lifecycle is `Available | Rejected`. Attempt lifecycle projects exactly `Running | Sealed |
Closed`; seal and close reasons remain in their immutable transition inputs and are derived from
the Trail rather than copied into lifecycle state. Proof invalidation, Route status, and Attempt
state are derived projection values, never independent facts.

`ObjectKnowledge` is a read-only newtype outside the domain owner. Its checked insertion derives
the key from the value identity and rejects structural rebinding. `DomainConfiguration` exposes
read-only accessors. Only reducer and replay construct authoritative configurations. SQLite
projections serialize these values as rebuildable observations; direct Agent SQL can inspect them
but never hydrates a second authoritative configuration or supplies admissible transition input.
Full audit and rebuild own Trail-to-projection equivalence.

## Transition Mapping

`TransitionInput` has one variant for each model relation:

1. `ActivateObjective`
2. `InstallMap`
3. `AddRoute`
4. `SelectRoute`
5. `StartAttempt`
6. `RecordEvidence`
7. `SealAttempt`
8. `Decision`
9. `CheckWait`
10. `RequestRemap`
11. `ReviseObjective`
12. `Abandon`

Application callers do not submit the model-level `SealAttemptInput`. The service accepts only
the current Attempt identity and seal reason, then materializes the unique complete Packet in the
locked admission prestate.

`TrailFact` is the immutable Objective-scoped pair `(objective, input)`. Its transition kind is
always derived from the `TransitionInput` variant, never stored as a second discriminator. Replay
rejects mixed Objective streams and facts whose Objective is inconsistent with the activation
payload, the current stream, or an Objective-bearing input.

Agent SQL rows are not persistent model mappings. Ordinary queries select only the exact state
fields, identities, or bounded Trail range needed at the current heads. Formal Review follows the
frozen Packet/dependency identity closure; formal Wait returns count/bytes from one snapshot and
emits all matching Evidence only when the complete set fits its Context budget. A projection row never changes object identity, authorizes a write, or
becomes admissible transition input by itself.

Human confirmation for Activate and Revise contains the exact typed ObjectiveSpec payload, action,
project identity, and expected project/objective heads. Abandon confirmation contains the exact
project identity, Objective identity, reason, and heads. The optional host/UI audit reference from
the blueprint is not part of either typed confirmation: if a real host contract later requires it,
its owner is reducer-inert event-envelope metadata, not the transition payload or object identity.

## Event Schema Boundary

Each transition kind has one stable snake-case schema name. The persistent event codec uses the
single `mobius.trail-event.v1` schema and enforces all of the following:

- one explicit schema version and deterministic parser;
- canonical encoding of every object field and ordered collection;
- rejection of unknown versions, variants, fields where closed, duplicate keys, non-canonical set
  members, invalid numeric ranges, and identity conflicts;
- byte-for-byte deterministic re-encoding tests;
- no upcaster, v0.5 import, heuristic parser, or projection fallback.

The selected implementation direction is one thin, versioned strict-JSON event envelope around
explicit serde mappings on the domain values. Serde is a deterministic boundary codec here: it
does not give domain code filesystem, database, transport, clock, or Runtime dependencies. A full
outer DTO mirror was rejected because it would duplicate every field, variant, identity, and
conversion rule without protecting an existing compatibility contract.

Closed structs reject unknown fields, variants have explicit stable names, string-keyed maps and
sets retain their `BTreeMap`/`BTreeSet` order, and a decoded event must re-encode byte-for-byte to
the original canonical bytes. That comparison rejects duplicate or reordered members, alternate
whitespace, trailing values, and other normalized spellings. A bounded shape walk runs before
typed decoding; map keys are checked against each value's identity, and replay rejects historical
identity rebinding. Unknown versions and variants fail closed. Digest syntax,
byte/depth/string/collection limits, twelve golden transition encodings, the full `i128` numeric
boundary corpus, identity conflicts, and deterministic round trips have executable coverage in
`domain/codec.rs`.

## Mechanical Coverage

Type-local tests assert eleven disjoint object kinds, twelve transition families, auditable
identity formulas, structural-conflict detection, and lifecycle state exposure. Codec, guards,
reducer, invariant audit, replay, and generated state-machine tests remain separate owners so none
can silently use itself as its only oracle.
