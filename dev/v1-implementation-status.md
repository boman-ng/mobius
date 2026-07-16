# Mobius v1 Implementation Status

Updated: 2026-07-16. The three `dev/` blueprints are authoritative. This file records phase status,
evidence locations, the supported v1 boundary, and residual risks without duplicating blueprint
or test detail.

## Phase Status

| Scope | Local status | Primary evidence |
|---|---|---|
| v0.5 archive and active-path cleanup | Complete | Tag `v0.5.0` retains the old source; active plugin paths contain no Python runtime, CSV ledger, Review MCP, launcher, fallback, or compatibility engine |
| Phase 1: binary and domain Core | Complete | `runtime/src/domain/`, `phase1_contract.rs`, generated state-machine coverage, and `I1..I19` audit checks |
| Phase 2: store, artifacts, API | Complete | SQLite/artifact process-loss tests, Core service tests, MCP protocol tests, report/CLI contracts, and recovery/audit checks |
| Phase 3: independent Subagent skill | Implemented; deterministic contracts green | `mobius-subagent`, its thirteen-condition contract tests, and the historical native-host evidence below |
| Phase 4: Composition and release | Implemented; deterministic local gates green | Copilot/Loop ownership contracts, public MCP accept/retry/replace/wait/remap/revise/abandon coverage, direct and delegated loops, Hook tests, and bundle gates |

These statuses describe implementation and deterministic local evidence. They do not publish a
release, promote the source marketplace from `NOT_AVAILABLE`, or replace exact-candidate native-host
and independent review gates.

## Ownership And Normal Paths

- One Rust package exposes one executable target named `mobius`.
- Trail is the only business fact source; projections and reports are derived and rebuildable.
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
  checks, Packet materialization, and next actions.
- `plugins/mobius/runtime/src/infrastructure/`: one project-bound SQLite database family, durable
  artifacts, crash recovery, projection rebuild, and integrity checks.
- `plugins/mobius/runtime/src/presentation/`: context-dark, pinned, replaceable human reports.
- `plugins/mobius/runtime/src/transport/`: stdio MCP, read/audit/doctor/report CLI, and lifecycle
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
historical evidence rather than evidence for the current candidate. Publication requires rerunning
the native-host gate against the exact release candidate as specified in
`docs/release-checklist.md`.

### Packaging and Release

- `.github/workflows/ci.yml`: pinned source and target jobs.
- `.github/scripts/`: reproducible binary build and bundle assembly.
- `tests/release_bundle_contract.sh`: source, bundle, archive, installed-cache, Hook, MCP, and
  single-executable contracts.
- `docs/release-checklist.md`: real Codex loader and native-host gates that remain release-host work.

## Verification Snapshot

The 2026-07-16 invocation-policy and host-boundary candidate passed:

- pinned format, locked all-target check, Clippy with warnings denied, and 167/167 Rust tests;
- all three Skill validators and the executable Composition/Subagent metadata contracts;
- source package contract, shell syntax, `git diff --check`, and managed-state ignore probes;
- fresh release build, assembled-bundle validation, and isolated real Codex `0.144.5` install with
  direct and delegated public-MCP loops.

This snapshot is implementation evidence, not publication approval. Release still requires the
exact-candidate independent review and explicit release decision below.

## Supported v1 Boundary

- Runtime target: `x86_64-unknown-linux-gnu`.
- Rust toolchain: pinned `1.85.0`.
- Minimum eligible native host: stable Codex CLI `>=0.143.0` on Linux x86-64.
- Most recently verified installed-plugin host: Codex CLI `0.144.5`; every actual release host must
  rerun the complete installed-plugin gate.
- Installed executable: `plugins/mobius/bin/mobius` only.
- Source marketplace: `NOT_AVAILABLE`; only an assembled, gated copy becomes `AVAILABLE`.
- Core workflow: local-only, with no hosted service, telemetry, daemon, downloader, or network
  requirement.

## Residual Risks And Release Boundary

- The pre-tool Hook is a cooperative lexical guard, not a hostile-process sandbox. Its supported
  literal-command grammar is owned by `dev/Mobius-implement.md` and exhaustive Rust tests.
- Mutation admission replays history before commit. v1 accepts this history-proportional
  cost instead of adding a cache or second truth source.
- The installed delegated lane consumes one already validated observation; full native-result
  validation and failure matrices stay in Rust/native-host gates rather than production protocol.
- Real Codex installation, native Subagent lifecycle behavior, a new host, changed experimental
  metadata, or a new target requires fresh evidence. Checked-in CI artifacts are not publishable
  without those gates and an exact-candidate independent review.

There are no open Phase 2–4 implementation ownership decisions. Exact-candidate native-host review
and publication remain separate, explicit release decisions.
