# Telegram Channel

Local MCP channel bridge for Codex.

## Setup

1. Create a Telegram bot with BotFather and copy the token.
2. Export the token before starting Codex:

```bash
export TELEGRAM_BOT_TOKEN="123456:..."
```

3. Allow the Telegram chats that may write into Codex:

```bash
export TELEGRAM_ALLOWED_CHAT_IDS="123456789,987654321"
```

Polling is disabled unless `TELEGRAM_ALLOWED_CHAT_IDS` is set. For a disposable local test bot you can opt out with:

```bash
export TELEGRAM_ALLOW_ALL_CHATS=1
```

4. Optional: choose where the Telegram update offset is persisted:

```bash
export TELEGRAM_OFFSET_FILE="$HOME/.codex/telegram-channel-offset.json"
```

## Protocol

The MCP server sends inbound Telegram messages as:

```json
{
  "method": "notifications/codex/channel",
  "params": {
    "source": "telegram",
    "text": "message text",
    "sender": "telegram user id",
    "chat_id": "telegram chat id",
    "id": "telegram:<chat_id>:<message_id>",
    "schema_version": 1
  }
}
```

Codex turns that notification into a user message in the active TUI session when the MCP server has `channel.enabled = true`. The plugin also exposes:

- `telegram_reply`: sends a response back to `channel_message_id`, `chat_id`, or the last inbound chat. Long messages are split for Telegram.
- `telegram_status`: reports polling, offset, allowlist, and recent routing state.
