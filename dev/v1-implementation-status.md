# Mobius v1 Implementation Status

Updated: 2026-07-16. The three `dev/` blueprints are authoritative. This file records phase status,
evidence locations, the supported v1 boundary, and residual risks without duplicating blueprint
or test detail.

## Phase Status

| Scope | Local status | Primary evidence |
|---|---|---|
| v0.5 archive and active-path cleanup | Complete | Tag `v0.5.0` retains the old source; active plugin paths contain no Python runtime, CSV ledger, Review MCP, launcher, fallback, or compatibility engine |
| Phase 1: binary and domain Core | Complete | `runtime/src/domain/`, `phase1_contract.rs`, generated state-machine coverage, and `I1..I19` audit checks |
| Phase 2: store, artifacts, API | Complete | Append-only SQLite Trail, rebuildable projections, four-tool write MCP, read-only audit CLI, artifact process-loss tests, reports, and recovery checks |
| Phase 3: independent Subagent skill | Implemented; deterministic contracts green | `mobius-subagent`, its fourteen-condition contract tests, and the historical native-host evidence below |
| Phase 4: Composition and release | Implemented; deterministic local gates green | Copilot/Loop ownership contracts, public MCP accept/retry/replace/wait/remap/revise/abandon coverage, direct and delegated loops, Hook tests, and bundle gates |

These statuses describe implementation and deterministic local evidence. They do not publish a
release, promote the source marketplace from `NOT_AVAILABLE`, or replace exact-candidate native-host
and independent review gates.

## Ownership And Normal Paths

- One Rust package exposes one executable target named `mobius`.
- Trail is the only business fact source; projections and reports are derived and rebuildable.
- Ordinary Agent reads use one literal canonical SQLite 3.40.1+ safe/read-only/query-only command.
  Skills select targeted projection fields or bounded Trail ranges; no Agent ORM or read MCP exists.
- The application service is the only mutation owner; public business mutation uses stdio MCP.
- `mobius-copilot` owns human-authorized activation, revision, abandonment, and Map installation for
  `Initial` or `SpecRevised` Mapping states. Fresh transitions and interrupted durable Mapping
  states converge on the same installation path.
- `mobius-loop` owns execution of an already active Objective, including Map installation for
  `Remap` or `WaitRevealedDrift` Mapping states.
- Host metadata disables implicit invocation for Copilot and Loop. The independent Subagent skill
  remains eligible for main-Agent discovery and selection without a second user invocation.
- The independent Subagent skill owns only generic delegated-task semantics. Main Agent Composition
  validates candidates and constructs every typed Core submission; discovery grants no additional
  permission or effect authority.
- CLI, reports, Hooks, and Subagents do not form alternate business-state paths.

## Evidence Index

### Domain and Core

- `plugins/mobius/runtime/src/domain/`: typed objects, guards, reducer, replay, and invariants.
- `plugins/mobius/runtime/src/application/`: project admission, commands, Core service, stale-head
  checks, Packet materialization, audit, and internal presentation/Stop reads.
- `plugins/mobius/runtime/src/infrastructure/`: one project-bound SQLite database family, durable
  artifacts, crash recovery, projection rebuild, and integrity checks.
- `plugins/mobius/runtime/src/presentation/`: context-dark, pinned, replaceable human reports.
- `plugins/mobius/runtime/src/transport/`: four-tool stdio MCP, audit/doctor/report CLI, and lifecycle
  Hooks over the same service and executable.

### Composition and Subagent

- `plugins/mobius/skills/mobius-copilot/`: explicit Objective contract actions and their Mapping
  ownership.
- `plugins/mobius/skills/mobius-loop/`: state-driven execution, operational remapping, candidate
  consumption, formal review, and completion gating.
- `plugins/mobius/skills/mobius-subagent/`: independent native delegation with no Core schema,
  path, API, persistence, queue, registry, or worker ledger.
- `plugins/mobius/runtime/tests/composition_skill_contract.rs` and
  `subagent_skill_contract.rs`: executable skill boundaries.
- `plugins/mobius/runtime/tests/mcp_protocol.rs`: public stdio MCP behavior, branch E2Es, direct and
  delegated Composition paths, stale/invalid candidate rejection, and context-surface checks.

