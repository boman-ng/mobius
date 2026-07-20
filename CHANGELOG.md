# Changelog

All notable changes to Mobius are documented here.

## 1.3.0 - 2026-07-20

- Add canonical Evidence Bundle v1 guidance and a deterministic test oracle for repository,
  artifact, external-object, and intrinsic baselines, including a fixed byte budget and
  Record/Seal/Decision freshness gates.
- Route later effects that invalidate accepted proof through existing Remap/carry/reverification,
  preserve mixed-baseline Packet history, and keep terminal Objectives immutable.
- Select Driver and Verifier by information value and independent failure-model coverage; require
  one fresh advisory Judge in every Stage Review, and validate generic native final results,
  freeze, coverage, budget, overflow, and degraded outcomes without assuming a particular review
  skill or provider.
- Add live-schema transition drafts, ten fail-closed Agent-control fixtures, exact confirmation and
  recovery rules, a synthetic D0 session-audit contract, and conditional C1/D1 gates.
- Keep full SHA-256 at machine integrity boundaries while using task-local semantic ids and optional
  last-seven-hex display hints in repeated Agent context.

## 1.2.2 - 2026-07-18

- Resolve the host SQLite executable through standalone PATH, canonicalization, and version probes
  so Agents use the supported binary instead of guessing a system path.
- Admit canonical safe read-only SQLite commands from their static shape and project binding
  without executing the host CLI inside Hook admission.
- Close bound-project `find` execution, output, and unsafe-pipeline mutation paths while preserving
  ordinary-scope effects and read-only consumers.

## 1.2.1 - 2026-07-18

- Give Copilot and Loop one explicit Agent cockpit, live-state router, submission fence, and
  deterministic re-entry path after errors, interruptions, or context compaction.
- Keep ordinary planning outside Mobius, while making activation and revision use one complete
  outer request with the five-field presentation-only interaction summary.
- Derive transition wrapper recovery guidance from the executable field table, reject unchanged
  malformed retries, and admit managed SQLite reads only through canonical shell-word encoding.
- Make Formal Review closure mechanically identity- and integrity-complete, and trigger optional
  Subagent verification only when bounded value, authority, and freshness gates pass.

## 1.2.0 - 2026-07-17

- Add expert-led, one-question-at-a-time Objective elicitation that challenges assumptions,
  separates outcomes from implementation preferences, and asks the human to confirm the typed
  Objective contract rather than design a Map or Route.
- Make the Loop main Agent design every initial, added, and replacement Route. Initial and
  specification-revision Maps now install with no predesigned Routes.
- Preserve the accepted understanding as one simple, deletable `interaction.md` per
  session/Objective revision. The summary stays outside Core hashes and Trail, is written only
  for an accepted current receipt, and is advisory input only while designing a Route.
- Select external Subagent Judges only from qualifying Runtime-advertised profiles, keep provider
  configuration Runtime-owned, and report unavailable or degraded profiles without substitution.
- Remove obsolete v1 status and typed-mapping snapshots; the three blueprints, executable gates,
  and release evidence remain the active sources of truth.

## 1.1.1 - 2026-07-17

- Make the project SQLite store explicit as an append-only Trail with rebuildable projections and
  document its stable five-table Agent read contract.
- Remove the Agent ORM, CLI read mode, MCP read and artifact-read tools, cursor/chunk DTOs, and their
  compatibility surface. Agents now use one canonical SQLite 3.40.1+ safe read-only command.
- Keep MCP focused on four validated writes: project initialization, artifact capture, typed
  transition, and explicit maintenance; read-only audit remains CLI-only. Remove unreachable audit
  cursor/digest metadata instead of exposing a continuation that no supported caller can use.
- Add two-stage SQLite/shell literal encoding, targeted SQL guidance, recursively complete Review
  closure, single-snapshot count/byte-admitted Wait enumeration, stale-head rechecks, and a Hook
  exception for only the literal supported SQLite command shape. Executable contracts cover
  pathological identities, a convergent multi-level Review DAG, and admitted/denied/truncated Wait.
