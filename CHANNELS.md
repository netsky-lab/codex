# Telegram and local loops on Codex 0.153.4

This branch forward-ports `channels-telegram-v0.147.0-local` onto the official
`rust-v0.153.4` release and restores the independent local `/loop` runner.
Native Codex goals remain available.

## Linux server bundle

The `channels-release` workflow builds an x86_64 Linux musl archive on pushes
to `channels-telegram-v*` branches. Download the archive and its `.sha256` file
from that run's artifacts, then verify and extract them:

```sh
sha256sum -c codex-channels-*.tar.gz.sha256
tar -xzf codex-channels-*.tar.gz
cd codex-channels-*/
./install.sh
~/.local/bin/codex-channels --version
```

The installer keeps the canonical resource layout, including the code-mode
host, bubblewrap, ripgrep, packaged zsh and Telegram marketplace. It installs
the separate `codex-channels` command under `~/.local/bin`; Node.js 20 or newer
must also be available for the Telegram bridge. Run `codex-channels` in a
persistent terminal session such as tmux to keep a local loop alive after SSH
disconnects. The loop ends when that Codex process exits.

To register the bundled Telegram marketplace during the first installation,
use `INSTALL_TELEGRAM_PLUGIN=1 ./install.sh`. Otherwise register the installed
bundle later with `codex-channels plugin marketplace add <installed-bundle>`
and `codex-channels plugin add telegram-channel@codex-channels`.
The installer prints its destination and refuses to overwrite an existing
version directory. The workflow creates a downloadable artifact by default;
publishing a GitHub Release requires an explicit manual dispatch option.

## Build and run

From the repository root:

```sh
just assemble-codex-package --cargo-profile dev --package-dir ./dist/codex-channels-dev
./dist/codex-channels-dev/bin/codex --version
```

Use `--cargo-profile release` for an optimized build. The canonical package
builder fetches the matching Codex-built V8 artifacts and includes the code-mode
host and runtime resources. Keep the package directory intact and run its
executable directly. Node.js must be available on PATH for the Telegram plugin.

Register the local marketplace with the newly built executable, from the
repository root:

```sh
./dist/codex-channels-dev/bin/codex plugin marketplace add "$PWD"
./dist/codex-channels-dev/bin/codex plugin add telegram-channel@codex-channels
```

Export `TELEGRAM_BOT_TOKEN` and `TELEGRAM_ALLOWED_CHAT_IDS` (or
`TELEGRAM_ALLOWED_ROUTES`) before launching that executable. The plugin README
documents forum topics, attachments, reactions, routing and offset files.
Use one offset file per bot. A separate Codex home can be selected with the
standard `CODEX_HOME` environment variable when testing this build.

## Loop commands

```text
/loop now --max 5 Review the next failing test, fix it and verify the change.
/loop 10 Check the build status and report meaningful changes.
/loop once Recheck the current project state.
/loop status
/loop stop
```

The first iteration starts when the current session is idle. Timed loops wait
the requested number of minutes between completed iterations; immediate loops
continue after each completed turn. `--max` limits the submitted loop turns;
omitting it leaves the loop running until stopped. `once` submits one turn.
Without a prompt, the runner uses its built-in autonomous continuation prompt.

Ordinary queued input has priority. A native goal that is actively continuing
takes priority too; completing or blocking that goal does not itself disable
the loop. Interruption or an error stops the loop. Loop state is local to the
current TUI session and does not persist across exit/restart. Old timers cannot
start work in a different task. A loop prompt is limited to 1024 UTF-8 bytes.

The model can use `loop_control` to operate the attached local runner. It is
restricted to the root agent. `/loop status` shows the authoritative local
state.

The built-in `loop` skill is installed automatically with this binary, independently
of the Telegram plugin. Codex can discover it for loop requests, or users can
invoke it explicitly with `$loop` to learn or operate the local runner.

## Channel compatibility

The plugin is version 0.5.2. It uses direct
`notifications/codex/channel` notifications. Inbound polling requires the
client's `codex/channel-notifications` capability with `schemaVersion: 1`, a
bot token and allowed chat/topic routing. A discovery-only client can list
tools and read status without acquiring the polling lock or consuming updates.

As in the 0.147 branch, the configured `ask`, `queue`, `immediate` and `context`
mode names are advertised metadata. Accepted incoming messages are submitted
to the active TUI session through its normal input path. There is no separate
channel approval/holding queue. Incoming messages cannot invoke TUI shell
escape syntax. Their structured envelope retains routing and attachment
metadata while the transcript displays a readable message.
The complete serialized inbound event (text, routing metadata and attachment
paths together) is limited to 8000 UTF-8 bytes; oversized events are rejected
with a diagnostic, rather than injecting unbounded content into model context.

## Verification

The repository uses `just test` (nextest), not direct `cargo test`:

```sh
just test -p codex-config
just test -p codex-rmcp-client -p codex-mcp
just test -p codex-app-server-protocol
just test -p codex-tui loop_
env -u NO_COLOR just test -p codex-tui
just test -p codex-core loop_control
just write-config-schema
just write-app-server-schema
node --check plugins/telegram-channel/scripts/telegram-channel.mjs
node plugins/telegram-channel/scripts/telegram-channel.mjs --self-test
```

These are local checks. A live bot exchange additionally requires the user's
Telegram credentials and an allowed chat.
The remote RMCP integration tests require the locally built
`codex-rs/target/debug/codex` executable, so build the CLI before running them.
