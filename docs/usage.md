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

## First Launch

The first launch opens onboarding before the empty workspace. First choose one of two default
layout types:

- **Split view:** one tab with the coding agent on the left and an interactive shell on the right.
- **Separate tabs:** one agent tab and one shell tab.

Next choose the built-in coding agent: Codex (`codex`), Claude Code (`claude`), OpenCode
(`opencode`), Pi (`pi`), or Oh My Pi (`omp`). The command palette remains available from both
onboarding steps through its configured shortcut and the visible Command Palette action.

Completing onboarding sets `general.onboarding_completed = true`; subsequent launches go directly
to the workspace. Projects opened afterward inherit the generated global default unless they
provide a project or personal layout override.

For onboarding development or demos, force the flow even after it has been completed:

```sh
YTTT_FORCE_ONBOARDING=1 cargo run
```

`YTTT_FORCE_ONBOARDING` accepts `1`, `true`, `yes`, or `on` case-insensitively. The override does
not clear the persisted completion marker; every normal launch remains forced only while the
environment variable is enabled.

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

### SSH projects

Run **Open SSH Project** from the command palette, the empty-workbench action, or the project
sidebar menu. This opens a dedicated picker instead of the SSH settings page:

1. Select a saved connection, or choose **New connection**.
2. Enter the host, port, user, optional starting root, and authentication details.
3. Accept or reject an unknown server fingerprint.
4. Browse remote directories over SFTP and open the current directory.

Authentication modes:

- **Auto**: try SSH agent, then the configured private key, then a previously saved password.
- **Agent**: authenticate only with identities from the SSH agent.
- **Private key**: load the configured key file and optional masked passphrase. The passphrase is
  used only for the current attempt and is not persisted.
- **Password**: use the password exactly as entered; **Remember password** writes it to the
  operating-system credential store, never to `ssh-connections.toml`.

Connecting verifies the server key against OpenSSH `known_hosts`. A new key opens a blocking
Trust/Reject dialog with an option to save it; a changed known key is rejected. The directory
browser starts at the saved root or the remote home directory and lists only directories. The
opened directory becomes the hard SFTP boundary: canonicalized reads and saves cannot escape it,
and symlinked directories are listed but not traversed. Use **SSH connections** in Settings only
when managing saved endpoints outside the open flow.

Recent SSH projects are marked **Remote/SSH**. Selecting one automatically reconnects its saved
connection, validates the remote root over SFTP, and opens it; connection or root failures remain
in the picker with retry and credential-edit actions.

Remote terminal panes request a PTY on the same authenticated SSH transport. The project tree,
editor reads, and conflict-checked temporary-file saves share its SFTP subsystem. Disconnecting
closes those sessions. Reconnecting refreshes expanded tree directories. Restored remote projects
remain visible while disconnected, but terminal and file operations require reconnecting the saved
connection.

Git status decorations, branch switching, and the diff panel execute `git` inside the configured
remote root over non-PTY SSH command channels. Status refreshes after opening or reconnecting,
manual tree refreshes, remote saves, and tree mutations.

SSH-backed projects use the global default layout; project-local and personal layout files are
local-project features. Recursive filesystem watching is local-only. Remote open documents and Git
status are rechecked during tree refreshes rather than watched continuously.

### Code navigation, folding, and search

The editor resolves its language from the explicit editor setting, file name, or file
extension. Language-aware highlighting covers plain text, TOML, JSON, JSONC, YAML, Markdown,
Bash, C, C#, C++, Fish, GDScript, Go, Java, JavaScript, Kotlin, Lua, PHP, PowerShell, Python,
Ruby, Rust, Scala, Swift, TypeScript, XML, and Zig.
Windows development files such as `.csx`, `.ps1`, `.psm1`, `.csproj`, `.vcxproj.filters`,
`.props`, `.targets`, `.xaml`, and `.sln` are detected by extension. The default icon theme
distinguishes C#, PowerShell, XML, and Windows solution/project files; matching icons from an
imported icon theme take precedence.

When the language has a syntax tree, cursor-following breadcrumbs show the enclosing symbols
above the document. Select a breadcrumb to move the cursor to that declaration. Multiline
structural regions provide fold controls in the line-number gutter; folding preserves the header
and closing line, and hides only the body.

