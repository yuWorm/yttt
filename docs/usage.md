# yttt Usage

## Product Model

The workspace hierarchy is:

```text
Project
  -> Tabs
    -> Split Tree
      -> Terminal Pane
```

The sidebar shows only currently opened projects. Recent projects are reachable from the
project palette, not shown by default.

## Layout TOML

Shareable project layout:

```text
<project>/.yttt/layout.toml
```

Personal app-local layout:

```text
<app config>/projects/<encoded-project-path>/layout.toml
```

`layout.save_current` writes the app-local layout. It does not modify the repository.

`layout.export_project_config` writes `<project>/.yttt/layout.toml` explicitly.

`layout.open_file` opens the project layout when it exists. If the project layout does
not exist but an app-local layout exists, it opens the app-local layout.

Example:

```toml
[project]
name = "yttt"
default_tab = "dev"

[[tabs]]
id = "dev"
title = "Dev"

[tabs.layout]
type = "split"
direction = "horizontal"
ratio = 0.65
left = { type = "pane", id = "server", title = "server", command = "npm run dev" }
right = { type = "pane", id = "shell", title = "shell", command = "$SHELL" }

[[tabs]]
id = "agent"
title = "Agent"
layout = { type = "pane", id = "codex", title = "Codex", command = "codex", kind = "agent", notify_on_exit = true, detector = "codex" }
```

`detector` is reserved for future output parsing. The current MVP does not parse agent
output.

## Keybindings

The editable keybindings file is:

```text
<app config>/keybindings.toml
```

Open or create it from the app with:

```text
settings.keybindings
```

Keybinding conflicts are reported on startup and through the visible keybinding warning
state.

Important default commands:

```text
command_palette.open
project.palette
tab.palette
pane.palette
tab.new
pane.split_vertical
pane.split_horizontal
pane.focus_left
pane.focus_right
pane.focus_up
pane.focus_down
pane.resize_left
pane.resize_right
pane.resize_up
pane.resize_down
settings.notifications
```

## Agent Status

A pane is treated as an agent pane when:

- `kind = "agent"` is set in layout TOML, or
- the command basename is `codex` or `claude`.

The app only reports process-level state:

- `agent running`
- `agent completed`
- `agent failed`

It does not infer thinking, waiting for input, tool use, or patch application states.
Those require future output detectors.

In-app toast is always produced for agent exit events when `notify_on_exit = true`.
System notification delivery is controlled by:

```text
settings.notifications
```

User-killed agent exits are not reported as failures.

## Manual Smoke Checklist

Run these before marking a product phase complete:

- `cargo run` opens an empty workspace.
- `YTTT_OPEN_PROJECT=/path/to/project cargo run` opens that project.
- `YTTT_DEV_FIXTURE=1 cargo run` shows the development fixture.
- `YTTT_DEV_FIXTURE=agent-exit cargo run` produces an agent completion toast.
- On macOS, `scripts/run-dev-app.sh --fixture dev` creates and opens
  `target/dev-app/yttt.app` for UI-tool-friendly smoke testing.
- On macOS, `scripts/run-dev-app.sh --fixture agent` opens the agent exit fixture
  through the same `.app` wrapper.
- Terminal panes accept input and render output.
- Terminal panes resize with the split area.
- Command, project, tab, and pane palettes can be opened from keyboard.
- Sidebar project rows are clickable.
- Tab rows are clickable.
- Pane focus can be changed from keyboard and pointer.
- Invalid layout TOML produces a visible error.
- Closing a project with running panes asks for confirmation.

## Known Limits

- No file explorer or editor.
- No client/server terminal runtime.
- No live process restore after restart.
- No output parser for agent internal state.
- No GUI layout editor.
- No OS-level notification click handler yet.
- No packaged release artifact yet. `scripts/run-dev-app.sh` is a development smoke
  wrapper, not a release package.
