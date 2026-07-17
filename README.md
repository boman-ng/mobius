# Mobius

Mobius v1 is a local-only, strictly serial, auditable route-finding system driven by a main Agent.
An Objective advances through Stage, Route, Attempt, Evidence, and Review until every current Stage
has an accepted proof, or a human abandons the Objective.

Mobius v1.1.1 is the current stable single-binary release. The checked-in marketplace deliberately
marks the source plugin `NOT_AVAILABLE` because source does not contain a release binary and has no
install-time build or download path. The GitHub release publishes the gated Linux x86-64 assembled
marketplace and its checksum.

## Architecture

The design has three owners with one-way composition:

```text
            Main Agent Composition
                 /             \
                v               v
          Model Core       Subagent Skill
```

- Model Core owns typed objects, guards, the pure reducer, Trail, Evidence admission, artifacts,
  persistence, and the only mutation service.
- The optional Subagent Skill owns generic delegation roles and result/effect envelopes. It knows
  nothing about Core objects, APIs, paths, or persistence.
- Main Agent Composition interprets open-world work, translates observations into typed Evidence,
  forms formal Judgments, and submits guarded commands to Core.

Core persistence is one project-local append-only SQLite Trail plus transactionally maintained,
rebuildable projections. The main Agent reads that database directly with a canonical SQLite
3.40.1-or-newer CLI in safe, read-only, query-only mode and selects only the rows needed for its
current decision. MCP has one purpose: validate initialization, artifact capture, typed transitions,
and explicit maintenance. This keeps audit facts available without imposing a second read protocol
or selecting the Agent's Route or work method.

The authoritative blueprints are:

- `dev/mobius-model.md`
- `dev/Mobius-subagent.md`
- `dev/Mobius-implement.md`

## Current Release

The active tree contains one Cargo package with one `mobius` binary target, the
`mobius-copilot` and `mobius-loop` Model skills, the independent Subagent skill, project-bound
SQLite and artifact stores, the Core service, public stdio MCP, read-only operational CLI modes,
context-dark reports, and narrow hooks. Direct and delegated Composition loops are tested through
the real MCP process with read-only SQLite observation; the delegated lane keeps worker output
candidate-only and lets only main construct typed Core input. Detailed evidence and the
supported-host boundary are recorded in
`dev/v1-implementation-status.md`.

`mobius-copilot` exclusively manages human-authorized Objective activation, revision,
abandonment, and the initial or specification-revision Map those actions require. It resumes an
interrupted durable Mapping state through that same installation path. `mobius-loop` executes an
already active Objective, including operational remap and wait-drift Map installation; it hands
contract changes back to Copilot instead of creating a second owner. Host policy requires users to
invoke both Composition skills explicitly. The independent `mobius-subagent` remains discoverable
so the main Agent may select bounded delegation while running an explicit Loop; discovery never
expands the task's permission or effect boundaries.

The v0.5 Python/CSV implementation is no longer present in the active plugin tree. Its durable
source remains tag `v0.5.0`; a checksummed local inspection copy is stored under `.tmp/`, which is
intentionally ignored and is not a release or compatibility path.

## Install v1.1.1

Download both release assets, verify the checksum, extract the marketplace, then install it through
Codex:

```bash
sha256sum --check mobius-1.1.1-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf mobius-1.1.1-x86_64-unknown-linux-gnu.tar.gz
codex plugin marketplace add ./mobius-1.1.1-x86_64-unknown-linux-gnu
codex plugin add mobius@mobius
```

Start a new Codex thread after installation so the v1 Skills, MCP server, and Hooks load from the
installed plugin cache. Review and trust the packaged Hooks before using Mobius on a project.
The host must provide a canonical absolute `sqlite3` executable at version 3.40.1 or newer; Mobius
does not bundle or download one.

## Release Artifact Contract

The first and only configured target is `x86_64-unknown-linux-gnu`. Its assembled marketplace root
contains the plugin at `plugins/mobius/` and exactly one executable at
`plugins/mobius/bin/mobius`. The manifest selects the MCP and hook configs, and both configs invoke
that same executable through paths relative to the installed plugin root.
The installed bundle excludes Rust source, development tests, Python, a SQLite CLI, launchers,
downloaders, and helper executables. The host SQLite prerequisite is deliberately external.

CI copies the assembled plugin into an isolated Codex-style cache and starts both `--help` and the
stdio MCP initialize handshake with an empty environment. The release-host gate admits stable
Codex CLI versions `>=0.143.0`, verifies the external SQLite prerequisite, installs the marketplace
through the actual host, verifies the resolved cache command and cwd, and runs complete direct and
delegated MCP-write/SQLite-read loops to `Achieved`. The minimum version is an admission floor;
every actual release host must pass the full gate. The checked-in marketplace stays unavailable;
only the assembled copy is marked `AVAILABLE`.

## Verification

Use `AGENTS.md` for contributor rules and the canonical development gate, and
`docs/release-checklist.md` for release-only build, installation, and host checks.

## License

Apache-2.0. See `LICENSE`.
