# Official Docs Basis

Mobius packaging follows the Codex plugin model described in official OpenAI Codex docs.

## Sources Consulted

- Codex Build plugins: https://developers.openai.com/codex/plugins/build
- Codex Plugins overview: https://developers.openai.com/codex/plugins
- Codex Agent Skills: https://developers.openai.com/codex/skills
- Codex MCP: https://developers.openai.com/codex/mcp
- Codex Hooks: https://developers.openai.com/codex/hooks
- Codex AGENTS.md guidance: https://developers.openai.com/codex/guides/agents-md
- Codex Open Source: https://developers.openai.com/codex/open-source

## Decisions

- Package reusable workflows as skills and distribute them through a plugin.
- Use a repository marketplace at `.agents/plugins/marketplace.json`.
- Store the plugin at `plugins/mobius` with `.codex-plugin/plugin.json` as the required manifest.
- Keep manifest component paths relative to the plugin root and prefixed with `./`.
- Bundle the Mobius Review stdio MCP server through `mcpServers`.
- Bundle lifecycle hooks under `hooks/hooks.json`; users still review and trust hooks in Codex.
- Keep repository development guidance, local test ledgers, and `AGENTS.md` outside the installed
  plugin runtime contract.
- Avoid claiming official public Plugin Directory publication. The official docs state self-serve
  public plugin publishing is coming soon, so v0.5.0 targets GitHub repository marketplace
  distribution.

## Runtime Shape

Mobius uses one bundled MCP shape: `plugin.json` points `mcpServers` at
`plugins/mobius/.mcp.json`, and that file defines the `mobius-review` stdio server.
