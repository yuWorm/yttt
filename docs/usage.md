# yttt Usage

## Product Model

The workspace hierarchy is:

```text
Project
  -> Work Item Tabs
    -> Terminal Tab
      -> Split Tree
        -> Terminal Pane
    -> File Tab
```

The sidebar shows only currently opened projects. Recent projects are reachable from the
project palette, not shown by default.

## Project Files and Editor

Terminal tabs and project files share one tab strip. Opening a file creates a file tab;
opening the same canonical path again selects the existing tab. Switching projects does not
close tabs: each project restores its own active work item, open file tabs, file-tree
expansion, panel visibility, and panel width.

The folder button is fixed after the scrollable tab area. It remains visible even when tabs
overflow, toggles the right project tree, and is visibly active only while that tree is open.
The left project sidebar is resized from its right edge; the right file tree is resized from
its left edge. Widths are persisted when a drag ends. The left sidebar keeps its expanded
width while collapsed, and hiding the right tree does not overwrite its saved width.

Directories load lazily. Refresh invalidates stale scans and reloads the root plus all
expanded directories. Git status decorates paths when available. Hidden entries are governed
by `project_panel.show_hidden`.

### Saving and external changes

`file.save` saves the active file. Autosave modes are:

- `off`: only explicit saves write the file.
- `on_focus_change`: save when the file loses focus, the work item changes, or the project
  changes.
- `after_delay`: save after `editor.autosave_delay_ms` without a newer edit.

Writes use a temporary file plus rename and verify the last known disk fingerprint first.
If a file changed on disk, choose Reload, Overwrite, or Cancel. If it was deleted, choose
Recreate or Cancel. A clean file changed externally reloads automatically; a dirty file is
never silently replaced.

Closing a dirty file offers Save, Discard, and Cancel. Project and app close combine all
dirty files with running terminal processes into one decision: Save All and Continue,
Discard and Continue, or Cancel. A save failure leaves the file, project, or window open.

### File editing limits

- Only regular UTF-8 text files are opened.
- The maximum file size is 10 MiB.
- Canonical paths must remain inside the project root.
- Symlinked directories are shown but not traversed.
- There is no continuous filesystem watcher; focus, project-tree refresh, and save boundaries
  perform external-change checks.
- The tree does not create, rename, move, duplicate, or delete filesystem entries.

## Settings TOML

The settings file is `<app config>/settings.toml`. These are the complete defaults:

```toml
[general]
language = "system"

[theme]
name = "yttt-dark"
# terminal = "another-theme" # optional; omitted to follow the UI theme

[notifications]
system = false

[terminal]
shell = "auto"
font_family = ""
font_size = 13.0
line_height = 1.15
padding = 6.0
scrollback = 10000
close_on_exit = true
show_scrollbar = true

[editor]
font_family = ""
font_size = 14.0
line_height = 1.4
tab_size = 4
soft_wrap = false
line_numbers = true
autosave = "off"
autosave_delay_ms = 1000
auto_detect_language = true
default_language = "plain_text"

[editor.lsp]
enabled = false
command = ""

[project_panel]
default_open = true
show_hidden = false
width = 280.0
project_sidebar_width = 216.0
```

Editor font family, font size, line height, soft wrap, and line numbers update all open files
without replacing their text or saved baseline. `tab_size` applies to files opened after the
change; reopen an existing file to apply it. Changing autosave to `off` cancels pending delayed
saves. `default_open` affects new project sessions. Editing `width` updates the selected
project and the default for future projects, while other open projects retain their own
widths. Valid width ranges are 200–520 px for the right tree and 160–420 px for the left
sidebar.

## Layout TOML

Global default layout:

```text
<app config>/default-layout.toml
```

Shareable project layout:

```text
<project>/.yttt/layout.toml
```

Personal app-local layout:

```text
<app config>/projects/<encoded-project-path>/layout.toml
```

Layout precedence is:

```text
project .yttt/layout.toml, otherwise global default
  -> personal mode = "patch" merges by stable tab/pane id
  -> personal mode = "replace" replaces the selected base
```

Projects without `.yttt/layout.toml` do not receive a copied local snapshot. They read the
latest global default when opened. Already-open projects keep their current tabs, panes, and
terminal processes when the global default is saved, reloaded, or reset.

`layout.default.edit`, `layout.default.reload`, and `layout.default.reset` manage the global
default and work without an open project.

`layout.save_current` writes the current runtime layout as a strict personal
`mode = "replace"` file. It does not modify the repository.

`layout.export_project_config` writes `<project>/.yttt/layout.toml` explicitly.

`layout.project.edit` edits the highest-priority project source: an existing personal file,
then project config, or a newly created personal replacement for an inherited project.
Invalid personal files open as raw TOML with diagnostics and are not overwritten until valid.

`layout.reset_local_override` deletes the personal file and restores inheritance the next
time the project is opened. It does not replace the currently running workspace.

`layout.open_file` reveals the personal file first, then project config, then the global
default used by an inherited project.

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

### Global default example

The global template intentionally has no project name. The project directory name is injected
when the template is materialized.

```toml
[project]
default_tab = "shell"

[[tabs]]
id = "shell"
title = "Shell"
layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }
```

### Personal layout V1

Patch example:

```toml
version = 1
mode = "patch"

[layout.project]
default_tab = "agent"

[[layout.tabs]]
id = "agent"
title = "Personal Agent"
```

Replacement example:

```toml
version = 1
mode = "replace"

[layout.project]
name = "yttt"
default_tab = "shell"

[[layout.tabs]]
id = "shell"
title = "Shell"
layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }
```

The V1 header and every nested layout object reject unknown fields. Missing or unsupported
`version`, unknown `mode`, and mode/body mismatches produce visible warnings rather than
falling back to an older schema.

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
project_panel.toggle
project_panel.refresh
tab.palette
pane.palette
tab.new
file.save
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
layout.default.edit
layout.default.reload
layout.default.reset
layout.project.edit
layout.save_current
layout.export_project_config
layout.reset_local_override
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
- Left and right sidebars drag in the correct direction and restore their saved widths.
- The folder button remains fixed after the tab scroll area and reflects tree visibility.
- Terminal and file tab rows are clickable; reopening a file deduplicates its tab.
- Project switching preserves each project's file tabs and file-tree state.
- Hidden-file changes and refresh reload expanded directories.
- Editor font, size, line height, soft wrap, and line numbers update open files without losing
  text; tab size applies after reopening.
- Manual save and both autosave modes write files.
- External modification and deletion show the appropriate conflict choices.
- Dirty file, dirty project with running terminals, and window close all protect unsaved data.
- Pane focus can be changed from keyboard and pointer.
- Invalid layout TOML produces a visible error.
- Closing a project with running panes asks for confirmation.

## Known Limits

- The project tree and text editor are not a general-purpose file manager or full IDE.
- File editing is limited to regular UTF-8 files up to 10 MiB, without a continuous watcher or
  create/rename/move/delete operations.
- No client/server terminal runtime.
- No live process restore after restart.
- No output parser for agent internal state.
- No GUI layout editor.
- No OS-level notification click handler yet.
- No packaged release artifact yet. `scripts/run-dev-app.sh` is a development smoke
  wrapper, not a release package.
