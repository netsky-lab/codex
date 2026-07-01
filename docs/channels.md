# Channels

Channels let a trusted MCP server deliver external messages into the active Codex
session. The initial supported bridge is `telegram-channel`, which long-polls a
Telegram bot and forwards allowed chat messages as MCP custom notifications.

## MCP Config

Channels are disabled by default for every MCP server. Enable them per server:

```toml
[mcp_servers.telegram-channel]
command = "node"
args = ["./plugins/telegram-channel/scripts/telegram-channel.mjs"]
env_vars = [
  "TELEGRAM_BOT_TOKEN",
  "TELEGRAM_ALLOWED_CHAT_IDS",
  "TELEGRAM_ALLOWED_THREAD_IDS",
  "TELEGRAM_ALLOWED_ROUTES",
  "TELEGRAM_POLL_TIMEOUT_SEC",
  "TELEGRAM_OFFSET_FILE",
  "TELEGRAM_DEBUG",
  "TELEGRAM_SEEN_REACTION",
  "TELEGRAM_DOWNLOAD_DIR",
  "TELEGRAM_MAX_DOWNLOAD_BYTES",
]

[mcp_servers.telegram-channel.channel]
enabled = true
mode = "queue"
queue_capacity = 50
dedupe_capacity = 200
rate_limit_per_minute = 30
```

`mode` is advertised to channel servers during MCP initialization. Current TUI
delivery is controlled locally with `/channels`:

```text
/channels status
/channels pause
/channels resume
/channels mute
/channels unmute
/channels clear
```

Paused messages are queued in memory and submitted on resume. Muted messages are
dropped with an audit entry in the TUI history.

## Architecture

Channels are a generic MCP delivery path, not a Telegram-specific code path.
The Codex core only knows about `ChannelMessageEvent` values produced by trusted
MCP servers with `channel.enabled = true`.

The delivery path is:

1. A channel-capable MCP server emits a channel notification.
2. `codex-mcp` validates the server channel policy, deduplicates by channel
   message id, applies the configured rate limit, and emits
   `EventMsg::ChannelMessage`.
3. The TUI records an audit entry, applies local `/channels` state
   (`pause`, `resume`, `mute`, `unmute`, `clear`), and submits accepted messages
   to the model as structured user input.
4. The app server forwards channel messages to app clients as
   `channel/message`.

Transport integrations live outside that generic path. The Telegram bridge is a
plugin-provided MCP server that maps Telegram Bot API updates into channel
notifications and exposes Telegram-specific tools such as reply, reaction,
typing, and file upload.

## Notification Protocol

Channel servers can send JSON-RPC notifications with one of these methods when
the MCP transport preserves custom notifications:

```text
notifications/codex/channel
notifications/claude/channel
notifications/channel
```

Params may be a string or an object. Object params should use this schema:

```json
{
  "id": "telegram:123:456",
  "schema_version": 1,
  "source": "telegram",
  "channel": "telegram",
  "sender": "42",
  "text": "message text",
  "chat_id": "123",
  "telegram_message_id": 456
}
```

Codex ignores empty text, deduplicates by `id`, rate-limits accepted messages,
and submits accepted messages to the model as structured JSON user input with
shell escapes disabled.

For stdio MCP servers using transports that drop non-standard notifications,
wrap the channel notification in a standard logging notification:

```json
{
  "method": "notifications/message",
  "params": {
    "level": "info",
    "logger": "codex-channel",
    "data": {
      "method": "notifications/codex/channel",
      "params": {
        "id": "telegram:123:456",
        "schema_version": 1,
        "source": "telegram",
        "text": "message text"
      }
    }
  }
}
```

## Telegram Plugin

The Telegram bridge is packaged as a Codex plugin marketplace entry. To install
this branch from Git:

```bash
codex plugin marketplace add netsky-lab/codex --ref channels-telegram-v0.141
codex plugin add telegram-channel@codex-channels
```

This installs **Telegram Channel** from the Codex Channels marketplace. The
plugin contributes the MCP server definition; Telegram tokens and allowlists are
still provided through the environment variables listed below.

The plugin requires:

```bash
export TELEGRAM_BOT_TOKEN="123456:..."
export TELEGRAM_ALLOWED_CHAT_IDS="123456789,987654321"
```

For Telegram supergroup forum topics, route by `chat_id:message_thread_id`:

```bash
export TELEGRAM_ALLOWED_ROUTES="-1001234567890:12,-1001234567890:34"
```

`TELEGRAM_ALLOWED_ROUTES="-1001234567890:*"` allows all topics in one
supergroup. `TELEGRAM_ALLOWED_THREAD_IDS` can also restrict topic ids after the
chat has been allowed with `TELEGRAM_ALLOWED_CHAT_IDS`.

For disposable local testing only:

```bash
export TELEGRAM_ALLOW_ALL_CHATS=1
```

The plugin persists Telegram offsets at
`$CODEX_HOME/telegram-channel-offset.json` unless `TELEGRAM_OFFSET_FILE` is set.
Use a separate `TELEGRAM_OFFSET_FILE` for each bot token when running multiple
Telegram channel bridges at the same time.
Set `TELEGRAM_DEBUG=1` during bridge debugging to include update-level
diagnostics in `telegram_status`; normal status output keeps those details out.
Accepted inbound messages get a Telegram 👀 reaction by default; set
`TELEGRAM_SEEN_REACTION` to a different emoji or `0` to disable it.
Incoming Telegram files are downloaded under
`$CODEX_HOME/telegram-channel-files` unless `TELEGRAM_DOWNLOAD_DIR` is set.
`TELEGRAM_MAX_DOWNLOAD_BYTES` defaults to 20 MiB.
Invalid numeric env values and malformed topic routes are ignored and reported
by `telegram_status`; with `TELEGRAM_DEBUG=1`, the exact configuration warning
is included.
Group messages include `bot`, `reply_to`, and `addressing` metadata so agents
in multi-bot chats can distinguish explicit mentions, commands, replies to this
bot, and replies to other bots. These are routing hints, not hard filters.
It exposes:

- `telegram_reply`: replies by `channel_message_id`, explicit `chat_id`, or the
  latest inbound allowed chat.
- `telegram_typing`: shows Telegram `typing` and other chat actions.
- `telegram_react`: sets an emoji reaction on an inbound Telegram message.
- `telegram_send_file`: uploads a local file to Telegram as a document or photo.
- `telegram_status`: reports polling, allowlist, offset, and routing state.

## Security Notes

Treat channel servers as input adapters, not trusted users. Keep
`channel.enabled = false` unless the server is expected to write into the active
session. Use Telegram chat allowlists for real bots. Do not expose a bot token to
untrusted workspaces.

Channel messages are still user input. Codex records an audit entry before
submitting them and disables local shell escape parsing for inbound messages, but
the model can still read and act on their text.

## Validation

Before publishing a channel plugin:

```bash
node --check plugins/telegram-channel/scripts/telegram-channel.mjs
node plugins/telegram-channel/scripts/telegram-channel.mjs --self-test
cargo test -p codex-mcp channel_notification_tests
cargo check -p codex-config -p codex-protocol -p codex-mcp -p codex-tui
```
