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
It then runs complete direct and delegated loops under `env -i` and `PATH=/nonexistent`. Both must
use the four-tool MCP write path, observe exact Core-owned review material through the canonical
read-only SQLite command, reach `Achieved`, and finish with a healthy read-only CLI audit. The
installed delegated lane consumes one prevalidated successful observation; the
full result validator and stale, incomplete, unauthorized, cleanup-pending, and missing-boundary
matrix remain solely in the Rust native-host gate below. The uploaded target artifact is not publishable unless every
preceding job, the real-loader gate, and the independent requirement-by-requirement cross-review
pass.

## Native Host Gate

An eligible v1 release host is Linux x86-64 with a stable Codex CLI version `>=0.143.0` and a
canonical absolute `sqlite3` version `>=3.40.1`. The
`codex-install` gate fails closed for an older version, a prerelease, or malformed version output.
This comparison is only the admission floor: every actual host version must pass the complete
installed-plugin, Hook, MCP, direct-loop, and delegated-loop gate before release.
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
4. For the delegated Composition E2E, require both forbidden boundaries and the complete public
   result envelope. A malformed success-shaped result must be rejected before submit.
5. Supply the validated task/result/opaque native identity transiently through
   `MOBIUS_NATIVE_TASK_JSON`, `MOBIUS_NATIVE_RESULT_JSON`, and
   `MOBIUS_NATIVE_RUNTIME_IDENTITY`, then run:

   ```bash
   cargo test --manifest-path plugins/mobius/runtime/Cargo.toml --locked \
     --test mcp_protocol \
     clean_stdio_mcp_delegated_composition_gates_full_result_then_main_reaches_achieved \
     -- --exact
   ```

The test must pass through the real stdio MCP process to `Achieved` and healthy audit. Do not commit
the native task/result, agent identity, thread items, usage, or a worker registry. Any new host or
changed experimental MCP metadata shape requires this gate again.

## Phase Preconditions

- Phase 1 proves all eleven object mappings, Map constraints, transitions, `I1..I19`, deterministic
  replay, strict persistent codec, and one binary target.
- Phase 2 proves project binding, exactly one SQLite database per project, transactions,
  idempotency, artifacts, Packet materialization, recovery, MCP, reports, and crash behavior.
- Phase 3 proves all thirteen independent Subagent acceptance conditions and native host lifecycle
  failures without Core knowledge or a Runtime mirror.
- Phase 4 proves Composition, typed human confirmation, narrow hooks, forbidden delegation
  boundaries, both end-to-end paths, clean-host execution, and every P0 release gate.

## Final Review

- Inspect the full diff and archive for secrets, personal paths, generated state, v0.5 runtime
  remnants, Python, launchers, downloaders, a second executable, or a second state path.
- Confirm the source marketplace is still `NOT_AVAILABLE` and only the assembled copy is
  `AVAILABLE`.
- Confirm the archive version matches Cargo, manifest, changelog, tag, and release notes.
- Confirm `plugins/mobius/.codex-plugin/plugin.json`, `.mcp.json`, and `hooks/hooks.json` still
  resolve to the same installed `bin/mobius`.
- Run an independent cross-review against every blueprint requirement and recorded command result.
- Confirm the three authoritative `dev/` blueprints contain no unresolved P0 decision for the
  release scope, and state accepted residual performance or cooperative-threat-model risk in the
  release review and notes.
