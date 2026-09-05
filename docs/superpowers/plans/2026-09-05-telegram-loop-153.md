# Telegram and Loop Forward-port Implementation Plan

**Goal:** Preserve the confirmed 0.147 Telegram integration on upstream 0.153.4 and restore the independent local loop.

**Architecture:** Keep channel transport/policy in rmcp-client and codex-mcp, display/submission in TUI, and transport-neutral events in protocol. Restore loop orchestration at the TUI boundary with scoped model control. Synchronize marketplace bridge copies.

**Tech Stack:** Rust, Tokio, RMCP, Node.js, just/nextest.

**Spec:** `docs/superpowers/specs/2026-09-05-telegram-loop-153-design.md`

## Global Constraints

- Base rust-v0.153.4; reference origin/channels-telegram-v0.147.0-local.
- Preserve native goals; loop remains independent.
- No secrets, live Telegram sends or replacement of installed Codex. Deliver Linux artifacts from a separate feature branch; public release publication is a separate explicit action.
- Use repository test/schema/format recipes; report any verification limitation accurately.

## Task 1: Forward-port channel runtime

- [x] Create feature branch from stable tag after fetching upstream.
- [x] Read current AGENTS.md and inspect channel patch against new source.
- [x] Restore existing regression coverage from 0.147 and attempt focused tests before adapting runtime.
- [x] Apply the net channel changes from the confirmed 0.147 branch, resolve transport and config changes against current APIs, preserving upstream edits.
- [x] Run just test -p codex-config -p codex-rmcp-client -p codex-mcp and protocol/TUI coverage.

## Task 2: Synchronize Telegram marketplace

- [x] Compare standalone bridge with 0.147 source, port direct notifications and polling/shutdown fixes.
- [x] Test node --check, --self-test, and stdio MCP startup without secrets/network calls.
- [x] Synchronize plugin code/manifest/docs and marketplace entries across repos.

## Task 3: Restore independent loops

- [x] Inspect the loop implementation preceding fca3d366c3 and restore behavior tests first.
- [x] Port /loop commands, timer generation, input priority and root loop_control; adapt current tool registration and app-server transport.
- [x] Cover max iterations, invalid options, stop/restart stale timers, queued user input, submission failure, interrupted turns, native goal interaction and per-thread status/control.
- [x] Run focused TUI/core/protocol tests and review snapshots.

## Task 4: Integration and delivery

- [x] Regenerate config/app-server schemas, including experimental and precomputed exports as required by current recipes.
- [x] Build CLI; smoke-test --version and loop help/command surface where feasible.
- [x] Run scoped just fix and just fmt after tests; inspect final diff, schema changes and both plugin copies.
- [x] Independent code review; resolve material findings with covering tests.
- [x] Record exact tested commands/results and remaining limitations; leave local branches ready for review.

## Linux artifact delivery

The feature branch push runs `.github/workflows/channels-release.yml`. Its build
assembles the canonical musl package, checks resources, installs into a temporary
prefix, runs `codex-channels --version`, and uploads the archive with its checksum.
A public release is only published on an explicitly requested manual dispatch.

Local verification included 287 config tests; 510 MCP/RMCP tests with the three
remote-process cases rerun successfully after CLI compilation; the 5072-test
TUI/protocol/app-server/core selection with every failure resolved and rerun;
the added loop-registration policy test; and all three loop-control integration
cases. Snapshot updates reflect the release version and restored command. Color
tests require NO_COLOR unset; locale-dependent numeric expectations were made
portable. Scoped Clippy and repository formatting completed successfully.
