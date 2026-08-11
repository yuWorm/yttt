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

## Desktop Permissions

Open **Settings → Permissions** to inspect and request desktop access used by development
workflows. The page checks authorization when opened, refreshes after yttt regains focus from
system settings, and provides a manual refresh action. Notifications, protected file access, and
developer tools are listed as core access. Accessibility and screen capture are optional and
should only be enabled for workflows that control another application, synthesize input, or
capture a screen or window.

On macOS, yttt detects notification, Accessibility, and Screen Recording authorization through
their native APIs. The first request uses the native system prompt; denied or already-granted
access can be managed in the exact Privacy & Security or Notifications pane. macOS does not expose
a supported status API for Full Disk Access or Developer Tools, so those rows report that the
status is unavailable and open the corresponding pane directly.

On Windows, yttt opens the matching Settings page when Windows exposes one and identifies access
that native desktop applications do not need to request separately. Linux has no single permission
center, so the page identifies access managed by the desktop environment, requested by an XDG
Desktop Portal when used, or available without separate approval.

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
environment = {}
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
autosave = "off"
autosave_delay_ms = 1000
auto_detect_language = true
default_language = "plain_text"

[editor.lsp]
enabled = false
command = ""

[vim]
mode = "disabled"

[project_panel]
default_open = true
show_hidden = false
width = 280.0
project_sidebar_width = 216.0
```

## Bars TOML

Window Bar and Status Bar configuration lives in the standalone `<app config>/bars.toml` file.
This file is independent of `settings.toml`, so a complete bar layout can be copied, versioned, or
shared without carrying unrelated application preferences. These are the complete defaults:

```toml
[window]
left = ["project-name", "project-path"]
center = []
right = ["projects-count", "terminals-count", "tabs-count", "editors-count", "app-cpu", "app-memory", "system-cpu", "system-memory", "git-branch", "git-changes", "command-palette", "settings"]

[status]
enabled = true
left = ["vim-mode", "surface", "vim-detail", "active-item"]
center = ["vim-keys"]
right = ["editor-language", "editor-position", "editor-dirty", "editor-diagnostics", "git-branch", "git-changes", "agent-state", "ssh", "update"]
```

On first launch after upgrading, a legacy `[bars]` section in `settings.toml` is moved to
`bars.toml`. If `bars.toml` already exists, it remains authoritative and the legacy section is
removed without overwriting the standalone file.

Editor font family, font size, line height, soft wrap, and line numbers update all open files
without replacing their text or saved baseline. `vim.mode` accepts `"global"`, `"editor"`, or
`"disabled"`. Global mode uses one window-level Vim state across project editors, terminals,
project trees, settings, panes, tabs, and palettes; Editor mode limits Vim to project editors.
Each editor still keeps its own cursor and selection state. `tab_size` applies to files opened
after the change; reopen an existing file to apply it. Changing autosave to `off` cancels pending
delayed saves. `default_open` affects new project sessions. Editing `width` updates the selected
project and the default for future projects, while other open projects retain their own widths.
Valid width ranges are 200–520 px for the right tree and 160–420 px for the left sidebar.

Window Bar and Status Bar module order can be edited under **Settings → Appearance → Window &
status bars** or directly in `bars.toml`. Both bars use independent `left`, `center`, and `right`
arrays. The same Settings page shows the standalone file path.
Available module IDs are:

- Workspace: `project-name`, `project-path`, `active-item`, `surface`
- Vim: `vim-mode`, `vim-detail`, `vim-keys`
- Editor and terminal: `editor-language`, `editor-position`, `editor-dirty`,
  `editor-diagnostics`, `terminal-title`, `terminal-state`
- Repository and runtime: `git-branch`, `git-changes`, `agent-state`, `ssh`, `update`
- Performance: `projects-count`, `terminals-count`, `tabs-count`, `editors-count`, `app-cpu`,
  `app-memory`, `system-cpu`, `system-memory`
- Actions: `command-palette`, `settings`

Modules without data for the active surface are omitted. Per-module width and empty-state behavior
can be overridden with a module table; width accepts 24–640 px:

```toml
[status.modules.active-item]
max_width = 320
hide_when_empty = true

