---
name: yttt-orchestrator
description: Drive other coding Agents in yttt terminals a person can watch. Use when running in a yttt pane created with --role orchestrator and a task should be split across worker Agents (Codex, Claude Code and others) that run in their own tabs.
---

# Orchestrating Agents in yttt

You run in a yttt terminal that a person made an orchestrator. You can start worker
Agents in new tabs of this project, give them work, wait for their turns, read their
replies and stop them. A person sees every worker's terminal and may take one over at
any time. Treat that as normal, not as an error.

Check first:

```sh
yttt ctl whoami --json
```

Continue only if `role` is `orchestrator` and `control` is `verified`. Otherwise tell the
person that this pane cannot orchestrate and stop.

If the `yttt` MCP server is configured (`yttt mcp`), use its tools (`yttt_spawn_agent`,
`yttt_send`, `yttt_wait`, `yttt_result`, ...). They take the same fields as the commands
below and return the same JSON.

## The loop

1. **Plan before spawning.** Split the work into tasks that do not edit the same files.
   Two workers changing one file at once will conflict. Keep tasks you cannot separate
   for one worker, or run them one after another.
2. **Start each worker with its whole task.** A new worker accepts input only after its
   first turn, so pass the task as the provider's initial prompt:

   ```sh
   yttt ctl agents spawn --provider codex --title "Parser tests" --json -- \
     'Fix the failing tests in crates/parser. Run cargo test -p parser. Write a summary of what you changed to .yttt/runs/parser-tests.md and finish.'
   ```

   Keep the returned `agent` handle. Ask every worker to end with a short summary, and
   to write longer results to a file under `.yttt/runs/`, because `result` returns only
   the final reply.
3. **Wait, then read.**

   ```sh
   yttt ctl agents wait --agent A,B --until finished --timeout 300 --json
   yttt ctl agents result --agent A --json
   ```

   `wait` returns as soon as one worker matches. Note which one (`agent`), handle it,
   and wait again for the rest. A `timed_out` answer is not a failure: look at the
   states it returns and wait again.
4. **Follow up with the turn number.**

   ```sh
   yttt ctl agents send --agent A --text 'Also cover the empty-input case.' --json
   # → {"sent":{"turn_seq_before":1,...}}
   yttt ctl agents wait --agent A --until finished --after-turn 1
   ```

   Always pass `--after-turn` with the `turn_seq_before` that `send` returned. Otherwise
   the turn that had already finished ends the wait at once.
5. **Close workers you no longer need** with `yttt ctl agents close --agent A`, and tell
   the person what each worker did.

## When a person steps in

- A `wait` that returns `control_changed`, or a `send` or `interrupt` that fails with
  `human_control`, means a person took the worker over or paused it. Stop sending to it.
  Do other work, or wait for that worker with `--until change` until `agents list --mine`
  shows its controller as `auto` again.
- Anything you queued for a worker is dropped when a person takes it over. Re-read its
  state and decide again; do not resend blindly.
- If a worker needs a decision you cannot make, hand it over:
  `yttt ctl agents release --agent A`, and say why.
- `--queue` is for one more instruction to a busy worker. Do not use it to line up a
  long plan; the person cannot see or edit the queue.

## Things you cannot do, by design

- Answer a worker's approval prompt or question. `wait --until waiting` tells you a
  worker is waiting. Tell the person which worker needs them and continue elsewhere.
- Drive panes you did not start and were not assigned. `forbidden` means exactly that.
- Interrupt providers other than Claude Code and Codex. Use `wait`, or ask the person.

## Error codes

| Code | Meaning | What to do |
|---|---|---|
| `not_ready` | The worker is busy or has not reported a turn yet | `wait --until ready`, then retry |
| `human_control` | A person drives or paused the worker | Leave it; see above |
| `forbidden` | Not your worker, or this pane is no longer an orchestrator | Do not retry |
| `not_found` | The worker's pane is gone | Remove it from your plan |
| `outcome_unknown` | The Host did not answer | Check `agents list --mine` before retrying |
