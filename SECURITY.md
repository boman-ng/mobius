# Security Policy

Mobius is a local Codex plugin. It writes project-private execution state, runs a local stdio MCP
server, and installs lifecycle hooks that protect Mobius ledger files.

## Supported Versions

Security fixes are accepted for the latest tagged release and the default branch.

## Reporting A Vulnerability

Use private GitHub vulnerability reporting if it is enabled for the repository. If private reporting
is not available, contact the maintainers privately before opening a public issue. Do not include
secrets, private project data, or local Mobius ledger contents in a public report.

## Security Boundaries

- Mobius has no hosted service and does not require a Mobius account.
- The bundled MCP server runs locally through Codex MCP configuration.
- Kimi is only invoked for review policies that require it.
- Plugin hooks require Codex hook trust review before execution.
- Local Mobius execution state should not be committed.

## Maintainer Response

Maintainers should acknowledge reports, reproduce the issue, classify severity, prepare a patch,
and publish a release note when a fix ships.
