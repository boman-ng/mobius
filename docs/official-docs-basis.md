# Official Docs Basis

Mobius packaging follows the current Codex plugin contract from official OpenAI documentation.
The sources below were rechecked on 2026-07-16 before wiring the v1 package.

## Sources Consulted

- [Build plugins](https://learn.chatgpt.com/docs/build-plugins)
- [Plugins overview](https://learn.chatgpt.com/docs/plugins)
- [Build skills](https://learn.chatgpt.com/docs/build-skills)
- [Subagents and custom agents](https://learn.chatgpt.com/docs/agent-configuration/subagents)
- [Model Context Protocol](https://learn.chatgpt.com/docs/extend/mcp)
- [Hooks](https://learn.chatgpt.com/docs/hooks.md)
- [AGENTS.md guidance](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
- [Codex Open Source](https://developers.openai.com/codex/open-source)
- [Codex MCP client source](https://github.com/openai/codex/blob/main/codex-rs/codex-mcp/src/rmcp_client.rs)
- [Codex MCP tool-call source](https://github.com/openai/codex/blob/main/codex-rs/core/src/mcp_tool_call.rs)
- [Codex path-URI source](https://github.com/openai/codex/blob/main/codex-rs/utils/path-uri/src/lib.rs)

## Codex Contract Applied

- `.codex-plugin/plugin.json` is the required plugin entry point.
- Manifest component paths are `./`-prefixed and resolve inside the installed plugin root.
- `mcpServers` may point to `.mcp.json`; that file may contain a direct server map. Mobius uses one
  direct `mobius` entry whose command is `./bin/mobius` and whose only argument is `mcp`.
- Installed plugins may load `hooks/hooks.json` by default or through the manifest `hooks` field.
  Mobius uses the explicit relative field so the release contract is mechanically visible.
- Plugin hook commands receive `PLUGIN_ROOT` and the writable `PLUGIN_DATA`; all three Mobius hooks
  invoke `${PLUGIN_ROOT}/bin/mobius` and select a hook mode. Users must still review and trust them.
- A plugin `SessionStart` hook may add developer context, and exit `0` with no output is a silent
  success. Mobius matches only `startup`, delivers its bounded Judge onboarding context once, stores
  only an atomic one-shot claim in `PLUGIN_DATA`, and emits no output in losing concurrent or later
  Sessions.
- Codex custom agents are standalone user- or project-level TOML files. The user-level
  `mobius-judge` file created by the one-time Agentic onboarding uses the documented custom-agent
  configuration surface; Codex loads it for a later spawned Session.
- Marketplace local source paths are relative to the marketplace root, and the install policy may
  be `NOT_AVAILABLE` or `AVAILABLE`.
- Codex installs marketplace plugins into its plugin cache and runs the installed copy.
- Package-registry plugin installs do not run lifecycle scripts. Mobius therefore has no valid
  install-time execution point: the first trusted `SessionStart(startup)` is the earliest supported
  onboarding boundary, and the plugin still ships a prebuilt target binary.
- `mobius-copilot` and `mobius-loop` set `policy.allow_implicit_invocation: false`; the user must
  invoke either skill explicitly. Both declare the bundled `mobius` stdio MCP dependency.
- `mobius-subagent` sets `policy.allow_implicit_invocation: true` and declares no Core dependency.
  This permits host discovery and main-Agent selection; it does not itself spawn work, authorize an
  effect, or widen an existing permission boundary.

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
- The SessionStart adapter does not implement provider discovery, HTTP, TOML editing, credentials,
  or model selection in Rust. It delegates that one-time configuration task to the main Agent and
  then leaves later Session startup paths silent.
- Manifest, MCP config, and hook config describe one execution path. There is no Python runtime,
  shell launcher, downloader, sidecar, SQLite CLI dependency, second executable, or fallback.
- A clean-cache test runs the copied executable with an empty environment and an unusable `PATH`,
  then completes an MCP initialize handshake. The extracted archive is validated again before
  upload. A separate release-host gate installs the assembled marketplace with the supported Codex
  CLI under an isolated home, confirms the resolved cache-root command and working directory, and
  exercises the installed copy through the public MCP wire.

These decisions establish the package and host-admission boundary. Phase completion still depends
on the phase evidence and independent P0 review recorded in `dev/v1-implementation-status.md`.
