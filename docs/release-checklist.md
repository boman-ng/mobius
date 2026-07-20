# Release Checklist

A Mobius v1 release is publishable only after all four phase gates, every P0 gate in
`dev/Mobius-implement.md`, the supported-host checks below, and the final independent review pass
against the exact candidate. No single package, unit-test, or native-agent result is a release
claim by itself.

## Source Gate

Run from a clean checkout:

```bash
export CARGO_TARGET_DIR="$PWD/.tmp/cargo-target"
test "$(rustc --version --verbose | sed -n 's/^release: //p')" = 1.85.0
cargo fmt --manifest-path plugins/mobius/runtime/Cargo.toml --all --check
cargo check --manifest-path plugins/mobius/runtime/Cargo.toml --locked --all-targets
cargo clippy --manifest-path plugins/mobius/runtime/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked --all-targets
bash tests/release_bundle_contract.sh source "$PWD"
git diff --check
git status --short --ignored=no
git check-ignore -q .mobius/probe
git check-ignore -q .tmp/probe
git check-ignore -q plugins/mobius/runtime/target/probe
```

The source contract must prove:

- one Cargo package exposes exactly one binary target named `mobius`;
- manifest, MCP config, and hook config use their canonical relative paths;
- the source marketplace contains exactly one Mobius entry and keeps it `NOT_AVAILABLE`;
- source contains no `plugins/mobius/bin/mobius`, Python runtime, or shell launcher path.

## Agent Path Optimization Gate

Release A-D0 must pass on the exact source candidate:

```bash
cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked \
  --test evidence_bundle_contract
cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked \
  --test proof_impact_contract
cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked \
  --test composition_skill_contract
cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked \
  --test judge_consumption_contract
cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked \
  --test session_audit_contract
cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked \
  --test mcp_protocol pre_record_material_drift_keeps_the_candidate_outside_trail -- --exact
cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked \
  --test mcp_protocol \
  record_and_decision_freshness_gates_preserve_history_and_block_stale_accept -- --exact
```

The gate must establish:

- Evidence Bundle v1 canonicalization, all four material kinds, full machine identities, the fixed
  byte budget, coherence, current applicability, and deterministic repository capture;
- Record/Seal/Decision drift behavior through real stdio MCP, including zero submission for an
  incoherent candidate, complete mixed-baseline Packet materialization, and a non-accept Decision
  when all mutable supports Evidence is superseded;
- proof-impact dispositions use existing Remap/carry/reverification and never reopen a terminal
  Objective or edit old Evidence, Decision, or Trail;
- Stage Risk Cards select Driver and Verifier by information value and Evidence coverage; every
  Stage Review creates one fresh required Judge after closure/freeze, while extra Judges remain
  information-value driven;
- Judge native final-output, envelope, result-budget, overflow, freeze, coverage, degraded,
  external-profile, and advice-only cases fail closed; absence or an unusable result blocks
  `accept` without assuming a particular review skill, provider, model, or private parser;
- canonical transition drafts match live MCP schema, all ten C0 faults close the fence, and C1
  stays `not_evaluated` until its real-session/draft corpus and ADR gate exist; and
- the D0 synthetic audit fixture and redaction/authority contract pass while D1 remains
  `not_evaluated` without a supported host export adapter.

Full SHA-256 remains present at every machine equality, freeze, artifact, Core, and Trail boundary.
Repeated Agent-facing material uses task-local ids and may show only `sha256:…<last 7 hex>` as a
display hint; a short suffix must never be used for lookup or admission.

## Target Bundle Gate

The only configured target is `x86_64-unknown-linux-gnu`:

```bash
target=x86_64-unknown-linux-gnu
version="$(jq -r '.version' plugins/mobius/.codex-plugin/plugin.json)"
bash .github/scripts/build-release-binary.sh "$target"
bash .github/scripts/assemble-release-bundle.sh \
  "$target" \
  "$CARGO_TARGET_DIR/$target/release/mobius" \
  "$PWD/.tmp/mobius-$version-$target"
bash tests/release_bundle_contract.sh bundle \
  "$PWD/.tmp/mobius-$version-$target"
# Required on a release host with the Codex CLI installed:
bash tests/release_bundle_contract.sh codex-install \
  "$PWD/.tmp/mobius-$version-$target"
```

The assembled root must contain:

```text
.agents/plugins/marketplace.json       # assembled copy only: AVAILABLE
LICENSE
SHA256SUMS
plugins/mobius/.codex-plugin/plugin.json
plugins/mobius/.mcp.json
plugins/mobius/bin/mobius              # the only executable
plugins/mobius/hooks/hooks.json
plugins/mobius/skills/...
```

The release helper requires the root-pinned Rust `1.85.0` toolchain plus a canonical host
`sqlite3 >= 3.40.1`, and produces one checksummed
x86-64 executable without Rust source, personal build-host paths, Python, or a system-SQLite runtime
dependency. Bundle validation exercises representative installed Hook and MCP wires; exhaustive
semantic matrices remain in Rust tests.

