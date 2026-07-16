# Changelog

All notable changes to Mobius are documented here.

## 1.1.0 - 2026-07-16

- Add one-time, trusted-startup onboarding for a provider-backed `mobius-judge` custom agent while
  keeping one Judge role contract and leaving later Sessions silent.
- Pin the native Scout, Researcher, Verifier, and default Judge model policies, while Driver
  inherits the Main Agent configuration and a different model family changes only Runtime spawn
  configuration.

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
