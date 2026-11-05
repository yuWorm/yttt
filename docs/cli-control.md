# CLI Control

`yttt ctl` controls an already running local desktop through its private local socket
(or Windows named pipe). Use the same build and profile as the desktop. An isolated
development profile can be selected with `YTTT_PROFILE_ROOT` on both processes.
The default development profile depends on the executable path; an installed release
and a development binary do not address the same desktop. Restart the desktop after
updating it to a build with CLI control support.

Human-readable help and output use the selected profile's Device language, even when no
desktop or Host is running. The CLI reads `config/device/settings.toml` without migrating
settings; only a missing Device file falls back to legacy `config/settings.toml`. System
or unreadable preferences use the operating-system language. Commands, option names,
protocol values, and machine-readable JSON remain unchanged.

## Installing the command

- **macOS:** drag the release app to `/Applications` or `~/Applications`, then launch it.
  The desktop automatically registers `/usr/local/bin/yttt` as a link to that bundle.
  If the directory needs administrator access, macOS asks for authorization. Development
  builds, custom `YTTT_PROFILE_ROOT` runs, and apps opened from a mounted DMG do not register.
  An existing command pointing elsewhere is never overwritten; a native alert explains
  conflicts or denied authorization. Resolve the issue and restart the app to retry.
  Keep the app at its installed location; moving it breaks the link. If you deliberately
  exclude `/usr/local/bin` from your shell's `PATH`, add it before using the short command.
- **Windows:** the installer adds the application directory to the installation scope's
  PATH by default. Open a new terminal after installing. Uninstall removes only a PATH
  entry added by the installer, preserving an entry that existed before installation.
  If PATH has been reorganized so the installer's addition cannot be identified safely,
  uninstall leaves it untouched rather than removing a potentially user-managed entry.
  The post-install launch prepends the selected application directory to the child's PATH
  while retaining the original user's inherited entries. Semicolon-containing destinations
  are rejected before installing. An installer-owned command must be upgraded in its existing
  directory and scope; uninstall first to change either. Uninstall reads the recorded scope
  rather than inferring it from current elevation. For per-user installs, running as another
  user is rejected before removing the app. Keep `yttt-installer-state.ini` with the application;
  missing or invalid scope metadata must be restored before uninstalling.