CI archives that directory, writes a separate archive checksum, extracts it into a fresh directory,
and runs `bundle-shape` so extraction, executable mode, layout, and internal checksums are rechecked
without repeating installed semantic smoke. The `codex-install` mode separately creates an isolated `HOME` and
`CODEX_HOME`, admits the assembled marketplace through the installed Codex CLI, installs the
plugin into the real Codex cache layout, and verifies the resolved cache cwd and relative command.
It then runs direct-work and delegated-candidate Core transport lanes under `env -i` and
`PATH=/nonexistent`. Both must use the four-tool MCP write path, observe exact Core-owned review
material through the canonical read-only SQLite command, reach `Achieved`, and finish with a
healthy read-only CLI audit. These lanes prove Core and translation boundaries; by themselves they
are not complete Mobius Composition paths because the shell smoke does not execute a native Stage
Judge. The installed delegated lane consumes one prevalidated successful observation; the
full result validator and stale, incomplete, unauthorized, cleanup-pending, and missing-boundary
matrix remain solely in the Rust native-host gate below. The uploaded target artifact is not publishable unless every
preceding job, the real-loader gate, and the independent requirement-by-requirement review
pass.

## Native Host Gate

An eligible v1 release host is Linux x86-64 with a stable Codex CLI version `>=0.143.0` and a
canonical absolute `sqlite3` version `>=3.40.1`. The
`codex-install` gate fails closed for an older version, a prerelease, or malformed version output.
This comparison is only the admission floor: every actual host version must pass the complete
installed-plugin, Hook, MCP, Core-lane, delegation-translation, and required-Stage-Judge gates
before release.
Bundle and extracted-archive smoke tests also require the MCP initialize version to equal the
installed plugin manifest version, preventing a stale runtime binary from passing under current
release metadata.

Before release, use the native Subagent workflow with the installed `mobius-subagent` skill and
record outcomes, not a copied Runtime ledger:

1. Freeze the skill, selected role profile, task baseline, and every supplied material.
2. Spawn a bounded Driver or Verifier without overriding model, provider, effort, sandbox, approval,
   or permission settings. Exercise native wait, same-envelope follow-up, completion, and interrupt.
3. Preserve spawn, configuration, Runtime, and permission failures exactly. Do not retry through a
   custom worker, alternate transport, elevation, or success-shaped fallback.
4. For the delegated-candidate translation lane, require both forbidden boundaries and the complete public
   result envelope. A malformed success-shaped result must be rejected before submit.
5. Supply the validated task/result/opaque native identity transiently through
   `MOBIUS_NATIVE_TASK_JSON`, `MOBIUS_NATIVE_RESULT_JSON`, and
   `MOBIUS_NATIVE_RUNTIME_IDENTITY`, then run:

   ```bash
   cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked \
     --test mcp_protocol \
     clean_stdio_mcp_delegated_candidate_translation_reaches_core_achieved \
     -- --exact
   ```

The test must pass through the real stdio MCP process to `Achieved` and healthy audit. Do not commit
the native task/result, agent identity, thread items, usage, or a worker registry. Any new host or
changed experimental MCP metadata shape requires this gate again.

The test above is the delegated-candidate translation lane. Before a Phase 4 or release claim,
also run both installed Loop paths through the host Agent: one with main-direct Stage work and one
with optional Driver/Verifier work. In each path, after the live Packet closure and material freeze,
spawn one fresh generic `mobius-subagent` Judge, validate its native final result and complete
freeze/coverage, have main address every finding, and only then submit `Decision(accept)`. Absence,
unavailable/degraded execution, stale material, partial coverage, inconclusive advice, or unresolved
findings must stop `accept`. Record only bounded release evidence, not a Runtime mirror.

## Phase Preconditions

- Phase 1 proves all eleven object mappings, Map constraints, transitions, `I1..I19`, deterministic
  replay, strict persistent codec, and one binary target.
- Phase 2 proves project binding, exactly one SQLite database per project, transactions,
  idempotency, artifacts, Packet materialization, recovery, MCP, reports, and crash behavior.
- Phase 3 proves all thirteen independent Subagent acceptance conditions and native host lifecycle
  failures without Core knowledge or a Runtime mirror.
- Phase 4 proves Composition, one fresh required Judge per Stage Review, typed human confirmation,
  narrow hooks, forbidden delegation
  boundaries, Evidence Bundle freshness/proof impact, risk-adaptive delegation, C0 controls, D0
  audit contract, both Judge-gated end-to-end paths, Core transport lanes, clean-host execution,
  and every P0 release gate. D1 is
  conditional and currently `not_evaluated`.

## Final Review

- Inspect the full diff and archive for secrets, personal paths, generated state, v0.5 runtime
  remnants, Python, launchers, downloaders, a second executable, or a second state path.
- Confirm the source marketplace is still `NOT_AVAILABLE` and only the assembled copy is
  `AVAILABLE`.
- Confirm the archive version matches Cargo, manifest, changelog, tag, and release notes.
- Confirm `plugins/mobius/.codex-plugin/plugin.json`, `.mcp.json`, and `hooks/hooks.json` still
  resolve to the same installed `bin/mobius`.
- Run an independent review against every blueprint requirement and recorded command result.
- Confirm the three authoritative `dev/` blueprints contain no unresolved P0 decision for the
  release scope, and state accepted residual performance or cooperative-threat-model risk in the
  release review and notes.
