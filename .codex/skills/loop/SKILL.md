---
name: loop
description: Use when Codex should continue work autonomously with the built-in /loop command, including immediate continuation after each completed turn, timed intervals, maximum iteration limits, one-shot continuation, status checks, or stopping an active loop.
---

# Loop

Codex has a built-in TUI slash command named `/loop` for autonomous continuation. Use it when the user asks you to keep working, continue a backlog, run periodic checks, or resume the next useful step without waiting for another user message.

If the `loop_control` tool is available, use it to start, stop, or inspect the loop. Do not print `/loop stop` as assistant text; assistant text is transcript content, not a command channel.

## Commands

- `/loop now [--max N] <task>` starts immediately and submits the next iteration as soon as each agent turn completes.
- `/loop --immediate [--max N] <task>` is the same as `/loop now`.
- `/loop <minutes> [--max N] <task>` starts immediately, then schedules the next iteration after each completed turn with the given minute interval.
- `/loop once <task>` submits one autonomous follow-up and then stops.
- `/loop status` shows current loop state.
- `/loop stop` stops the loop.

## When To Use

Prefer immediate mode for active implementation backlogs where there are known remaining tasks:

```text
/loop now --max 20 continue the backlog; after each turn, inspect progress, pick the next pending task, implement it, verify it, and stop only when there is no useful next step
```

Prefer timed mode for monitoring, long-running services, rate-limited APIs, or tasks that need time between checks:

```text
/loop 10 --max 6 check whether the test run finished; if it did, inspect failures and fix the next actionable issue
```

Use `--max` by default for bounded autonomous work unless the user explicitly wants an unbounded loop.

## Safety

Do not start a loop for destructive, ambiguous, or high-risk work unless the user clearly asked for autonomous continuation. If the goal is complete or blocked, call `loop_control` with `action: "stop"` and a short reason, then report the blocker.
