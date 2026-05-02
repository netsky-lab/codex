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

For bridge diagnostics while testing, enable verbose status fields:

```bash
export TELEGRAM_DEBUG=1
```

Inbound messages get a Telegram 👀 reaction by default after the bridge accepts them. To change or disable it:

```bash
export TELEGRAM_SEEN_REACTION="👍"
export TELEGRAM_SEEN_REACTION=0
```

Incoming Telegram files are downloaded under `$CODEX_HOME/telegram-channel-files` by default:

```bash
export TELEGRAM_DOWNLOAD_DIR="$HOME/.codex/telegram-channel-files"
export TELEGRAM_MAX_DOWNLOAD_BYTES=20971520
```

In groups, inbound messages include `bot`, `reply_to`, and `addressing`
metadata. `addressing.probably_addressed_to_bot` is a hint for multi-bot chats,
not a hard filter: Codex can still respond when the surrounding conversation
clearly asks this bot.

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
    "schema_version": 1,
    "bot": {
      "id": "bot user id",
      "username": "bot_username"
    },
    "reply_to": {
      "message_id": 100,
      "sender": {
        "id": "telegram user id",
        "is_bot": false,
        "username": "sender_username"
      },
      "text": "message being replied to"
    },
    "addressing": {
      "is_reply_to_bot": false,
      "is_reply_to_other_bot": false,
      "mentions_bot": true,
      "command_to_bot": false,
      "probably_addressed_to_bot": true
    }
  }
}
```

Codex turns that notification into a user message in the active TUI session when the MCP server has `channel.enabled = true`. The plugin also exposes:

- `telegram_reply`: sends a response back to `channel_message_id`, `chat_id`, or the last inbound chat. Long messages are split for Telegram.
- `telegram_typing`: shows a Telegram chat action such as `typing` or `upload_document`.
- `telegram_react`: sets an emoji reaction on an inbound Telegram message.
- `telegram_send_file`: uploads a local file to Telegram as a document or photo.
- `telegram_status`: reports polling, offset, allowlist, and recent routing state. With `TELEGRAM_DEBUG=1`, it also includes update counters and the last observed update summary.