- Bound each complete Subagent result with one serialized-byte budget and deduplicate fan-out while
  keeping model selection and work routing advisory and main-Agent owned.

## 1.0.0 - 2026-07-16

- Archive the complete v0.5.0 release tree outside the active source path.
- Implement the Rust single-binary cutover described by the v1 model, subagent, and implementation
  blueprints under `dev/`.
- Remove the Python runtime, CSV ledger, Review MCP, launchers, and v0.5 skills from the active
  plugin tree instead of retaining a compatibility or fallback path.
- Add project-bound SQLite Trail storage, durable artifacts, Core-owned Packet materialization,
  strict stdio MCP, read-only operational modes, and recoverable context-dark reports whose
  historical heads and Trail digest are verified before reuse or terminal refresh.
- Add a cooperative, fail-closed pre-tool guard for explicit Core-owned state and destructive
  bound-project scopes. The precise supported literal-command grammar remains owned by the
  engineering blueprint and executable Hook tests rather than release notes.
- Make the Core-owned `.mobius/.gitignore` policy self-ignoring (`*`) and hook-protected so ordinary
  cleanup cannot peel the policy before private state.
- Treat an alternate-database candidate that disappears between managed-directory enumeration and
  open as absent during concurrent bootstrap; retain fail-closed handling for every other I/O error.
- Add the independent native-Subagent skill and main-Agent Composition gates without introducing a
  worker runtime, shared Core schema, caller attestation, or automatic result-to-state adapter.
- Require explicit host invocation for `mobius-copilot` and `mobius-loop`, while keeping the
  Core-independent `mobius-subagent` discoverable for bounded delegation selected by the main
  Agent without a second user invocation.
- Name the human-authorized Objective contract skill `mobius-copilot`, with no legacy skill alias,
  second contract owner, or runtime path.
- Assign operational remap and wait-drift Map installation to `mobius-loop`, while Copilot retains
  initial and specification-revision Map ownership; interrupted durable Mapping states resume
  through the same Copilot installation path without repeating the accepted contract transition.
- Complete clean-environment direct and delegated MCP loops, full-envelope negative cases, native
  result bridging, and an isolated Codex install gate through `Achieved` and healthy audit; admit
  stable hosts `>=0.143.0`, require the actual release host to pass the complete gate, and require
  the packaged MCP runtime version to match the plugin manifest.
- Keep source marketplace installation unavailable and add an
  `x86_64-unknown-linux-gnu` assembly gate that creates a target bundle with exactly one runtime
  executable, checksums it, and revalidates a clean cache copy.
- Pin release verification to Rust `1.85.0`, remap dynamic source/home build paths, and reject
  personal build-host paths in the assembled ELF.
- Keep exhaustive Hook semantics in Rust while using only representative installed-binary probes;
  remove weaker duplicate crash and packaging coverage, and trim the delegated fixture to the field
  consumed by the installed lane where stronger process or public-boundary tests own the signal.
- Keep the checked-in marketplace unavailable and publish only the verified assembled Linux
  x86-64 target with its external checksum.

## 0.5.0 - 2026-07-09

- Replace the public model with Objective, Work Item, Criterion, Route, Route Run, Timebox,
  Evidence, Review Target, Review Judgment, Review Feedback, and Verdict.
- Add `budget.csv` as the route-run time ledger with harness-internal, external-blocking,
  external-detached, mixed, and unknown clock domains.
- Replace retry-count budgeting with Route Run Timeboxes and no-viable-route classification.
- Rename the bundled MCP surface to Mobius Review and record checkpoint or exit Review Judgments.
- Refresh skills, references, hooks, manifest metadata, and tests around the canonical v0.5 model.
- Prune old public ledgers, command names, review result blocks, and release-facing docs.

## 0.4.0 - 2026-07-08

- Previous public model before the v0.5 naming and budget cutover.

## 0.3.0 - 2026-07-07

- Previous planning and loop hardening release.

## 0.2.0 - 2026-07-07

- Previous review-recording and loop diagnostics release.

## 0.1.0 - 2026-07-06

- Package Mobius as a repo-distributed Codex plugin.
