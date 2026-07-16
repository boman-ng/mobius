# Security Policy

Mobius v1 is designed as a local Codex plugin. The target runtime stores project-private state in
one SQLite database under the canonical project root, exposes a local stdio MCP adapter, and uses
narrow lifecycle hooks to protect Core-owned state and completion claims.

## Supported Versions

Security fixes are accepted for v1.1.0 and the default branch. The checked-in source marketplace
remains unavailable because it does not contain the release executable; use the checksummed GitHub
release bundle.

## Reporting A Vulnerability

Use private GitHub vulnerability reporting if it is enabled for the repository. If private
reporting is unavailable, contact the maintainers privately before opening a public issue. Do not
include secrets, private project data, or local Mobius state in a public report.

## Security Boundaries

- Mobius has no hosted service, telemetry, account, or network dependency for its core workflow.
- Project binding, path containment, symlink rejection, typed guards, and atomic Trail append are
  programmatic boundaries.
- Main Agent ownership and Subagent forbidden boundaries are cooperative instruction contracts;
  caller attestation and hostile delegated agents are outside the stated threat model.
- Plugin hooks require Codex hook trust review before execution.
- The pre-tool hook is a cooperative lexical guard. It protects explicit Core-owned state and
  destructive operations whose resolved scope contains a bound Mobius project, while derived
  `views/` and ordinary unbound work remain outside Core authority. Ambiguous recognized mutation
  forms fail closed; the exhaustive supported grammar and path rules are owned by
  `dev/Mobius-implement.md` and executable Hook tests.
- Alternate-database discovery treats a candidate that disappears between enumeration and open as
  absent; every other candidate-open or read error remains fail closed.
- Shell-shaped input is analyzed only within the documented literal-command model. Dynamic shell
  state, alternate interpreters, archive contents, namespace changes, and concurrent path swaps are
  outside that model; the hook is not a hostile-process sandbox.
- Release bundles contain exactly one executable, `plugins/mobius/bin/mobius`; MCP and hook config
  invoke that same installed file. Build scripts and tests remain outside the plugin bundle.
- The supported Linux bundle uses the crate's bundled SQLite library and is exercised from an
  isolated real Codex cache through complete MCP loops with no usable command search path. It has
  no Python, SQLite CLI, downloader, or helper-process requirement.
- `SHA256SUMS` binds every assembled file, and CI revalidates the extracted archive before upload.
- Project-local `.mobius/` state can contain durable evidence and must not be committed or treated
  as disposable merely because it is private to Mobius.
- Only an exact independently reviewed, checksummed assembled artifact may be published as a
  Mobius release.

## Maintainer Response

Maintainers should acknowledge reports, reproduce the issue, classify severity, prepare a patch,
and publish a release note when a fix ships.
