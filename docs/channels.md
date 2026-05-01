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
  "TELEGRAM_POLL_TIMEOUT_SEC",
  "TELEGRAM_OFFSET_FILE",
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

The plugin requires:

```bash
export TELEGRAM_BOT_TOKEN="123456:..."
export TELEGRAM_ALLOWED_CHAT_IDS="123456789,987654321"
```

For disposable local testing only:

```bash
export TELEGRAM_ALLOW_ALL_CHATS=1
```

The plugin persists Telegram offsets at
`$CODEX_HOME/telegram-channel-offset.json` unless `TELEGRAM_OFFSET_FILE` is set.
It exposes:

- `telegram_reply`: replies by `channel_message_id`, explicit `chat_id`, or the
  latest inbound allowed chat.
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
