# Official Docs Basis

Mobius packaging follows the current Codex plugin contract from official OpenAI documentation.
The sources below were rechecked on 2026-07-16 before wiring the v1 package.

## Sources Consulted

- [Build plugins](https://learn.chatgpt.com/docs/build-plugins)
- [Plugins overview](https://learn.chatgpt.com/docs/plugins)
- [Build skills](https://learn.chatgpt.com/docs/build-skills)
- [Model Context Protocol](https://learn.chatgpt.com/docs/extend/mcp)
- [Hooks](https://learn.chatgpt.com/docs/hooks.md)
- [AGENTS.md guidance](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
- [Codex Open Source](https://developers.openai.com/codex/open-source)
- [Codex MCP client source](https://github.com/openai/codex/blob/main/codex-rs/codex-mcp/src/rmcp_client.rs)
- [Codex MCP tool-call source](https://github.com/openai/codex/blob/main/codex-rs/core/src/mcp_tool_call.rs)
- [Codex path-URI source](https://github.com/openai/codex/blob/main/codex-rs/utils/path-uri/src/lib.rs)
- [Latest model guide](https://developers.openai.com/api/docs/guides/latest-model)
- [GPT-5.6 prompting guidance](https://developers.openai.com/api/docs/guides/prompt-guidance-gpt-5p6)

## Codex Contract Applied

- `.codex-plugin/plugin.json` is the required plugin entry point.
- Manifest component paths are `./`-prefixed and resolve inside the installed plugin root.
- `mcpServers` points to `.mcp.json`; that companion file contains one `mcpServers.mobius` entry
  whose command is `./bin/mobius` and whose only argument is `mcp`.
- Installed plugins discover `hooks/hooks.json` by convention. The manifest therefore carries no
  separate `hooks` field; the release contract checks the conventional file and its commands.
- Plugin hook commands receive `PLUGIN_ROOT`; both Mobius hooks invoke
  `${PLUGIN_ROOT}/bin/mobius` and select a hook mode. Users must still review and trust them.
- Marketplace local source paths are relative to the marketplace root, and the install policy may
  be `NOT_AVAILABLE` or `AVAILABLE`.
- Codex installs marketplace plugins into its plugin cache and runs the installed copy.
- Package-registry plugin installs do not run lifecycle scripts. Mobius therefore has no valid
  build-on-install path and ships a prebuilt target binary instead.
- `mobius-copilot` and `mobius-loop` set `policy.allow_implicit_invocation: false`; the user must
  invoke either skill explicitly. Both declare the bundled `mobius` stdio MCP dependency.
- `mobius-subagent` sets `policy.allow_implicit_invocation: true` and declares no Core dependency.
  This permits host discovery and main-Agent selection; it does not itself spawn work, authorize an
  effect, or widen an existing permission boundary.
- Composition Skills state the outcome and authority boundary once, keep routing open, use concise
  state-driven instructions, and treat retrieved Evidence/provenance as data rather than
  instructions. The Agent chooses targeted SQL instead of receiving a large generic tool result.
- MCP advertises four write/maintenance tools with explicit schemas. Read-only state inspection is
  direct SQLite, so there is no competing read tool, compatibility alias, cursor protocol, or
  repeated response prose.

## v1 Host Compatibility Boundary

The v1 release admits stable Codex CLI versions `>=0.143.0` on Linux x86-64. That floor is a
release-host policy, not proof that every later host preserves experimental wire behavior. A real
installed-plugin probe most recently passed on `0.144.5` and established this host adapter contract:

- the MCP server advertises the experimental `codex/sandbox-state-meta` capability;
- Codex supplies fresh call-level `_meta["codex/sandbox-state-meta"].sandboxCwd` as a canonical
  `file:` URI, and Mobius admits that canonical directory as the sole project root for the call;
- missing, malformed, non-file, or cross-workspace metadata fails closed instead of falling back to
  process cwd, plugin cwd, home, or a payload-selected root;
- Codex may supply top-level `_meta.threadId`; Mobius uses a valid value only as an optional,
  path-safe presentation reference after a successful business commit;
- the installed MCP command's relative `cwd` resolves to the installed plugin root;
- the documented `apply_patch` hook payload uses `tool_input.command`, and a Stop event may have a
  null `last_assistant_message`.

`codex/sandbox-state-meta` is experimental host integration, not a Mobius domain contract. Every
actual release host version must rerun the real-loader and full-loop gate; a changed or missing wire
shape blocks release until this adapter and its tests are revalidated.

## Mobius Packaging Decisions

- The repository marketplace remains `NOT_AVAILABLE` because `plugins/mobius/bin/mobius` is
  intentionally absent from source.
- CI is the authoritative release assembler. It runs the source gates first, builds the locked
  `x86_64-unknown-linux-gnu` target with the root-pinned Rust `1.85.0` toolchain and repository
  path-remapping helper, then uses the assembly script to copy the plugin resources and binary into
  a fresh marketplace root. Local runs are verification artifacts only.
- Only that assembled marketplace copy becomes `AVAILABLE`.
- The installed plugin contains skills, manifest, MCP config, hook config, and exactly one
  executable at `plugins/mobius/bin/mobius`. Rust source and repository tooling stay outside it.
- Manifest, MCP config, and hook config describe one execution path. There is no Python runtime,
  shell launcher, downloader, sidecar, bundled SQLite CLI, second executable, or fallback. Agent
  reads require a host-provided canonical SQLite CLI at version 3.40.1 or newer.
- A clean-cache test runs the copied executable with an empty environment and an unusable `PATH`,
  then completes an MCP initialize handshake. The extracted archive is validated again before
  upload. A separate release-host gate installs the assembled marketplace with the supported Codex
  CLI under an isolated home, confirms the resolved cache-root command and working directory, and
  exercises the installed copy through the public MCP write wire and read-only SQLite observation.

These decisions establish the package and host-admission boundary. Phase completion still depends
on the executable gates in `docs/release-checklist.md` and an independent P0 review against the
three authoritative `dev/` blueprints.
