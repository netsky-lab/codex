---
name: loop
description: Use when the user requests the local Codex /loop runner, repeated checks or autonomous continuation in the current TUI, or asks to inspect or stop that loop.
---

# Local loop

Use the real `loop_control` tool to operate this runner. Printing `/loop`, shell
sleep loops, native goals, and scheduled automations do not control it. Enable
looping only for requested repeated work; an ordinary task is not a loop request.

## Control

`action` is `start`, `stop`, or `status`. For `start`, choose:

| `mode` | Behavior |
| --- | --- |
| `timed` | First turn when idle; subsequent turns wait `interval_minutes` after completion. Interval: 1–525600 minutes. |
| `immediate` | Continue when idle after each completed turn. |
| `once` | Submit one follow-up turn when idle. |

`max_iterations` is an optional positive limit on submitted loop turns. Omitting
it allows indefinite repetition until stopped. Starting replaces the current
loop and resets its counter. Prefer `status` when inspecting existing work.

Preserve the user's task, interval, limit, and stop conditions in the request.
Keep `prompt` within 1024 UTF-8 bytes and optional `reason` within 512 bytes.
An omitted prompt uses the built-in autonomous continuation prompt.

Example tool arguments:

```json
{"action":"start","mode":"timed","interval_minutes":10,"max_iterations":6,"prompt":"Check the staging build. Call loop_control with action=stop when it passes. Continue monitoring if the native goal is blocked."}
```

On each iteration, perform the requested work. Call `stop` when the user requests
it or their explicit stop condition is met. Native goal completion, blockage, or
clearing alone does not disable this independent loop.

## State and lifetime

The returned TUI status is authoritative. Report success only after a successful
response; after an error or timeout, inspect `status` before deciding to retry a
mutation. Without an attached active TUI, report that local control is unavailable.
Only the root agent can control its active TUI thread.

Queued user input, pending steers, rate-limit recovery, and an active native goal
take priority. A pending loop resumes when eligible. Interruption or a turn error
stops it; do not silently undo a user's stop.

State belongs to this TUI session and is not persisted across exit, restart, or
task switching. tmux keeps the process alive through terminal disconnection; it
does not make loop state survive a Codex restart.

For manual entry in the TUI, users can use `/loop 10 --max 6 <prompt>`,
`/loop now <prompt>`, `/loop once <prompt>`, `/loop status`, or `/loop stop`.
