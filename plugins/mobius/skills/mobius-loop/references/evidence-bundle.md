# Evidence Bundle, Freshness, and Proof Impact

Load this recipe only in `Attempting` or `Reviewing`. It governs observations that depend on
mutable work products, command output, artifacts, or external objects. It is a Composition gate:
Core still owns Evidence admission, Packet materialization, Trail, and transitions.

## Freeze one Evidence Bundle v1

Prefer a `CoreSnapshot` containing the complete canonical Bundle bytes. Use `Inline` only when the
observation is small, fully self-contained, and intrinsic: it must not rely on a mutable path,
repository state, command output, artifact locator, or external object.

```yaml
schema: mobius.evidence-bundle.v1
canonicalization: mobius.canonical-json.v1
material_baseline:
  kind: repository_worktree | artifact_set | external_object_set | intrinsic
  scope:
    include: [finite normalized locator]
    exclude: [finite normalized locator]
  before: {}
  after: {}
verification:
  - check_id: task-local unique id
    command_or_method: redacted reproducible description
    exit_status: integer | not_applicable
    output_identity: complete sha256 content identity
    assessment: supports | contradicts | unknown
observed_effects:
  changed_surfaces: [finite normalized locator]
counterevidence: [concrete observation]
limits: [uncovered scope or assumption]
```

All public fields are required and `verification` is nonempty. Every `check_id` is unique. Each
verification freezes actual output bytes or a CoreSnapshot identity; a command name, process
status, summary, or mutable locator alone is not output identity. Record meaningful failed checks
as `contradicts` or `unknown`; do not omit them to make the candidate look successful.

Canonicalize with these exact rules:

- accept only the v1 schema and canonicalization names and reject unknown fields;
- encode UTF-8 JSON without insignificant whitespace, floats, timestamps, absolute personal paths,
  secrets, unrelated raw logs, or a self-referential digest;
- sort object keys by UTF-8 bytes; sort and deduplicate set-like scope, entry, effect,
  counterevidence, and limit arrays; preserve semantic `verification` order;
- require every SHA-256 identity to be `sha256:` plus 64 lowercase hexadecimal characters;
- require every material locator to be UTF-8, project-relative, `/`-separated, and free of empty,
  `.`, `..`, `~`, control, absolute, drive-letter, and backslash segments; and
- reject canonical bytes over 131072 bytes. Narrow the atomic observation instead of splitting one
  semantic check into success-shaped fragments.

Full digests remain mandatory at capture, artifact, Bundle, freeze, MCP/Core, Trail/SQLite,
equality, lookup, and integrity boundaries. After a full value has been checked, assign a
task-local semantic id such as `B1`, `E3`, or `JM1` and use that id in repeated Agent context. If a
human needs a hash clue, render `sha256:…<last 7 hex>` only as a display hint. On a suffix collision,
extend the suffix until distinct or show only the local id. Never recover, compare, freeze, submit,
or admit by a short suffix. A delegation carries each full material digest once in its freeze
declaration and refers to it by material id thereafter.

## Capture the material kind

Use the same capture version, scope, exclusions, and toolchain inputs for `before`, `after`, and
freshness recapture.

- `repository_worktree`: require a canonical Git project root and a finite nonempty scope. Freeze
  complete `base_tree`, `tracked_delta_digest`, `untracked_manifest_digest`, and
  `toolchain_config_digest`. Use exact `HEAD^{tree}`; an unborn or non-Git workspace uses
  `artifact_set`. Hash the complete index-plus-worktree binary diff with C locale, literal
  pathspecs, full index, no color, no external diff/text conversion, and no rename detection.
  Build the untracked manifest after standard ignores; sort entries by relative path and include
  path, kind, byte size, and content digest. Hash symlink target bytes without following them.
  Reject non-UTF-8 paths, special files, containment failures, or unreadable material. Toolchain
  inputs are a sorted manifest of declared lockfiles, build configuration, feature values, and
  tool versions; an empty manifest still has a deterministic digest. Exclude `.mobius`, `.tmp`,
  and each managed Cargo target only by its resolved exact project-relative locator. Fail capture
  when that resolution is required but uncertain. A different ordinary path segment named
  `target`, such as `src/target`, remains material.