[window.modules.git-branch]
max_width = 180
hide_when_empty = true
```

The editor surface supports Normal, Insert, Visual, and Visual Line modes; counts; `h/j/k/l`,
word, line, document, and `gj`/`gk` display-line motions; `i/a/I/A/o/O`; `d/c/y` with motions or
doubled linewise operators; `x/s/r/p/P/u/Ctrl-R`; and `/`, `n`, and `N` search navigation.
Global mode additionally maps modal navigation and actions onto the opened-project list, terminals,
project trees, settings, panes, tabs, palettes, and dialogs. The compact shared Status Bar can show
the active mode, current surface, pending key sequence, editor position and diagnostics, Git,
agent, SSH, update, and performance state according to the configured module layout. The unnamed
editor register is shared across documents and mirrored to the system clipboard.

On a terminal surface, `i/a/I/A` enter Terminal mode and restore direct process input. `Escape`
and `Ctrl-[` are sent to the terminal process for shells and TUI applications; use
`Ctrl-\ Ctrl-N` to return to yttt Normal mode.

`Ctrl-W h/j/k/l` moves directionally across the left **Projects** list, work-area panes and groups,
and the right project tree. When Projects owns Global Vim focus, `j/k` and `gg/G` select opened
projects while keeping focus in the list.

When the project tree owns Global Vim focus, its current row is the operation target: `j/k` move
it, `h` collapses or selects the parent, and `l` expands, descends, or opens a file. Tree operations
use that keyboard-selected row immediately rather than waiting for the project model to refresh.

`terminal.shell = "auto"` selects the first detected shell for the current platform. Detection
covers `SHELL` and common macOS/Linux shells, plus `COMSPEC`, PowerShell, `cmd.exe`, and shells
available on `PATH` on Windows. Add executable paths or command names through Settings; they are
stored in `terminal.custom_shells`, and selecting one stores it in `terminal.shell`.

Global terminal variables can be added in Settings or declared by replacing the empty inline map
with a table:

```toml
[terminal.environment]
NODE_ENV = "development"
RUST_LOG = "yttt=debug"
```

Names must match the portable form `[A-Za-z_][A-Za-z0-9_]*`. Values override the environment
inherited by yttt and are injected into every subsequently started local or SSH shell and
command. Already-running processes keep their original environment until restarted.

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

`detector` remains reserved for terminal-output detectors. Provider-aware agent progress does not
scrape terminal text; it uses authenticated provider hook events.

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

The file stores sparse overrides over the built-in defaults. It reloads automatically after a
successful in-app save or external edit. A minimal configuration is:

```toml
schema_version = 6
leader = "space"

[[bindings]]
keys = "<leader> f"
command = "file.find"
context = "Workspace"

[[bindings]]
keys = "<leader> p"
command = "command_palette.open"
context = "Workspace"
```

`<leader>` expands to the top-level `leader` key. Multi-key sequences separate keystrokes with
spaces. Set `unbind = true` on an entry to remove its exact default assignment; user overrides
do not replace unrelated defaults. Contexts are GPUI predicate strings, and the settings editor
reports invalid commands, invalid sequences, and conflicting assignments before saving.

Directional focus is exposed through the configurable `pane.focus_left`, `pane.focus_right`,
`pane.focus_up`, and `pane.focus_down` actions. Despite their compatibility-preserving IDs, they
move across terminal panes, work-area groups, editors, and the project tree.

Global Vim project-tree defaults follow the core neo-tree workflow:

```text
j / k       next / previous visible entry
h / l       collapse-or-parent / expand-or-open
gg / G      first / last visible entry
Enter / o   open a file or toggle a directory
a / A       create file / directory
r / d       rename / delete
y / x / p   copy / cut / paste
z           collapse all directories
H / R       toggle hidden files / refresh
/           open the project file finder
q           hide the project tree
```

Each operation above has its own `project_tree.*` or existing command action and can be rebound or
unbound in Settings. Space remains the configurable Vim leader instead of shadowing it with
neo-tree's default toggle mapping; `project_tree.vim.toggle` is available for users who prefer that
assignment.

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

- `kind = "agent"` is set in layout TOML;
- the configured command basename is one of the five onboarding Agents: `codex`, `claude`,
  `opencode`, `pi`, or `omp`; or
- a local shell pane has a live process-tree match for one of those commands. This last path
  detects an Agent started manually after opening a new tab or pane.

Local process discovery samples all pane roots in one shared monitor, recognizes native
executables plus the Node/Bun package paths used by the script-backed CLIs, and chooses the
nearest matching descendant. Two missed samples end the detected run, avoiding false completion
during launcher handoff. SSH panes have no local process tree, so manual Agent discovery there
still requires a provider hook.

All five built-in Agents have managed provider adapters. For configured command panes, yttt:

1. creates a stable Agent instance for the Project/Tab/Pane scope;
2. installs the provider hook or extension without replacing unrelated user configuration;
3. injects a per-launch instance ID, generation, and random authentication token; and
4. receives bounded events through the local hook server or authenticated terminal-title frames.

OMP extensions are installed locally and bootstrapped under the remote user's home directory for
SSH panes. Provider hooks, not process names or terminal text, are authoritative for the session,
active task, tool action, waiting reason, turn completion, and child-agent lifecycle. Process
discovery and start/exit events remain the fallback for startup, interruption, failure, and
manually launched local Agents.

The normalized status model includes:

- `starting`
- `idle`
- `working`
- `waiting`
- `completed`
- `failed`
- `interrupted`
- `stale`

The project sidebar groups compact one-line Agent rows below each project. Each row shows a
status icon, an Agent-type icon, and `pane name — current task · current action`; overflow is
ellipsized instead of adding stacked metadata lines. Oh My Pi uses its OMP mark, child Agents use
the generic Agent glyph, and hovering either icon identifies it. Clicking an Agent selects its
project, tab, and pane. Project Agent groups can be collapsed; the collapsed project IDs and the
latest bounded Agent snapshots are persisted.

If yttt closes while a resumable Agent is still running, reopening the workspace restores that
session with the provider's native command: `claude --resume`, `codex resume`,
`opencode --session`, `pi --session <session-file>`, or `omp --resume`. Configured Agent panes keep
their command overrides; Agents detected inside a shell resume with the detected provider command.
An explicit provider session title is preferred, otherwise the first prompt supplies a stable
bounded title. The title survives restoration, while a custom pane title remains authoritative.
Normal CLI exit still removes a detected shell Agent instead of retaining it for restoration.

The managed transport never persists or logs the launch token or complete tool payload. Session
identity, model, resume path, title, and prompt summary are bounded before persistence. Unknown
instances, stale generations, and invalid tokens are rejected before provider event normalization.

In-app toast is always produced for agent exit events when `notify_on_exit = true`.
`settings.notifications` persists the intended native-notification preference, but the
platform notifier is currently a no-op placeholder. User-killed agent exits are not reported as
failures.

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
- Provider-level task and tool progress currently has a first-party adapter only for Oh My Pi;
  other Agent commands retain process-level fallback status.
- No GUI layout editor.
- Native system notifications and notification click routing are not implemented.
- macOS packages are ad-hoc signed; Developer ID signing and notarization require release
  credentials.