- **Linux:** run the installer included in the release archive; see the
  [installation instructions](../README.md#installation). It always establishes persistent
  shell configuration, independently of the invoking process's PATH. Bash login and
  non-login startup files are both covered; Zsh honors `ZDOTDIR` and Fish honors
  `XDG_CONFIG_HOME`. The installer requires `flock`; `~/.yttt-install.lock` is retained
  as a lock inode but is unlocked when the installer exits. HOME containing `:` is rejected
  before writing files, because it cannot be represented as one PATH element.
  Installer messages follow `LC_ALL`, then `LC_MESSAGES`, then `LANG`: Chinese locales
  select Simplified Chinese; other locales select English. The Windows installer offers
  English and Simplified Chinese independently of the application's Device preference.

The command and desktop use the same executable, so updating the application does not
leave a separately copied CLI behind. To bypass PATH registration on macOS, run
`/Applications/yttt.app/Contents/MacOS/yttt ctl --help` directly.

## Commands

```sh
yttt ctl --help
yttt ctl windows list --json
yttt ctl projects list --json
yttt ctl tabs list --project PROJECT_ID --json
yttt ctl panes list --project PROJECT_ID --json
yttt ctl agents list --project PROJECT_ID --json
```

Lists describe open local desktop windows and their terminal tabs. Recent projects,
file tabs, closed workspaces, and remote Client processes are outside this interface.
Window IDs last for the lifetime of the window. Project, tab, and pane identifiers
come from list responses; pane IDs are scoped to their project and tab. CLI-created
tabs and split panes use UUIDs so separate windows and later creations do not reuse
the same new terminal address. Existing layouts can attach to the same Host terminal
from multiple windows: `--window` selects the layout, not a separate copy of that process. Specify
`--window WINDOW_ID` when the same project is open in multiple windows. Ambiguous
mutations fail rather than choosing whichever window currently has focus.

## Identity and roles

```sh
yttt ctl whoami --json
yttt ctl agents create --project PROJECT_ID --provider claude --role orchestrator --title Lead
```

Every terminal the Host starts receives `YTTT_SELF`, its own encoded project, terminal
and generation. `whoami` reads it without contacting the desktop and reports `external`
outside a yttt terminal. A pane created with `--role orchestrator` also receives
`YTTT_AGENT_CONTROL_TOKEN`; `whoami` then asks the Host to verify it and reports
`verified`, `rejected` (the pane restarted or its role changed) or `unavailable`.
Worker panes report `not_granted`. The role takes effect when the pane's process starts,
is kept with the layout, and is never copied from a personal layout template. Inherited
identity variables are replaced at launch, so a process cannot pass its identity to a
terminal it opens. Roles guard against Agent mistakes; they are not a boundary against
other processes of the same user, which can still reach the desktop socket.

## Orchestrating Agents

Inside an orchestrator pane these commands go straight to the Host, so they also work
for projects on a Host-managed SSH connection:

```sh
yttt ctl agents spawn --provider codex --title Parser -- 'Fix the parser tests'
yttt ctl agents list --mine --json
yttt ctl agents close --agent TERMINAL
```

`spawn` opens a new Agent tab in the orchestrator's project and answers with the worker's
handle, its terminal session ID. The tab opens in the background: what the project showed stays
shown, so a person's typing does not land in the new worker. Provider options that skip or
pre-grant approvals (for example Claude Code's `--dangerously-skip-permissions`,
`--permission-mode` or `--allowedTools`, and Codex's `--yolo`, `--full-auto`, `-a`, `-s` or `-c`)
are refused with `forbidden`: approvals stay with a person, who can start such a worker directly. An orchestrator may list and close only workers it
started or that a person assigned to it; anything else is `forbidden`. Closing a worker's
only pane closes its tab, and a project's last terminal tab is never closed. In any other
pane these commands fail with `forbidden`. A person hands an existing Agent to an
orchestrator, or takes it back, from outside it:

```sh
yttt ctl agents assign --project PROJECT_ID --tab TAB_ID --pane PANE_ID --to TERMINAL
yttt ctl agents assign --project PROJECT_ID --tab TAB_ID --pane PANE_ID --clear
```

An orchestrator drives a turn and reads its outcome:

```sh
yttt ctl agents send --agent TERMINAL --text 'Now add tests'   # prints turn_seq_before
yttt ctl agents wait --agent TERMINAL --until finished --after-turn 1 --timeout 300
yttt ctl agents result --agent TERMINAL --json
yttt ctl agents interrupt --agent TERMINAL
```

`send` pastes into a worker that reported a finished or idle turn and presses Enter, like a
person typing; it refuses a busy worker with `not_ready`. Its answer carries the number of
the worker's last turn: wait with `--after-turn` set to it, so a turn that finished before
the input does not end the wait. `wait` takes up to 16 comma-separated handles and returns
when one meets the condition (`matched`), ends (`exited`), or the timeout passes
(`timed_out`, with the current states); `change` counts only changes after the wait began.
A wait lasts at most 600 seconds; call it again to keep waiting. `result` reads the worker's
last reply from its provider's transcript (Claude Code and Codex); for other providers, or
before a reply exists, `source` is `unavailable`. `interrupt` presses Esc in a working Claude
Code or Codex worker and reports whether its turn ended within `--timeout` (default 10 s).

### MCP server and skill

`yttt mcp` serves the same operations as Model Context Protocol tools on stdio (`yttt_whoami`,
`yttt_spawn_agent`, `yttt_list_agents`, `yttt_send`, `yttt_wait`, `yttt_result`,
`yttt_interrupt`, `yttt_release`, `yttt_close`). Register it with the orchestrator's provider,
for example `claude mcp add yttt -- yttt mcp`; the provider starts it inside the orchestrator
pane, so it inherits that pane's credential. Outside an orchestrator pane every tool except
`yttt_whoami` fails with `forbidden`. Some clients pass only selected environment variables to MCP servers; if
`yttt_whoami` reports `external` inside an orchestrator pane, configure the client to pass
`YTTT_SELF`, `YTTT_AGENT_HOOK_ENDPOINT` and `YTTT_AGENT_CONTROL_TOKEN` through. Tool waits default to 120 seconds, below the Host's limit,
because MCP clients may end long calls. [`docs/skills/yttt-orchestrator/SKILL.md`](skills/yttt-orchestrator/SKILL.md)
is a skill that teaches an orchestrator the loop, the use of `--after-turn`, and what to do when a
person steps in; copy it into the provider's skills directory.

### Handing a worker between a person and its orchestrator

A worker is driven by its orchestrator (`auto`), by a person (`human`), or `paused`.
Typing in a worker the orchestrator drives takes it over: the Host counts keys and pastes a
desktop sends to it, including `panes send`, as a person's. Replies the terminal sends by itself,
such as focus and mouse reports and cursor or color answers, do not take it over. The orchestrator's `send` and
`interrupt` then fail with `human_control`, and a `wait` in progress returns
`control_changed`. Anything the orchestrator had queued for the worker is dropped, because
the person may have changed the task.

```sh
yttt ctl agents send --agent TERMINAL --queue --text 'Then update the docs'   # in the orchestrator
yttt ctl agents release --agent TERMINAL                                       # in the orchestrator
yttt ctl agents control --project PROJECT_ID --tab TAB_ID --pane PANE_ID --set auto|human|paused
```

`--queue` holds up to 32 messages while the worker is busy, taken over or paused, and the
Host writes them in order once the worker is `auto` and ready, one per turn. The queue lives
in the Host's memory and is lost if the Host restarts. If a worker starts no turn within
30 seconds of a write (input such as a local slash command), the Host stops waiting for one.
An orchestrator cannot `close` a worker a person drives or paused. `release` hands the worker to a person.
A person hands it back, takes it over or pauses it with `agents control`, or with the
**Take Over Agent**, **Pause Agent** and **Hand Agent Back** commands on the focused pane;
the worker's tab shows who drives it. Orchestrators cannot answer approval prompts or
structured questions; `wait --until waiting` tells them a person is needed.

Ownership is kept with the layout and survives a Host restart. Every orchestrator request,
allowed or refused, is recorded in the Host's audit log with the orchestrator's terminal.

## Creating tabs

```sh
yttt ctl tabs create --project PROJECT_ID --title Tests --command 'cargo test'
yttt ctl agents create --project PROJECT_ID --provider codex -- 'Review the project and propose a plan'
yttt ctl agents create --project PROJECT_ID --provider claude --title Review
```

Agent providers are `codex`, `claude`, `grok`, `groky`, `opencode`, `pi`, and `omp`.
Arguments after `--` pass to the provider unchanged; use that provider's own syntax
for an initial prompt or other options. The desktop's normal Agent initialization,
hooks, environment, and session handling also apply to CLI-created Agent tabs.

A `created` response includes the target IDs and current startup state. It confirms
that the layout exists and startup was requested, **not** that a shell or Agent is
ready or that a command succeeded. Query `panes list` for the live lifecycle state.
Creating a tab or pane selects it in the window; list, read, and send commands do not
change selection. Layout changes use the existing workspace publication and restore
flow; creation does not promise that an asynchronous save has finished.

## Managing panes

```sh
yttt ctl panes split --project PROJECT_ID --tab TAB_ID --pane PANE_ID --direction vertical
yttt ctl panes split --project PROJECT_ID --tab TAB_ID --pane PANE_ID --direction horizontal --command 'cargo test'
yttt ctl panes resize --project PROJECT_ID --tab TAB_ID --pane PANE_ID --direction right --percent 5
yttt ctl panes rename --project PROJECT_ID --tab TAB_ID --pane PANE_ID --title Worker
yttt ctl panes focus --project PROJECT_ID --tab TAB_ID --pane PANE_ID
yttt ctl panes close --project PROJECT_ID --tab TAB_ID --pane PANE_ID
```

`horizontal` creates side-by-side panes; `vertical` stacks panes. `tabs focus`,
`tabs rename`, and `tabs close` use the same target flags without `--pane`.
Closing a pane terminates its process. Closing the only pane closes its tab;
closing a tab terminates its processes. The last terminal tab is protected: create
another tab first. No interactive confirmation is shown for explicit CLI close commands.

## Input and output

```sh
yttt ctl panes send --project PROJECT_ID --tab TAB_ID --pane PANE_ID --text 'cargo test' --enter
yttt ctl panes read --project PROJECT_ID --tab TAB_ID --pane PANE_ID --json
yttt ctl agents send --project PROJECT_ID --tab TAB_ID --pane PANE_ID --prompt-file task.md
printf '%s' 'Explain the failing test' | yttt ctl agents send --project PROJECT_ID --tab TAB_ID --pane PANE_ID --stdin
```

Input uses the terminal's current paste encoding. `--raw` on `panes send` sends
literal bytes from the UTF-8 text instead; `--enter` appends Enter. `agents send`
appends Enter automatically and requires a running Agent that has reported an idle,
completed, failed, or interrupted turn. It rejects working, waiting, unknown, and
not-yet-reported Agent states. Use `panes send` for deliberate interaction with an
approval prompt or another terminal program. A ready Agent state is a best-effort
observation, not a provider-level guarantee that a new turn will be accepted.

`input_accepted` means the Host accepted the terminal bytes. It does not promise
that the program consumed them, the shell command completed, or an Agent turn
succeeded. Input is bounded to 64 KiB. `panes read` returns the current viewport,
not a complete transcript or separate stdout/stderr. Agent list responses include
process and turn state, waiting reason, task, current action, children, and timestamps
when available; these depend on the provider's installed hooks and observations.

## Dispatch center

```sh
yttt ctl board show
yttt ctl board show --window WINDOW_ID
yttt ctl board answer --project PROJECT_ID --tab TAB_ID --pane PANE_ID \
  --request REQUEST_ID \
  --answers-json '[{"question_id":"delivery","selected_options":["Continue"],"custom_input":null}]' \
  --json
```

`board show` selects the native workspace-level Dispatch center, including on an observing
desktop. The target window must have at least one open project; an empty Dashboard returns
`not_ready` and is left unchanged. It clears project selection without closing projects,
restarting processes, or toggling the page closed. Consequently `projects list` reports no
selected project.
Explicit `tabs focus` or `panes focus` returns to the targeted project's work area.
`agents list --json` includes the structured `agent.board` snapshot with plans, execution
records, messages, and question requests. Use those request/question IDs and exact option
labels when answering.
Task-row usage in the Dispatch center comes from the task's associated execution record, not
from a per-todo spend allocation. Execution records also contain optional `current_action` and
`telemetry` fields. Missing metrics remain `null`, not zero; `cost_microusd` is integer
micro-USD and `self_estimated_completion` is an optional Agent estimate, not accepted progress.
Token scope differs by provider; see [Dispatch center](usage.md#dispatch-center) before
aggregating reported values.

`board answer` uses the same Host submission path as the native question cards. Every
question in a request must have an answer. Multi-select accepts several labels; custom
answers use `custom_input`. `agent_question_submitted` means the Host queued the answer.
Only a later question state of `answered` confirms that the OMP adapter accepted it.
Native terminal answers may win the race; already resolved, unavailable, read-only child,
or stale-scope requests are rejected. Submission never writes an answer into PTY input
and is never automatically replayed.


## Results and scope

`--json` produces one JSON response on stdout, or a structured error on stderr.
Exit status is `0` for success, `2` for invalid requests, and `1` for operational
failures. Requests are never automatically retried. An `outcome_unknown` error means
the response was lost or timed out: inspect the target before retrying a mutation.
Responses are limited to 16 MiB to include bounded Agent board histories; narrow large queries using target flags.

Layout mutations are rejected while a dialog or palette owns the window's input,
so pending UI actions cannot accidentally apply to a different project or tab.
Writes execute through the desktop's existing Host connection. Any connected desktop may
change the layout; terminal input and `panes send` need the terminal controller, which the
CLI never takes from another Client, and a disconnected desktop cannot write at all.
Orchestrator commands do not use the desktop: they go to the Host directly and follow the
rules in [Orchestrating Agents](#orchestrating-agents). The Host does not plan or schedule
work across turns by itself; an orchestrator Agent or a script decides what to send next.
