# Telegram Channel Forward-Port to Codex 0.147.0

## Objective

Forward-port the final, observable Telegram Channel behavior from
`netsky-lab/codex` branch `channels-telegram-v0.142.5` onto the official
`rust-v0.147.0` release. The first deliverable is a separate local binary named
`codex-telegram` that can be run alongside the installed Codex 0.147.0 binary.
No remote branch is pushed and the installed `codex` symlink is not replaced.

## Behavioral Compatibility

The port preserves the behavior of the final 0.142.5 channel branch:

- A channel-capable MCP server is disabled by default and must be explicitly
  enabled with `channel.enabled = true`.
- An accepted Telegram message is delivered to the active TUI session and
  immediately submitted as a new user turn.
- The model receives a structured JSON envelope containing message identity,
  routing metadata, attachments, source, sender, and text.
- The TUI history shows a concise human-readable message instead of the raw JSON
  envelope.
- Shell escape parsing is disabled for channel-originated messages.
- Downloaded image attachments become local image inputs; other attachments are
  listed in the displayed and structured payloads.
- Duplicate IDs, excess bursts, and messages above the configured per-minute
  rate are dropped.
- Telegram tools for reply, typing, reaction, status, and file upload remain
  available through the plugin.

The configuration modes `ask`, `queue`, `immediate`, and `context` remain part
of the advertised channel capability, but retain the final branch's actual
behavior: they do not alter TUI delivery. There is no `/channels` command or
local pause/mute queue in this port.

## Source and Workspace

The working copy is `/home/netsky/codex-telegram-v0.147.0`, with local branch
`channels-telegram-v0.147.0-local` based on official tag `rust-v0.147.0`.
The functional reference is the net change made by the 34 Netsky channel
commits after release commit `26de83050b20f7e0ee211b9739e52ae00ce8032a`,
ending at `fca3d366c359c00d75a8ef81180fb2dd032d0a0b`.

The Telegram plugin comes from the final feature branch, not from the standalone
marketplace repository. The standalone repository currently carries a
logging-notification variant, while the final feature branch uses direct MCP
custom notifications and is the behavior being preserved.

## Architecture

The delivery path is:

1. `telegram-channel.mjs` long-polls the Telegram Bot API, enforces Telegram
   chat/topic allowlists, downloads accepted attachments, and emits a custom MCP
   notification.
2. `codex-rmcp-client` exposes an incoming custom-notification callback from its
   client service while continuing to delegate ordinary MCP logging and
   elicitation behavior to existing handlers.
3. `codex-mcp` parses channel notification methods and payloads, applies the
   configured enablement policy, duplicate suppression, burst capacity, and
   per-minute rate limit, then emits `EventMsg::ChannelMessage`.
4. `codex-app-server` forwards that event as a global `channel/message`
   notification. Existing TUI app-scoped routing delivers it to the active
   session.
5. `codex-tui` formats separate model and display representations and submits
   the message using the normal user-message path with shell escapes disabled.

## Component Boundaries

### Configuration

`codex-config` owns `McpServerChannelConfig` and `McpServerChannelMode`. The
channel policy is nested under each MCP server, defaults to disabled, and is
included in generated configuration schema output. Existing constructors and
test fixtures receive a default channel policy without changing their behavior.

### Protocol

`codex-protocol` owns transport-neutral `ChannelMessageEvent` and
`ChannelMessageAttachment` types plus the `EventMsg` variant. The app-server
protocol wraps the core event without duplicating its fields.

### RMCP Transport Hook

`codex-rmcp-client` owns only the generic callback needed to surface a custom
server notification. It does not know about Telegram or channel policy. The
callback is preserved in initialization context so reconnects retain identical
notification handling.

### Channel Adapter

Channel-specific parsing, capability advertisement, state, and unit tests live
in a focused module under `codex-mcp`, rather than adding roughly 500 lines to
the already large `rmcp_client.rs`. The module accepts server name, policy, raw
method/params, and an event sender. It produces no event for unrelated custom
notifications or invalid/empty payloads.

### App Server and TUI