Use the **Find** toolbar button, `⌘F` on macOS, or `Ctrl+F` elsewhere to open the in-file
search control. Search matches are highlighted and the current match can be traversed with
the control's navigation buttons.

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
- Canonical paths must remain inside the local or configured remote project root.
- Symlinked directories are shown but not traversed.
- Local active projects are watched recursively. Create, modify, and remove events refresh
  expanded tree directories, open-document disk state, and Git status after a short debounce.
- Create, rename, and delete work in local and SSH trees. Copy/move paste works only between local
  projects; operations involving an SSH project are rejected.

## Settings TOML

The settings file is `<app config>/settings.toml`. These are the complete defaults:

```toml
[general]
language = "system"
ui_font_family = ""
ui_font_size = 16.0
ui_line_height = 1.618034
onboarding_completed = false

[theme]
name = "yttt-dark"
# terminal = "another-theme" # optional; omitted to follow the UI theme
# icon_theme = "Material Icon Theme" # optional; defaults to built-in icons

[notifications]
system = false

[terminal]
shell = "auto"
custom_shells = []
font_family = ""
font_size = 13.0
line_height = 1.15
padding = 6.0
scrollback = 10000
show_scrollbar = true

[editor]
font_family = ""
font_size = 14.0
line_height = 1.4
tab_size = 4
soft_wrap = false
line_numbers = true
vim_mode = false
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
without replacing their text or saved baseline. `vim_mode` enables modal keybindings immediately
for open project code editors; mode state is kept per document. `tab_size` applies to files opened
after the change; reopen an existing file to apply it. Changing autosave to `off` cancels pending
delayed saves. `default_open` affects new project sessions. Editing `width` updates the selected
project and the default for future projects, while other open projects retain their own
widths. Valid width ranges are 200–520 px for the right tree and 160–420 px for the left
sidebar.

Vim mode supports Normal, Insert, Visual, and Visual Line modes; counts; `h/j/k/l`, word,
line, document, and `gj`/`gk` display-line motions; `i/a/I/A/o/O`; `d/c/y` with motions or
doubled linewise operators; `x/s/r/p/P/u/Ctrl-R`; and `/`, `n`, and `N` search navigation.
The unnamed register is shared across documents and mirrored to the system clipboard.

`terminal.shell = "auto"` selects the first detected shell for the current platform. Detection
covers `SHELL` and common macOS/Linux shells, plus `COMSPEC`, PowerShell, `cmd.exe`, and shells
available on `PATH` on Windows. Add executable paths or command names through Settings; they are
stored in `terminal.custom_shells`, and selecting one stores it in `terminal.shell`.

## Theme TOML

Place user themes in `<app config>/themes/*.toml` and select one with `[theme].name` in
`settings.toml`.

```toml
name = "custom-dark"
mode = "dark"

[ui]
focus_ring = "#7aa2f7"
selection = "#3f5f9f"
```

`ui.selection` is the global text-selection color for inputs and code editors. It is independent
from `ui.focus_ring`; if omitted, it defaults to the resolved `ui.focus_ring` value for
backward-compatible existing themes.

## Icon Themes

Set `[theme].icon_theme` to an icon package directory, icon-theme family, or individual theme
name. The Appearance settings page offers **Icon theme** with **Built-in** plus all installed
package theme names. Choosing a theme saves `[theme].icon_theme` and immediately updates
project-tree, file-tab, and editor-header icons. Packages use Zed-compatible JSON and SVG paths:

```text
<app config>/themes/icons/<package>/
├── icon_themes/
│   └── <theme>.json
└── icons/
    └── *.svg
```

The resolver supports exact file names, dotted suffixes such as `eslint.config.js`, extensions,
named folders, generic open/closed folders, and expand/collapse chevrons. Invalid or missing
icons fall back to the built-in component icons. SVG paths are constrained to the selected package.

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

The project layout may omit `project.name`; the project directory name is used in that case.
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

Each tab accepts `startup = "lazy" | "eager"`. The field defaults to `lazy`; a lazy tab starts
when it is first selected. An eager tab starts all of its panes when the project opens, even when
another tab is selected. The `default_tab` always starts because it is initially selected.

Example:

```toml
[project]
name = "yttt"
default_tab = "agent"

[[tabs]]
id = "dev"
title = "Dev"
startup = "eager"

[tabs.layout]
type = "split"
direction = "horizontal"
ratio = 0.65
left = { type = "pane", id = "server", title = "server", command = "npm", args = ["run", "dev"], execution_mode = "command", exit_behavior = "auto_restart" }
right = { type = "pane", id = "shell", title = "shell", command = "", execution_mode = "shell", exit_behavior = "manual_restart" }

[[tabs]]
id = "agent"
title = "Agent"
layout = { type = "pane", id = "codex", title = "Codex", command = "codex", execution_mode = "command", exit_behavior = "manual_restart", kind = "agent", notify_on_exit = true, detector = "codex" }
```

`execution_mode = "shell"` starts a persistent interactive shell and queues the pane `command`
as shell input. When that command finishes, the shell remains open at its prompt; an empty command
just opens the shell. Shell syntax, expansion, aliases, and functions apply, while the separate
`args` field is ignored. On Unix, supported shells start as interactive login shells.

`execution_mode = "command"` loads the user's shell environment on supported Unix shells, then
replaces that shell with `command`. `args` keep their argument boundaries and receive no shell
expansion. The pane process exits when the command exits. Command stdout and stderr remain
connected to the pane PTY.

`exit_behavior` accepts `close`, `auto_restart`, or `manual_restart` and applies when the pane
process exits. In shell mode that means the persistent shell itself, not each command run inside
it. Use command mode for services that must close or restart when their command exits. Automatic
restarts wait 500 ms before starting a fresh PTY. Existing layouts may omit `args`,
`execution_mode`, and `exit_behavior`; their defaults are `[]`, `shell`, and `close`.

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
layout = { type = "pane", id = "shell", title = "Shell", command = "" }
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
layout = { type = "pane", id = "shell", title = "Shell", command = "" }
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
`settings.notifications` persists the intended native-notification preference, but the
platform notifier is currently a no-op placeholder.

User-killed agent exits are not reported as failures.

## Manual Smoke Checklist

Run these before marking a product phase complete:

- `cargo run` opens an empty workspace.
- `cargo run -- /path/to/project`, `cargo run -- --project /path/to/project`, and
  `YTTT_OPEN_PROJECT=/path/to/project cargo run` open a project.
- `YTTT_DEV_FIXTURE=1 cargo run` shows the development fixture.
- `YTTT_DEV_FIXTURE=agent-exit cargo run` produces an agent completion toast.
- On macOS, `scripts/run-dev-app.sh --fixture dev` creates and opens
  `target/dev-app/yttt.app` for UI-tool-friendly smoke testing.
- On macOS, `scripts/run-dev-app.sh --fixture agent` opens the agent exit fixture
  through the same `.app` wrapper.
- On macOS, `scripts/build-macos-dmg.sh` creates `target/macos/yttt.dmg`.
- On Windows, `scripts/build-windows-installer.ps1` creates the Inno Setup installer.
- On Linux, `scripts/build-linux-tar.sh` creates the versioned binary and desktop-integration
  tarball.
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
- Open a supported source file; its breadcrumb trail follows the cursor, and selecting a
  breadcrumb moves the cursor to that symbol.
- Fold and unfold a multiline structural region from the line-number gutter; the header and
  closing line remain visible.
- Press `⌘F` on macOS, `Ctrl+F` elsewhere, or select the **Find** toolbar control in a file tab;
  the in-file search control opens and navigates between matches.
- Manual save and both autosave modes write files.
- External modification and deletion show the appropriate conflict choices.
- Creating, modifying, or deleting a file outside yttt updates the active project's file tree
  and Git status without reselecting the project.
- Dirty file, dirty project with running terminals, and window close all protect unsaved data.
- Pane focus can be changed from keyboard and pointer.
- Invalid layout TOML produces a visible error.
- Closing a project with running panes asks for confirmation.

## Known Limits

- The project tree and text editor are not a general-purpose file manager or full IDE.
- File editing is limited to regular UTF-8 files up to 10 MiB. Continuous filesystem watching is
  limited to the active project; inactive projects refresh when selected.
- No client/server terminal runtime.
- No live process restore after restart.
- No output parser for agent internal state.
- No GUI layout editor.
- Native system notifications and notification click routing are not implemented.
- macOS packages are ad-hoc signed; Developer ID signing and notarization require release
  credentials.