### Historical Alpha Native-Host Evidence

The Codex `0.144.4` native-host gate was exercised on 2026-07-15 for the then-current exact bytes:

- `mobius-subagent/SKILL.md`:
  `192ddd21a20cd694378ff8cce7e9ca2efe0e988a46fe27c4a46b60ed63452630`;
- `references/role-profiles.md`:
  `c4ce21b73ab01b87d10f1da874fe01d0187932047f206264def932e9ad8b45d0`;
- `subagent_skill_contract.rs`:
  `7b82922907b02e15f28ae0f5cb833c8b1121513c8c031d52663aa50e60a52ee5`.

That run exercised native wait, same-envelope follow-up, normal completion, interruption, and
truthful spawn, configuration, Runtime, and permission failures. The current source intentionally
changes the Skill invocation contract and adds host metadata, so those exact-byte results remain
historical evidence rather than evidence for the current candidate.

### v1.0.0 Exact-Candidate Native-Host Evidence

The Codex `0.144.5` native-host gate was rerun on 2026-07-16 for the v1.0.0 candidate and these
exact bytes:

- `mobius-subagent/SKILL.md`:
  `630cfa479870ff3d2e93d7a3e337dde771ad49d3288f296b2f0b3f89483a65b3`;
- `agents/openai.yaml`:
  `26fbe980ab78ee4b6dc5fcf7194501fba95ca3578a1a7550cc650864e5a965b9`;
- `references/role-profiles.md`:
  `c4ce21b73ab01b87d10f1da874fe01d0187932047f206264def932e9ad8b45d0`;
- `subagent_skill_contract.rs`:
  `afc7773a527b569e7b4950931a342360abe3a5fb9c497b6e3d9bd7c1f010c963`.

The run used an opaque native identity without model, provider, effort, sandbox, approval, or
permission overrides. It exercised spawn, wait, same-envelope follow-up, normal completion, schema
correction before consumption, and interrupt. Both Mobius forbidden
boundaries remained explicit and observed. No configuration, Runtime, or permission failure
occurred in the successful run, and no alternate runtime, elevation, retry transport, or
success-shaped fallback was used. The validated native task, corrected complete result, and opaque
identity were supplied transiently to the delegated stdio MCP E2E; its stale, partial, failed,
unauthorized, cleanup-pending, missing-boundary, and missing-provenance matrix rejected every
invalid candidate before Core submission, then the valid candidate reached `Achieved` and a healthy
audit. Native task/result/identity material was not persisted in the repository.

The Context-optimization work after v1.0.0 changes the Subagent Skill, its contract test, the Agent
read protocol, and MCP output. The hashes and installed-host run in this section are therefore
historical release evidence, not evidence for the current working tree.

### Packaging and Release

- `.github/workflows/ci.yml`: pinned source and target jobs.
- `.github/scripts/`: reproducible binary build and bundle assembly.
- `tests/release_bundle_contract.sh`: source, bundle, archive, installed-cache, Hook, MCP, and
  single-executable contracts.
- `docs/release-checklist.md`: real Codex loader and native-host gates required for each exact
  release candidate.

## Historical v1.0.0 Verification Snapshot

The 2026-07-16 invocation-policy and host-boundary candidate passed:

- pinned format, locked all-target check, Clippy with warnings denied, and 167/167 Rust tests;
- all three Skill validators and the executable Composition/Subagent metadata contracts;
- source package contract, shell syntax, `git diff --check`, and managed-state ignore probes;
- fresh release build, assembled-bundle validation, and isolated real Codex `0.144.5` install with
  direct and delegated public-MCP loops;
- exact-byte native Subagent spawn/wait/follow-up/completion/interrupt and native-result delegated
  stdio MCP validation through `Achieved` and healthy audit.

This snapshot is implementation evidence, not publication approval. Release still requires the
exact-candidate independent review and explicit release decision below.

## Current Working-Tree Context Snapshot

