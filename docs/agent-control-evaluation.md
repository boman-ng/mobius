# Agent Control C0/C1 Evaluation

Mobius first hardens the existing Skill-driven control path. The C0 contract uses live typed state,
canonical mutation templates, exact identity extraction, an ephemeral closure/batch ledger, exact
confirmation preview, and fail-closed recovery. It does not add a read facade.

## C0 deterministic corpus

The checked-in `agent-control-faults-v1.json` fixture has ten synthetic fault classes:

1. unsafe SQLite quote or matcher construction;
2. stale project/objective heads;
3. unknown project binding;
4. wrong outer command wrapper;
5. missing Route `structural_context`;
6. copied subject or AcceptanceContext from another Attempt;
7. incomplete Review identity closure;
8. CoreSnapshot digest/size/content mismatch;
9. Wait batch over its item/byte budget; and
10. compaction or interruption with remembered cockpit state.

Every fixture must close the submission fence before mutation and identify the one supported
recovery. Canonical template tests query the real stdio MCP `tools/list` schema and compare all
twelve transition payloads, so documentation cannot silently drift from the executable wire.

## C1 status

C1 requires all of the following evaluation volume before a trigger rate is meaningful:

- ten deterministic fault fixtures;
- at least ten allowlist-redacted representative real sessions; and
- at least one hundred transition drafts, with every transition kind represented.

Only the deterministic fixture set is checked in. The repository has no authorized corpus of ten
representative real sessions and one hundred drafts. C1 is therefore `not_evaluated`, regardless of
whether the C0 fixtures pass. Mobius does not add `inspect`, validate-transition,
confirmation-preview, a generic query enum, an MCP read tool, or another normal read path.

If a future authorized corpus reaches the minimum, record counts and calculate these triggers:

- any guessed-head/unknown-state mutation, wrong subject/context reaching Core, or Decision after
  incomplete closure;
- safe-read or compaction rebuild failure in at least two of twenty sessions;
- Composition-caused `invalid_tool_input` above one per one hundred mutation attempts; or
- mean recovery cost above one complete fence rebuild per error.

Reaching a trigger permits only an ADR proposal. Production work still requires accepted blueprint
changes to the read, application, transport, Model-skill, and release-gate sections. A future fixed
view must remain read-only, bounded, non-recommending, and unable to accept SQL, arbitrary paths or
objects, cursors, mutation fields, or next-action queries.
