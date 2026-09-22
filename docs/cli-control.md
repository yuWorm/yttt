# CLI Control

`yttt ctl` controls an already running local desktop through its private local socket
(or Windows named pipe). Use the same build and profile as the desktop. An isolated
development profile can be selected with `YTTT_PROFILE_ROOT` on both processes.
The default development profile depends on the executable path; an installed release
and a development binary do not address the same desktop. Restart the desktop after
updating it to a build with CLI control support.

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

## Results and scope

`--json` produces one JSON response on stdout, or a structured error on stderr.
Exit status is `0` for success, `2` for invalid requests, and `1` for operational
failures. Requests are never automatically retried. An `outcome_unknown` error means
the response was lost or timed out: inspect the target before retrying a mutation.
Responses are limited to 256 KiB; narrow large queries using target flags.

Layout mutations are rejected while a dialog or palette owns the window's input,
so pending UI actions cannot accidentally apply to a different project or tab.
Writes execute through the desktop's existing Host connection and control lease.
An observing or disconnected desktop cannot mutate state, and the CLI never takes
control from another Client. Headless Host control and automatic multi-turn Agent
scheduling are not provided by this interface.
