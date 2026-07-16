# Mobius Plugin Development Instructions

This file extends higher-level Codex instructions for this repository. It governs development,
testing, and release work for Mobius source. It is contributor guidance only and must never become
part of the installed runtime contract.

## Project Map

- Theoretical model: `dev/mobius-model.md`.
- Subagent blueprint: `dev/Mobius-subagent.md`.
- Engineering blueprint and phase gates: `dev/Mobius-implement.md`.
- Plugin manifest: `plugins/mobius/.codex-plugin/plugin.json`.
- Rust package: `plugins/mobius/runtime/`.
- Domain types, guards, reducer, and invariants: `plugins/mobius/runtime/src/domain/`.
- Application service and live admission: `plugins/mobius/runtime/src/application/`.
- SQLite and artifact adapters: `plugins/mobius/runtime/src/infrastructure/`.
- Human report renderer: `plugins/mobius/runtime/src/presentation/`.
- MCP, CLI, and hook adapters: `plugins/mobius/runtime/src/transport/`.
- Model Composition skills: `plugins/mobius/skills/mobius-copilot/` and
  `plugins/mobius/skills/mobius-loop/`.
- Independent delegation skill: `plugins/mobius/skills/mobius-subagent/`.
- Bundled MCP and hooks: `plugins/mobius/.mcp.json` and `plugins/mobius/hooks/hooks.json`.
- Marketplace entry: `.agents/plugins/marketplace.json`.
- Release-facing docs: `README.md`, `docs/`, `CHANGELOG.md`, `SECURITY.md`, `LICENSE`, and
  `.github/`.

The v1 skills, MCP config, and hooks use the single packaged Rust binary. Do not restore v0.5
files, launchers, or review services alongside them.

## Source Of Truth

- `dev/mobius-model.md` owns the theoretical objects, transitions, completion rule, and `I1..I19`.
- `dev/Mobius-implement.md` owns module boundaries, persistence, artifact, API, report, transport,
  phase, and release contracts.
- `dev/Mobius-subagent.md` owns generic delegated-task semantics and must remain independent of
  Core.
- Rust domain code owns the executable typed mapping and reducer only where it faithfully realizes
  the blueprints.
- Trail is the only business fact source. Projections and human views are derived and rebuildable.
- The application service is the only mutation owner. MCP is the only normal mutation transport.

If code and blueprint disagree during v1 construction, treat the discrepancy as unfinished work;
do not silently redefine the blueprint through tests or implementation convenience.

## Architecture Rules

- Produce one Cargo package and one executable target named `mobius`.
- Keep domain code pure: no filesystem, time, environment, host, Runtime, transport, SQLite,
  presentation, or Subagent dependency.
- Keep Subagent resources free of Objective, Map, Stage, Attempt, Evidence, Decision, Trail,
  database, Core path, and Core API knowledge.
- Keep Mobius local-only; add no hosted service, telemetry, global daemon, or network requirement.
- Maintain one SQLite database at `<canonical-project-root>/.mobius/mobius.sqlite3` when Phase 2
  exists. Do not add home, XDG, system-temp, or global fallback state.
- Fail closed at every review, artifact-integrity, stale-head, confirmation, and parser boundary.
- Keep CLI read/audit/doctor/report/hook adapters free of business mutation commands.
- Do not add a Python runtime, fallback, compatibility shim, parallel ledger, sidecar, second
  executable, or v0.5 import path.
- Preserve explicit user activation: model skills apply only to explicitly targeted Mobius
  Objectives.

## Archive And Generated State

- Tag `v0.5.0` is the durable v0.5 source archive. `.tmp/mobius-v0.5.0*` is a local, ignored
  inspection copy and must never be used by runtime or release checks.
- `.tmp/dev-d2-v0.5/` contains checksummed derived v0.5 diagrams and is not a v1 source of truth.
- `.mobius/`, `.tmp/`, Cargo `target/`, virtual environments, caches, and bytecode stay untracked.
- Never run broad ignored-file cleanup without protecting `.tmp/` archive material.

## Phase Discipline

- Phase 1 is complete only with the single-binary skeleton, all eleven object types, every model
  transition and guard, `I1..I19`, deterministic reducer/replay, and Manifest equivalence tests.
- Phase 2 adds project binding, SQLite, artifacts, Core service, MCP, safe reports, recovery, and
  crash tests.
- Phase 3 adds the independent Subagent skill and its thirteen acceptance conditions.
- Phase 4 adds Composition skills, human confirmation flow, hooks, packaging, both end-to-end
  paths, and release gates.
- Later-phase absence must be explicit. Never use a fallback, alias, stub success, or second engine
  to make an incomplete phase look complete.

## Commit Rules

- Use Conventional Commits.
- Each commit must be atomic and contain no more than four distinct changes.

## Verification Matrix

For Rust or phase-facing changes, run:

```bash
export CARGO_TARGET_DIR="$PWD/.tmp/cargo-target"
cargo fmt --manifest-path plugins/mobius/runtime/Cargo.toml --all --check
cargo check --manifest-path plugins/mobius/runtime/Cargo.toml --locked --all-targets
cargo clippy --manifest-path plugins/mobius/runtime/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked --all-targets
git diff --check
```

Also verify ignored state for repository hygiene:

```bash
git check-ignore -q .mobius/probe
git check-ignore -q .tmp/probe
git check-ignore -q plugins/mobius/runtime/target/probe
```

Use the narrowest relevant test first. A phase or release claim additionally requires every gate
listed for that scope in `dev/Mobius-implement.md`; the commands above are not a substitute for
SQLite, durability, protocol, clean-host, cross-target, or end-to-end evidence once those phases
exist.

## Review Before Finishing

- Inspect diffs for scope expansion, personal paths, secrets, stale repository coordinates,
  generated state, and hidden dependency on the development checkout.
- Confirm active files contain no Python runtime or v0.5 behavior contract.
- Confirm the manifest names the current phase honestly and points only to components that exist.
- Confirm marketplace name/path remain aligned with the plugin manifest.
- Run an independent cross-review for non-trivial code, architecture, or release claims.
- Never claim v1 release readiness from a narrow active-phase test result.