The current candidate deletes the public read service, CLI read mode, MCP read and artifact-read
tools, cursor/chunk DTOs, ORM-only infrastructure, and their compatibility tests. MCP now exposes
exactly project initialization, artifact capture, typed transition, and maintenance-bound audit.
Protocol E2Es observe state through read-only SQLite; Hook coverage admits only the exact supported
CLI shape and proves two-stage SQLite/shell quoting on pathological identities. Composition Skills
define targeted ordinary reads, progressively disclosed recursive Review closure, count/byte-
admitted single-snapshot Wait enumeration, and head rechecks. Tracked fixtures execute a convergent
multi-level Review DAG and the exact Wait reference SQL, including denial and truncation.

The current candidate passes format, locked all-target check, Clippy with warnings denied, and all
165 Rust tests. All three Skill validators, plugin validation, the source contract, final assembled
bundle gate, and isolated real Codex `0.144.5` install gate pass. The installed direct and delegated
loops use four-tool MCP writes, canonical read-only SQLite observations from an ordinary external
cwd, exact Core-owned review material, `Achieved`, and healthy CLI audit.

A checksum-verified official SQLite 3.53.3 source build supplies the ignored local prototype CLI.
The exact safe/read-only/query-only command rejects DDL and `writefile()`. The formal Wait query
returns one admitted zero-match summary; on 1,000 matching Evidence rows it reports count/bytes and
returns only a 120-byte summary when the declared budget fails. The first independent
exact-candidate review found quoting, transitive Review, and tracked-Wait gate blockers; this tree
contains their focused remediations. A later stateless review returned no objections and identified
only unreachable audit cursor metadata; final6 deletes that metadata. The final stateless Level-1
review of final6 reports no objections, blockers, required revisions, or minor comments. Current
native-Subagent evidence remains separate before any release-readiness claim.

## Supported v1 Boundary

- Runtime target: `x86_64-unknown-linux-gnu`.
- Rust toolchain: pinned `1.85.0`.
- Minimum eligible native host: stable Codex CLI `>=0.143.0` and canonical SQLite CLI `>=3.40.1`
  on Linux x86-64.
- Most recently verified installed-plugin host: Codex CLI `0.144.5`; every actual release host must
  rerun the complete installed-plugin gate.
- Installed executable: `plugins/mobius/bin/mobius` only.
- Agent read prerequisite: external canonical absolute SQLite CLI `>=3.40.1`; it is not bundled.
- Source marketplace: `NOT_AVAILABLE`; only an assembled, gated copy becomes `AVAILABLE`.
- Core workflow: local-only, with no hosted service, telemetry, daemon, downloader, or network
  requirement.

## Residual Risks And Release Boundary

- The pre-tool Hook is a cooperative lexical guard, not a hostile-process sandbox. Its supported
  literal-command grammar is owned by `dev/Mobius-implement.md` and exhaustive Rust tests.
- Mutation admission replays history before commit. v1 accepts this history-proportional
  cost instead of adding a cache or second truth source.
- Ordinary Agent reads trust the transactionally maintained typed projection inside Mobius's
  cooperative private-database boundary; explicit audit/rebuild proves Trail equivalence. A hostile
  local database writer would require a separately designed immutable state-hash chain and schema
  migration.
- Direct SQL has no server-enforced row or byte cap. Skills bound ordinary queries, while formal
  Wait emits the complete matching Evidence set only when count/bytes fit the declared Context
  budget; otherwise it returns summary-only and fails closed.
- Artifact capture still carries the complete bytes in one MCP request. Skills permit it only when
  the atomic payload fits the current MCP and Context budget; an oversized required blob blocks
  instead of introducing chunking, path ingestion, or a second mutation path.
- A bounded maintenance audit can report `complete=false`; the supported response exposes totals,
  not a dead continuation token. Remediate the reported integrity class and rerun audit.
- The Hook verifies literal executable path and version per call but cannot remove the same-user
  Hook-to-exec replacement race; this remains a cooperative boundary.
- The installed delegated lane consumes one already validated observation; full native-result
  validation and failure matrices stay in Rust/native-host gates rather than production protocol.
- Real Codex installation, native Subagent lifecycle behavior, a new host, changed experimental
  metadata, or a new target requires fresh evidence. Checked-in CI artifacts are not publishable
  without the applicable gates and an explicit release decision.

There are no open Phase 2–4 ownership decisions. Release-host matrix coverage, current
native-Subagent verification, explicit release decision, and publication remain separate gates.
