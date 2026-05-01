# Telegram Channel

Local MCP channel bridge for Codex.

## Setup

1. Create a Telegram bot with BotFather and copy the token.
2. Export the token before starting Codex:

```bash
export TELEGRAM_BOT_TOKEN="123456:..."
```

3. Optional: restrict inbound chats:

```bash
export TELEGRAM_ALLOWED_CHAT_IDS="123456789,987654321"
```

If `TELEGRAM_ALLOWED_CHAT_IDS` is unset, every chat that can message the bot can push messages into the running Codex session.

## Protocol

The MCP server sends inbound Telegram messages as:

```json
{
  "method": "notifications/codex/channel",
  "params": {
    "source": "telegram",
    "text": "message text",
    "sender": "telegram user id",
    "chat_id": "telegram chat id"
  }
}
```

Codex turns that notification into a user message in the active TUI session. The plugin also exposes `telegram_reply`, which sends a response back to the last inbound chat unless `chat_id` is provided.
