# Changelog

All notable changes to Mobius are documented here.

## 0.4.0 - 2026-07-08

- Add a derived Review Contract View to `packet-read` and MobiusCV prompts so reviewers share a
  consistent, non-authoritative review boundary without adding another source of truth.
- Harden final evidence gates with explicit final-scoped evidence, currentness diagnostics,
  change-set coverage checks, and refresh templates before strict exit review.
- Separate delta and exit review policy behavior, including high-risk delta Kimi escalation and
  explicit reopen semantics for passed stages.
- Add hook and doctor diagnostics for installed-plugin health, protected ledger reads, and
  restricted read pipelines.
- Expand regression and release-bundle coverage for the revised loop, evidence, hook, MobiusCV,
  and release contracts.

## 0.3.0 - 2026-07-07

- Add agentic Mobius plan and loop guidance grounded in first-principles framing, stage control
  checks, blind-spot inspection, disconfirmation, and pruning discipline.
- Strengthen MobiusCV reviewer prompts around evidence quality, assumptions, counterevidence,
  Goodhart risk, variety coverage, contract drift, and fail-closed completion.
- Replace the legacy shell verification wrapper with pytest-based release-bundle tests and a direct
  GitHub Actions CI workflow for syntax, pytest, hook-health, and Git hygiene checks.
- Document pytest-first verification and Conventional Commit expectations for repository work.

## 0.2.0 - 2026-07-07

- Harden MobiusCV review recording with reviewer workspace preflight, canonical packet loading,
  retryable reviewer infrastructure failures, and explicit subagent lifecycle guidance.
- Add loop diagnostics for machine-usable next actions and human-readable status explanation.
- Add structured evidence ergonomics, validity scopes, compact replay metadata, and raw review
  retention rules for pass and non-pass reviews.
- Expand regression coverage for the Soma-session failure categories and include it in the local
  release gate.
- Route repairable exit `blocked` reviews back to final evidence refresh instead of terminal
  goal blockage, while preserving true terminal blocked verdicts.
- Add deterministic final-evidence freshness checks for exit packets and compact diagnostics for
  generated Python artifacts before expensive external review.
- Add structured review-attempt diagnostics for failure kind, retryability, diagnostic refs, and
  retry counts.

## 0.1.0 - 2026-07-06

- Package Mobius as a repo-distributed Codex plugin.
- Add repo marketplace metadata under `.agents/plugins/marketplace.json`.
- Ship plugin source under `plugins/mobius`.
- Publish Apache-2.0 license and open-source contributor/security documentation.
- Add release-grade manifest metadata and local MCP/hook runtime documentation.
- Add GitHub Actions verification and a local release gate script.
- Add a v0.1.0 release checklist with tag-pinned install and refresh commands.
