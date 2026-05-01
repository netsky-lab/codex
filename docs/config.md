# Configuration

For basic configuration instructions, see [this documentation](https://developers.openai.com/codex/config-basic).

For advanced configuration instructions, see [this documentation](https://developers.openai.com/codex/config-advanced).

For a full configuration reference, see [this documentation](https://developers.openai.com/codex/config-reference).

## MCP channels

MCP custom notifications can be promoted into active TUI user input only when a
server explicitly opts into channels:

```toml
[mcp_servers.telegram-channel.channel]
enabled = true
mode = "queue"
queue_capacity = 50
dedupe_capacity = 200
rate_limit_per_minute = 30
```

See [Channels](./channels.md) for the notification schema, Telegram bridge, and
security notes.

## Lifecycle hooks

Admins can set top-level `allow_managed_hooks_only = true` in
`requirements.toml` to ignore user, project, and session hook configs while
still allowing managed hooks from requirements and managed config layers. This
setting is only supported in `requirements.toml`; putting it in `config.toml`
does not enable managed-hooks-only mode.