- `artifact_set`: require a finite nonempty scope and nonempty entries sorted by unique
  `logical_id`; every entry has a complete content `digest` and `size_bytes`. A locator does not
  substitute for either identity field.
- `external_object_set`: require a finite nonempty scope and nonempty entries sorted by unique
  `logical_id`; every entry freezes a stable immutable version/object identity or complete content
  digest. A mutable URL or the word `latest`, `current`, `head`, `main`, `master`, `tip`, or
  `unstable` is not an immutable version. If no stable identity exists, capture the bytes first;
  otherwise the Bundle is invalid.
- `intrinsic`: require empty scope, `before={}`, `after={}`, and a complete observation content
  identity. It cannot reference mutable external material.

For mutable kinds, `valid` means the common and kind-specific rules pass. `coherent` additionally
requires canonical `before == after`. `current-applicable` additionally requires a fresh capture of
the same kind and exact scope to equal canonical `after`. A coherent historical Bundle whose
current identity differs is `superseded`; missing, mismatched-kind, mismatched-scope, unreadable,
or unsupported current capture is `unverifiable`. Intrinsic valid observations are always
coherent and current-applicable.

## Apply all three freshness gates

1. Before `RecordEvidence`, capture `before`, perform the checks, inspect effects and cleanup, then
   capture `after`. An invalid, incoherent, over-budget, or external-prose-only candidate stays
   outside Core. Re-run against a fresh baseline; never submit it as Evidence.
2. Before `SealAttempt`, recapture every mutable scope represented by accepted Evidence in the
   current Attempt. If any required supports observation is superseded or unverifiable, remain
   `Attempting`, repeat the affected verification, and append new Evidence. Never delete or rewrite
   the old Evidence. Once sealed, Core still materializes the complete accepted Evidence set into
   the Packet.
3. Before `Decision`, first complete recursive Packet/Evidence/artifact closure. Recapture each
   distinct mutable scope once at the final fence and classify every Bundle. Read superseded and
   unverifiable history, but each mutable Criterion marked `satisfied` needs at least one covering
   current-applicable `supports` observation. Address every current-applicable `contradicts` or
   `unknown` in findings. If that cannot be done, choose a legal non-accept direction.

Any investigation, effect, validation, recapture, delegated work, or changed payload after the
final recapture breaks the submission fence. Re-read and classify again. These checks narrow the
check-to-submit race; they prove a frozen material version, never permanent external-world
freshness.

## Compute proof impact after effects stabilize

Keep one ephemeral matrix in current context:

```text
accepted Decision | owning Stage | frozen scope/baseline | changed surface | disposition
```

Classify each prior accepted proof:

- `unaffected` only when scope, toolchain, configuration, dependencies, and ownership are proven
  disjoint;
- `needs_reverification` when a changed surface intersects an accepted scope, global config,
  migration, lease/concurrency behavior, security boundary, or shared filesystem behavior;
- `requires_remap` when the Stage contract, dependency topology, Criterion ownership, or accepted
  understanding changed; and
- `unknown` when scope, ownership, identity, or external version is incomplete. In a Navigating
  state, treat this fail-closed as requiring Remap.

If only the current Stage is affected, remain `Attempting`, recapture, reverify, and append
Evidence. If an accepted Stage or its transitive dependents are affected, use the existing
`RequestRemap` path from the current Navigating state, install one new Map revision, mark affected
carry `invalid`, and re-run them. Amend the Map when the contract or topology changed. Do not
create an empty Remap for `unaffected` material.

Concrete regression anchors:

- lease timing or token changes after S3 acceptance need S3 reverification;
- symlink containment or another filesystem/security-boundary change requires Remap;
- migration or bootstrap ownership changes after S4 acceptance need S4 reverification;
- a proven S5-only documentation change can leave earlier proof unaffected; and
- incomplete ownership or scope is `unknown`, so the accept path stops.

Never edit old Evidence, Decision, Trail, or projection to express drift, and never treat a final
integration pass as automatic renewal of every prior Criterion. In `Achieved` or `Abandoned`, Core
accepts no reopening transition: report the defect as an audit finding and ask the user to create a
new Objective if repair is wanted.

The Evidence Bundle evaluator is a deterministic test oracle only. It does not enter domain code,
create another state store, resolve the external world for Core, or infer success from any
Subagent role or result.