The app server forwards accepted events without applying Telegram-specific
logic. TUI formatting and submission live in a focused channel module adjacent
to input submission. This keeps the active-session behavior at the presentation
boundary and avoids adding channel policy to `codex-core`.

### Telegram Plugin

The plugin is copied as a self-contained plugin directory with manifest, MCP
configuration, documentation, and the version 0.5.1 Node.js bridge. Secrets and
allowlists remain environment variables and are never written into the source
tree.

## Input and Security Rules

- `channel.enabled` is false by default for every MCP server.
- The plugin does not poll unless a chat/topic allowlist is configured, unless
  the explicit disposable-test override `TELEGRAM_ALLOW_ALL_CHATS=1` is set.
- Empty text and unsupported notification shapes are ignored.
- Dedupe state is bounded by `dedupe_capacity`.
- Burst and one-minute acceptance windows are bounded and held per MCP server.
- Incoming channel messages cannot trigger local TUI shell escape syntax.
- File downloads retain the plugin's maximum-size check and configured download
  directory.
- Bot tokens are accepted only from the runtime environment and never appear in
  docs, tests, commits, or command output.

## Error Handling

- Malformed or unrelated custom notifications are ignored without terminating
  the MCP session.
- Disabled, duplicate, queue-full, and rate-limited messages are dropped with a
  diagnostic warning.
- Failure to forward an accepted event is logged and does not crash the client.
- A poisoned delivery-state lock is treated as a dropped message with a warning.
- Telegram polling errors use the plugin's existing retry behavior.
- Existing polling lock, offset persistence, shutdown, and parent-watchdog
  behavior from plugin 0.5.1 is retained.
- Messages arriving before TUI session configuration follow the existing input
  queue behavior.

## Validation

Validation is staged so failures identify the affected boundary:

1. Run `node --check` and the plugin's `--self-test` mode.
2. Add failing-first tests for channel configuration defaults and parsing.
3. Add RMCP tests proving custom notifications reach the supplied callback and
   reconnect initialization retains it.
4. Add channel adapter tests for parsing, attachments, disabled policy,
   deduplication, burst capacity, and rate limiting.
5. Add protocol/app-server coverage for the new event and notification mapping.
6. Add TUI tests and snapshots proving the model payload retains routing data,
   display history hides raw JSON, images become local image inputs, and shell
   escape handling is disabled.
7. Run focused crate tests, formatting, and scoped lint fixes as required by the
   repository.
8. Build the release binary and expose it through a separate local symlink at
   `/home/netsky/.local/bin/codex-telegram` without modifying
   `/home/netsky/.local/bin/codex`.
9. Run a non-secret smoke test of `codex-telegram --version` and plugin startup.
10. Provide an end-to-end launch command that reads the user's existing
    `TELEGRAM_BOT_TOKEN` and `TELEGRAM_ALLOWED_CHAT_IDS` environment variables.
    The user performs the final live Telegram exchange with their bot.

The complete workspace test suite is not run without explicit permission, in
accordance with repository instructions. Focused tests for all changed crates
are part of the local deliverable.

## Non-Goals for the First Deliverable

- Publishing or pushing a GitHub branch, tag, release, or pull request.
- Replacing the installed Codex binary or its `current` release symlink.
- Restoring removed autonomous loop-control code.
- Implementing `/channels`, pause/mute queues, or distinct behavior for channel
  modes.
- Redesigning the Telegram plugin protocol or adopting the standalone
  logging-notification variant.
- General refactoring unrelated to the channel delivery path.

## Acceptance Criteria

- A release build based on Codex 0.147.0 is available locally as
  `codex-telegram` while the existing `codex` command still resolves to the
  original installed 0.147.0 binary.
- The plugin self-test and all focused tests for changed crates pass.
- With a valid bot token and allowed chat, an inbound Telegram message appears
  in the active TUI session as a readable message and starts a model turn.
- Routing metadata remains available to the model so `telegram_reply` can reply
  to the originating message or chat.
- Reaction, typing, attachment download/input, and outbound file tools remain
  available.
- No secrets or user-specific Telegram identifiers are committed.
