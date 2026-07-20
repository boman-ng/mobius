# Risk, Delegation, and Result Gate

Load this recipe before `StartAttempt`, after the Attempt's effects stabilize, and for the required
Judge ritual after Review material is recursively closed and frozen. This is an ephemeral
Composition control. It is not a Core object, vote, or second lifecycle.

## Build one Stage Risk Card

Use current typed Stage, Route, Acceptance Context, and freshly inspected project facts:

```text
Stage/Route:
Effects: none | read-only | bounded files | external/reversible | external/irreversible
Risk dimensions:
  migration/storage:
  concurrency/lease/locking:
  filesystem/path/symlink/permissions:
  security/trust boundary:
  crash/recovery/durability:
  cross-stage integration:
  unknown or weakly testable behavior:
Delegation:
  Driver: use | skip; expected information or execution value:
  Verifier: recommend | skip; distinct failure model and method:
  Judge: required at Stage Review; pending | current | unusable; frozen-material question:
Evidence independence:
  required coverage:
  available methods:
  selected methods:
Fresh baseline: exact material identity | pending
```

Create the card once before the Attempt and revise it only when a newly observed effect, risk, or
baseline changes the decision. Keep it in current context. Never write it to Trail, `.mobius/`, a
worker registry, or another persistence surface.

## Select work and reserve the Stage Judge ritual

- Use a Driver when one coherent change has a finite write set, explicit authorization, a checked
  baseline, observable effects, and declared validations. Skip it when authorization is ambiguous,
  the work is tightly coupled, or the main Agent's next action immediately depends on doing the
  work itself.
- Prefer a Verifier for migration, concurrency or lease behavior, filesystem and symlink
  containment, permissions or security boundaries, crash recovery, durability, external effects,
  and cross-owner integration. Start it only after the relevant effects have happened and
  stabilized. A Verifier observes; it does not repair its subject.
- For one required Judge execution per Stage Review, first complete recursive Packet closure and
  freeze the exact materials, questions, criteria, known risks, and required coverage. Create a
  fresh generic Judge task only then. The required execution is an independent challenge, remains
  advisory, and never performs the formal Review.
- Additional Judges require distinct information value: each must cover a separate unresolved
  question, failure model, or counterargument under one finite fanout budget.

Driver and Verifier use the selection signals above. Do not require a Driver for every Attempt,
map risk labels mechanically to roles, fix an additional worker count or role order, or use a vote
threshold. The required Stage Judge is the sole built-in role ritual; an accepted ObjectiveSpec,
Stage verification rule, or separately human-approved risk policy may impose further verification
requirements when visible before the Attempt.

## Gate Evidence independence and coverage

For each material high-risk failure model, require at least one method capable of exposing that
failure independently of the implementation path. A suitable method may be a Verifier, a
deterministic adversarial fixture, fault injection, an independent CI lane, a comparison
implementation, or a direct reproduction whose mechanism differs from the implementation.

Record the failure model, method, observed coverage, counterevidence, and limits in the Evidence
Bundle. Method names or successful process status do not establish coverage. If no available
method covers a required failure model, keep the related Criterion unresolved; the reason is
insufficient Evidence, not a missing role.

The main Agent may perform all Stage work and verification without Driver or Verifier, using
sufficiently independent direct checks. There is no Judge-free Composition accept path. If the
required Judge cannot return current, complete, valid advice, Core state does not move
automatically; the main Agent keeps the Stage in Review or chooses an evidence-backed non-accept
outcome allowed by the Model.

## Construct every Mobius delegation

Use the generic `mobius-subagent` task and selected-role contracts. In addition to the generic
envelope, every Mobius delegation must include both integration boundaries exactly:

- Do not call any Mobius MCP tool.
- Do not read or write `.mobius/` managed state.

Give the task a finite result budget, a current delegation baseline, explicit objectives and DONE
conditions, the selected role's complete input/output shape, and all effect authorization. Do not
pass a Core handle, mutation instruction, business continuity, or remembered worker state.

## Validate the native final result

Fix the result to its native Runtime task identity, status, items, and final output. Then check, in
order:

1. exactly one complete public result envelope and exactly one selected `role_output` exist;
2. every objective, assumption, DONE condition, and boundary item is closed, including unknown or
   not-evaluated outcomes;
3. effects, authorization, provenance, verification, unexpected impact, cleanup ownership,
   artifacts, uncertainties, blockers, and overflow are internally complete;
4. the delegation baseline and every material freeze still match current facts;
5. the selected role's required subjects, checks, questions, criteria, risks, and coverage are
   complete; and
6. the serialized result obeys its finite byte budget without silent correctness-critical
   truncation.

`completed`, a successful spawn, a help/template check, or a nonempty summary does not make a
result valid. Timeout, interruption, spawn/configuration/permission failure, missing final output,
invalid envelope, stale baseline, partial coverage, or unresolved cleanup remains failed,
degraded, stale, partial, or inconclusive as applicable. Never synthesize a successful envelope.
Inspect actual effects even when the result is unusable.

Changed objective, role, authorization, baseline, or frozen material requires a fresh task. Do not
patch an old result or continue a thread as a persistent business actor. Every accepted downstream
fact remains a new, independent main-Agent decision.

## Apply the Judge freeze gate

A Judge task has nonempty `materials`, `questions`, and `criteria`. Every material declares one of
`inline`, `content_digest`, `immutable_version`, or `immutable_object_id`; every question,
criterion, and known risk names all necessary material ids and its required coverage.

Treat material, freeze-check, coverage, answer, criterion, risk, and severity statuses as the
closed enums in the generic Judge contract. Compare each nonempty `required_coverage` with the
actual material coverage; never accept arbitrary coverage text as a status. Close every material,
question, criterion, risk, finding, recommendation, artifact, and evidence reference by its
task-local id. Missing, extra, duplicate, unknown, or cross-inventory references make the result
invalid before any disposition is consumed.

The main Agent checks each freeze before consuming advice:

| Material and coverage | Permitted assessment | Overall disposition |
|---|---|---|
| every necessary freeze matched and required coverage complete | determinate or inconclusive | one supplied option or inconclusive |
| matched freeze with partial coverage | inconclusive | inconclusive |
| stale or mismatched freeze | inconclusive | inconclusive |
| unverifiable or inaccessible material | inconclusive | inconclusive |

Findings and recommendations cannot bypass this table. A Runtime-advertised qualifying external
profile may be selected when an independent model-family question has material value. If none is
advertised, or spawn/final-output validation fails, record external Judge advice as unavailable or
degraded. An internal Judge remains internal and must not be relabeled as external.

Judge output is advice only. It cannot become Evidence, Decision, proof, transition, completion,
or a substitute for the main Agent's recursive identity closure, applicability classification, and
formal `ReviewDecision`.

A missing Judge task or result is `absent`, never an inferred review lifecycle or disposition, and
blocks `accept`. Unavailable, degraded, stale, partial, or inconclusive advice also blocks
`accept`. Only a valid current result with matched freezes and complete required coverage can fill
the required Judge slot, and it is necessary but not sufficient: unresolved Judge findings block
`accept`, while the main Agent still owns the complete formal Review and final disposition.
