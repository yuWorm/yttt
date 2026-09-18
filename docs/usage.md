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

The first launch opens onboarding before the empty workspace. Choose the interface language and
terminal font, then one of two default layout types:

- **Split view:** one tab with the coding agent on the left and an interactive shell on the right.
- **Separate tabs:** one agent tab and one shell tab.

Next choose the built-in coding agent: Codex (`codex`), Claude Code (`claude`), Grok (`grok`;
the `groky` fork is recognized as the same provider), OpenCode (`opencode`), Pi (`pi`), or Oh My
Pi (`omp`). The command palette remains available from both onboarding steps through its configured
shortcut and the visible Command Palette action.

Language selection previews the interface without writing configuration. Completing onboarding
requires Host control, saves the selected Host layout and Agent, then writes the Device language
and `general.onboarding_completed = true`. Configuration reads alone never create defaults or
install hooks. Projects opened afterward inherit the Host default unless they provide a project
or personal layout override.

Normal application launches remain local. The bottom status bar identifies the current
environment; connecting a remote service never replaces the current local window.
Use **Remote services** on the homepage to open one management window with two tabs:
**Remote connections** and **Remote access to this computer**. The first tab lists saved SSH
and network Host connections together. Click a row to connect; use its **⋯** menu to edit or
delete it. **Add connection** opens a type-specific SSH or network Host modal in the same window.
Forms scroll independently of their fixed bottom actions; **Escape** or **Cancel** discards
unsaved input and returns to the list. **Save** only updates the record; **Save and connect**
explicitly does both. Missing credentials open a small credential-only prompt instead of the
full editor. The workbench stays usable behind the manager; connecting opens an independent Client window.
The connecting window inherits the launching window's theme, UI style, fonts, window effect,
and language before its first frame, including imported Zed themes. Progress, SSH fingerprint
verification, takeover decisions, retry, and cancellation use the same workbench controls.
**Connection details** expands environment and Client identifiers; the bottom actions stay visible
when the content scrolls. Normal takeover uses **Continue here**; only forced takeover is marked
as destructive. Interface text is available in English and Chinese; underlying diagnostic errors
remain in their original language under **Technical details**.

With **Restore last session** enabled (the default for new preferences), local and remote
windows restore the Host's confirmed workspace before starting terminal/Agent views. This
includes dynamic terminal tabs, file tabs, split layout and active work items—not just the
project list. Multiple saved workspaces restore separately; a confirmed empty workspace stays
empty even when recent-project history exists. Disable the setting to start at the initial
project menu, where **Restore Last Session** explicitly loads a saved workspace through the
same restoration path. Opening a directory or a new empty window remains a separate action.

Surviving Host processes are reattached, not duplicated. After a cold Host restart, previously
running shells are recreated without replaying their startup commands, and saved Agent sessions
use the provider's resume command, including previously started lazy tabs. Workspace restoration
also resumes saved Agent sessions whose panes were already marked exited, including Agents
started inside a shell. This happens during Host resource reconciliation, not immediately on
process exit. Failed or unavailable resume retains the original session and tab rather than
silently starting a fresh conversation; exited Agents without a saved session remain stopped.
Other command processes remain stopped until explicitly started; observers never spawn them.

If a terminal launch loses its response, the pane shows **Reconciling** with **Retry**, rather
than claiming that the command failed. Retry preserves the original launch identity; it does not
start a second copy of that command. The client checks the Host catalog and reattaches when the
terminal is present. If the result can no longer be established, the pane remains unresolved.
A changed Host epoch ends the old attempt without replaying it; a subsequent explicit start is a
new attempt. Ordinary confirmed failures and exits use **Restart** instead.

If an attached session disappears from the Host catalog, the pane shows **Terminal session
unavailable**, not a launch failure. **Reconnect** only attaches to an existing session; it never
starts another shell or Agent. The pane keeps listening and reconnects when the same session
address becomes available again.

The legacy `terminal-placements.json` file is no longer read or written and can be left untouched.
Its contents or revision cannot block terminal startup. Resource protocol v9 requires matching
client and Host updates. A Host retains at most 4,096 launch-attempt records for its lifetime;
at capacity it refuses new launches instead of forgetting old attempts and risking duplicate
execution. It never restarts itself or interrupts active jobs to clear this limit.

For onboarding development or demos, force the flow even after it has been completed:

```sh
YTTT_FORCE_ONBOARDING=1 cargo run
```

`YTTT_FORCE_ONBOARDING` accepts `1`, `true`, `yes`, or `on` case-insensitively. The override does
not clear the persisted completion marker; every normal launch remains forced only while the
environment variable is enabled.

## Desktop and Host Lifecycle

The desktop UI and the headless Host are separate processes. Closing the last production window
keeps the desktop tray/menu-bar control plane available; it detaches views but does not terminate
Host-owned terminals, projects, SSH connections, or Agent jobs. A second invocation for the same
profile forwards its open request to the existing desktop shell instead of creating another tray.

On macOS and Windows, the tray menu exposes **Open yttt**, **New Window**, **Open Logs**, current
Host terminal/client/job counts, **Start Host**, **Stop Host If Idle**, **Restart Host If Idle**,
**Quit Desktop**, and **Quit All**. Safe stop and restart return `Busy` while resources are active;
they never kill those resources. **Quit Desktop** uses the Host's actual lifetime: it stops a
`DesktopOwned` Host, its network listener, terminals and Agent tasks, but only disconnects from
an `Independent` Host. **Quit All** explicitly stops either kind. The confirmation explains the
consequences; cancellation leaves the desktop owner connected. Confirmed workspaces and drafts
survive Host shutdown.

Linux and environments without a usable tray retain the same control path through CLI commands:

```sh
yttt --host-status
yttt --start-host
yttt --stop-host
yttt --restart-host
yttt --force-stop-host
yttt --open-logs
```

These commands are profile-scoped and use the authenticated lifecycle protocol; only the explicit
force-stop command may terminate a busy Host. Development builds use executable-scoped runtime and
credential namespaces, preventing them from attaching to an installed production Host.

### Start Host at Login

**Settings → Permissions → Background Host** exposes an opt-in **Start Host at login** switch.
The first enable opens an explicit confirmation describing the background Host access. Installation
never registers a startup item, and disabling the switch removes future login startup without
stopping an already-running Host.

The registration is per-user: macOS 13+ uses the bundled `SMAppService` LaunchAgent, Windows uses
the current user's `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` key, and Linux prefers a
systemd user unit with XDG autostart as the fallback. The registered command contains only the
executable, `--start-host`, and the stable production profile ID. Authentication tokens, credential
paths, environment values, and other secrets are resolved by the profile-scoped launcher at runtime
and are never written into the startup item.

Tray-independent status and registration controls are also available:

```sh
yttt --login-startup-status
yttt --enable-login-startup --confirm-remote-access
yttt --disable-login-startup
```

Registration is supported only by packaged, persistent production builds. macOS can report
**Approval required** until the user approves the item in System Settings. Real registration tests
must run only in a disposable environment. On macOS, the default smoke copies the supplied app to
a uniquely identified, ad-hoc-signed temporary bundle, verifies Host startup, replaces that bundle
in place to verify update continuity, then unregisters and removes it. The disposable OS account
provides the isolated production profile required by registration. On other immediate-start
backends, the same command requires registration to make a previously stopped Host reachable:

```sh
YTTT_DISPOSABLE_LOGIN_STARTUP_SMOKE=1 \
YTTT_LOGIN_STARTUP_SMOKE_CONFIG_HOME=/tmp/empty-disposable-yttt-config \
  scripts/run-login-startup-smoke.sh /path/to/yttt.app/Contents/MacOS/yttt
```

Windows Run entries and Linux XDG autostart execute only after login. Verify those paths across a
real sign-out/sign-in with a state file on persistent storage:

```sh
YTTT_DISPOSABLE_LOGIN_STARTUP_SMOKE=1 \
YTTT_LOGIN_STARTUP_SMOKE_PHASE=prepare \
YTTT_LOGIN_STARTUP_SMOKE_STATE_FILE=/persistent/path/login-startup-smoke.state \
  scripts/run-login-startup-smoke.sh /path/to/packaged/yttt

# Sign out and back in to the same disposable OS account.
YTTT_DISPOSABLE_LOGIN_STARTUP_SMOKE=1 \
YTTT_LOGIN_STARTUP_SMOKE_PHASE=verify \
YTTT_LOGIN_STARTUP_SMOKE_STATE_FILE=/persistent/path/login-startup-smoke.state \
  scripts/run-login-startup-smoke.sh /path/to/packaged/yttt
```

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

### Connect to an existing desktop Host

Use this path to access computer A's already-running yttt from B without deploying another Host:

1. On A, open **Remote services → Remote access to this computer**. Access is off by
   default; the initial address is `127.0.0.1:43123`. Enable only after the work windows have
   published their state. A port conflict leaves the listener disabled.
2. Copy connection information on A and transfer the resulting Base64 connection code privately.
   It includes the listening address, Host certificate, stable environment/profile identity and
   a work credential—not A's local administration token. Base64 is not encryption.
3. Forward A's listening TCP port using a general-purpose TCP tunnel. Keep the loopback bind when
   the forwarder's endpoint runs on A. Binding another interface requires explicit confirmation
   and appropriate firewall protection.
4. On B, open **Remote services → Remote connections → Add connection → Network Host**.
   The **Connect to existing yttt** command-palette action opens the same modal.
   Give the connection a name and paste A's code into **Connection code**: the address and
   authentication information are filled automatically. The decoded payload is limited to 8 KiB.
   Replace the address with one reachable on B when needed (for example a forwarded
   `127.0.0.1:54321`, or A's LAN address instead of its loopback/wildcard listening address).
   The code stays masked. **Save** adds a record without connecting; **Save and connect** also
   opens the Client. Saved names and routes can be edited without re-entering credentials.
   Optional remembered credentials use the OS keychain; a keychain error does not fall back to
   plaintext storage. Previously saved credentials remain usable. Clicking a saved record with
   missing or unavailable credentials prompts only for a code matching that Host's environment.
5. The separate Client verifies TLS 1.3, the imported certificate and environment before using
   the work credential. The address is a route, not Host identity: a forwarded `localhost`
   address is valid. No SSH login, binary deployment, or Server startup happens on this path.
6. Choose observation or profile-wide control. All saved work windows restore under stable
   workspace IDs, while B's original local windows remain attached to B.

**Ownership and handoff.** Host execution settings, layouts, files and published editor drafts
belong to A's Host. Appearance, themes, keybindings and other Device preferences remain on B.
Device administration, login-startup consent and credentials are not shared configuration.
Controller/observer authority is independent of local/remote connection type: an observer can
change Device preferences but cannot write Host or Project settings. One authenticated Client
session controls the entire profile across all its windows. During normal handoff the old Client
freezes shared editing and terminal input, publishes all windows, then relinquishes control.
A workspace or settings save failure cancels handoff.
The five-second deadline never automatically grants control: **Force takeover** explicitly chooses
the last durable state. The former controller remains an observer; reconnecting does not replay
old mutations or automatically regain input authority.
After an explicit control handoff back to this Client, existing terminal panes reacquire their
input leases and refresh their writer contexts by reattaching. The running processes are not
restarted, and input and terminal-close operations use the renewed authority.

**Recovery limits.** Draft bodies are separate from layout manifests: up to 6 MiB per document,
64 MiB per workspace and 1 MiB per manifest. An oversized or failed publication remains an error;
unpublished edits are not advertised as saved. Host restart restores confirmed windows and drafts,
then reconnects or rebuilds shells and Agent sessions as described above; arbitrary commands are
not automatically replayed.

Forced control loss, disconnection, or a stale Host epoch preserves unpublished edits in
Device-private recovery storage, keyed by Device profile, Host environment and workspace.
**Recover local drafts** restores matching editor drafts explicitly; it does not replace the
Host workspace wholesale. Failed Host/Project settings retain their candidate and confirmed
baseline for **Retry**, **Copy**, or **Discard**. Retrying requires current control and rejects a
changed baseline instead of replaying stale settings.

**Local management.** A can reclaim control, disconnect all TCP sessions, change the address, reset
credentials or disable access. Closing the management window or a work window does not close the
listener. Disable and reset revoke every TCP channel but preserve existing tasks and do not affect
the separate SSH work entrance. If persisting a disable fails, the effective closed state and the
unsaved preference are reported separately; the previous preference may apply after restart.
Remote windows cannot manage A's listener or login startup.

Quitting A's desktop-owned Host stops remote access and running tasks after confirmation.
Closing B's remote windows only detaches B. See [Desktop and Host Lifecycle](#desktop-and-host-lifecycle)
for independent background Hosts. TCP forwarding was exercised with a generic local byte forwarder;
this is not certification of any particular third-party tunnel product.

### SSH projects

Manage SSH records in **Remote services → Remote connections**. Choose **Add connection → SSH
connection** to create one, or use a row's **⋯ → Edit** action. Clicking the saved row connects
without saving any editor input. Password authentication prompts when no stored password is
available; encrypted configured private keys prompt for a temporary passphrase.

Run **Open SSH Project** from the command palette, the empty-workbench action, or the project
sidebar menu. This opens a dedicated picker instead of the SSH settings page:

1. Select a saved connection, or choose **New connection**.
2. Enter the host, port, user, optional starting root, and authentication details.
3. Accept or reject an unknown server fingerprint.
4. Continue in the separate remote Client, browse Host directories, and open a project.

The remote directory picker starts in the **Host user's home directory**, not the local Client's
home, and includes hidden folders such as `.config`. The compact picker shows the connection
name/endpoint, path input, **Open Current Folder** action, and directory rows.

- Click a folder to enter it; `..` returns to its parent.
- Use `Up`/`Down` to select a row and `Tab` to enter the selected directory.
- Type an absolute path or `~/…` to filter directory names by their case-sensitive prefix.
  For example, `/Volumes/WorkSpace/Pro` browses `/Volumes/WorkSpace/` and shows matching
  directories such as `Projects`. A trailing `/` lists that directory's children.
- The first match is selected automatically; `Enter` or `Tab` enters the selected directory.
  With no prefix, selecting the first row and pressing `Enter` opens the current folder as a project.
  With no matches, `Enter` does nothing; **Open Current Folder** still opens the browsed parent.
- `Escape` closes the picker. **New folder** creates the path entered in the Host picker.

Filtering within the same parent reuses the loaded directory list; it does not recursively search
the Host. The input remains editable during loading, and incoming results preserve your typed prefix.
Directory symlinks appear alongside ordinary directories and can be browsed or selected as project
roots. File links, broken links, and unresolvable cycles are not offered as directories.

Authentication modes:

- **Auto**: try SSH agent, then the configured private key, then a previously saved password.
- **Agent**: authenticate only with identities from the SSH agent.
- **Private key**: load the configured key file and optional masked passphrase. The passphrase is
  used only for the current attempt and is not persisted.
- **Password**: use the password exactly as entered; **Remember password** writes it to the
  operating-system credential store, never to `ssh-connections.toml`.

Connecting verifies the server key against yttt's `ssh-host-keys.toml`. Unknown or changed keys
require explicit approval; OpenSSH's `known_hosts` is not modified. Saved endpoint metadata and
OS-keychain credentials remain local. The remote window always labels its SSH endpoint.

yttt deploys the matching standalone `yttt-server` release into the remote user's
`~/.local/share/yttt/server/<version>/<platform>-<architecture>/` directory. Linux and macOS on
x86_64/aarch64 are supported. SSH must permit direct Unix-socket forwarding. Release downloads
are checked against `SHA256SUMS`; the Host itself has no GUI dependency or default public listener.
The Server descriptor exposes only a dedicated `work.sock` and work token. Its local administration
socket and token are separate; even a same-user SSH work connection cannot become a desktop owner
or invoke device administration.
An incompatible idle Host can be replaced; an incompatible busy Host refuses upgrade with its
resource blockers instead of killing work.

The remote Host owns execution configuration, default and personal layouts, project `.yttt` files,
Git, recursive project watching, terminal PTYs, Agent hooks and Agent history. **Open Project**
inside a remote Client browses the Host filesystem, including directory creation; it never opens a
local native folder dialog. Device theme and keybinding changes stay on this computer.
Manage SSH endpoints and Host-private administration from the local Client.

Remote workspace snapshots include project/tab/group/pane layout, active selection, tree state,
editor state and unsaved drafts. A save is acknowledged only after durable Host persistence.
Revision conflicts and disk failures remain visible rather than claiming success; closing with
an unconfirmed workspace save requires keeping the window open or explicitly discarding that save.

A second Client can restore the same environment after explicitly taking control. The old Client
then loses mutation authority, including terminal input, file/config writes and workspace commits.
Disconnecting or exiting a remote Client leaves Host processes alive. A Host or machine restart
cannot resurrect the old PTY: the controlling Client retains its tab and layout, recreates a clean
shell or resumes a saved Agent session, and leaves other commands stopped for **Start a new process**.

Legacy recent SSH entries remain available and launch the new remote Client using their saved
endpoint/root. They are skipped during local workspace auto-restoration; no local settings or
old mixed-environment layout is silently copied to the remote environment.

### Code navigation, folding, and search

The editor resolves its language from the explicit editor setting, file name, or file
extension. Language-aware highlighting includes plain text, TOML, JSON, JSONC, YAML, Markdown,
Bash, C, C#, C++, Fish, GDScript, Go, Java, JavaScript/JSX, Kotlin, Lua, PHP, PowerShell, Python,
Ruby, Rust, Scala, Swift, TypeScript/TSX, HTML, Vue, CSS, SCSS, Dockerfile/Containerfile,
HCL/Terraform, Nix, XML, and Zig.
Windows development files such as `.csx`, `.ps1`, `.psm1`, `.csproj`, `.vcxproj.filters`,
`.props`, `.targets`, `.xaml`, and `.sln` are detected by extension. The default icon theme
distinguishes C#, PowerShell, XML, and Windows solution/project files; matching icons from an
imported icon theme take precedence.

Vue single-file components highlight template directives and interpolations, JavaScript,
TypeScript, JSX, or TSX script blocks, and CSS or SCSS style blocks. Use quoted `lang`
attributes to select an embedded language, such as `<script setup lang="ts">` or
`<style scoped lang="scss">`; omitted `lang` defaults to JavaScript or CSS.
Pug templates, Less, and indented Sass are not registered preprocessors. Nix highlighting
covers Nix source, not dynamically selected embedded languages.

For languages with a symbol extractor, cursor-following breadcrumbs show the enclosing symbols
above the document. Symbol parsing runs in the background after a short typing pause; until the
current snapshot is ready, navigation uses the last completed outline. Select a breadcrumb to
move the cursor to that declaration. Multiline
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
- The maximum file size is 6 MiB.
- Canonical paths must remain inside the local or configured remote project root.
- Directory symlinks can be expanded when their targets remain inside the project root.
  The tree keeps the link-relative paths; ancestor cycles and broken targets report an error
  instead of recursively expanding. Deleting a link removes the link, not its target.
  To use a directory link targeting outside the current project, open it as a separate project.
- Active projects are watched recursively by their Host. Create, modify, and remove events
  refresh expanded tree directories, open-document disk state, and Git status.
- Create, rename, delete and copy/move operate within one Host environment. Local and remote
  Clients do not perform implicit cross-environment file transfers.

## Settings Window

Open Settings with `Cmd+,` on macOS or `Ctrl+,` on other platforms. Settings opens in its own
native window, with a searchable category sidebar and a scrollable content area. Repeating the
command focuses the existing window instead of opening a duplicate. Closing and reopening it
retains the search and selected category for that workbench session.

Use the native close control or `Cmd+W` / `Ctrl+W` to close the settings window without closing
workbench tabs. Settings navigation, shortcut recording, and nested dialogs stay in this window;
Global Vim settings navigation does not switch tabs in the background workbench.

**Edit Default Layout** and **Edit Project Layout** open the TOML file in a separate editor window
with its filename in the title bar. Use **Save** or `Cmd+S` / `Ctrl+S` to validate and save.
Invalid TOML stays open with an error below the editor and leaves the file unchanged. Closing
Settings does not close this editor or discard its buffer. **Cancel**, the native close control,
or `Cmd+W` / `Ctrl+W` discards unsaved edits; Escape leaves the editor open.

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

## Configuration Targets

Settings are organized by **feature**, not by storage target. Agent, Terminal, Editor and the other
categories remain discoverable together. Search covers all categories and matches configuration
keys, localized labels/descriptions and English names, including in the Chinese interface.
For example, searching `terminal.shell` takes you directly to the default shell control.

The information strip below the category heading identifies the current **Environment** and
**Project**. Local development/test environments have readable labels. Expand **Configuration
details** for the profile ID and aligned configuration paths. Long paths truncate within the
available width; hover a path's copy button to read it in full, or click to copy the complete path.
Settings actions use the same compact filled buttons in the Zed style. Each setting describes
its destination, application timing and any read-only restriction alongside the control:

| Destination | Contents | Storage and authority |
| --- | --- | --- |
| Local preferences | Appearance, themes/icons, fonts, keybindings, Vim, notifications, editor presentation/autosave, UI preferences | `<local-profile-config>/device`; editable by controllers and observers, including while disconnected |
| Environment | Shells/environment, scrollback, keyboard protocol, new-tab commands, Agent defaults, editor language/tab defaults and LSP, default layouts | Owning Host profile, shared by its connected clients; requires a connected controller, with no transfer in progress |
| Project | `editor.tab_size`, `editor.auto_detect_language`, `editor.default_language`; project layouts have their own existing format | `<project>/.yttt/settings.toml`, or the Host profile's isolated overlay; requires control and a writable project-config policy |

Under an overridable Editor or Languages setting, the named project's summary shows whether it
uses the environment default or a project value. **Customize for this project** expands a separate
project control; changing the environment control above it still changes the environment default.
Expanding the project control or leaving its value unchanged does not create an override.
Project controls are bound to the displayed project and snapshot generation, so stale input cannot
write to a newly selected project. Shell and Agent settings do not gain project overrides.

Unavailable environment settings remain visible with the reason they are read-only. Disconnected
values are labeled as last known; local preferences remain editable. Retained drafts still use
**Retry**, **Copy**, and **Discard**, with retry authority determined by the draft's actual destination.

**Restore environment default** removes the project override and restores inheritance. Editor tab size
and language settings apply to newly opened/reopened files; shell and Agent launch settings apply
to new sessions. Existing file contents and running processes are not replaced.

`<app config>` below means the owning Host's original profile config directory.
`<device config>` means `<local-profile-config>/device`, never a directory on the connected Host.
On first binding, local legacy Device fields, themes, icons, keybindings and bars are copied into
the Device root. Existing standalone bars take precedence over embedded legacy bars. Migration
does not rewrite Host files, and a completion marker prevents deleted Device overrides from being
reimported on every launch.

Missing configuration uses in-memory defaults. Opening settings or a layout editor does not
create files; explicit saves, imports, onboarding completion, and controlled Agent initialization
are the write boundaries. Malformed files and I/O failures remain visible.

Concurrent Clients serialize Device `settings.toml` saves with a local file lock and reject drafts
whose confirmed Device values no longer match disk. Unreadable files are not overwritten.
A rejected candidate remains available for **Copy**, **Retry**, or **Discard**; retrying never
forces an overwrite. After a conflict, copy any changes you want to keep, discard the old draft,
reload the current settings, and apply those changes again.

## Settings TOML

Host fields live in `<app config>/settings.toml`; Device fields live in
`<device config>/settings.toml`. The following illustrates the merged defaults, not a single
file to copy into both destinations:

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

Window Bar and Status Bar configuration lives in the standalone `<device config>/bars.toml` file.
This file is independent of `settings.toml`, so a complete bar layout can be copied, versioned, or
shared without carrying unrelated application preferences. These are the complete defaults:

```toml
[window]
left = '[project-name] [|] [project-path] [|] [git-branch] [|] [git-changes]'
center = ''
right = '[projects-count] [terminals-count] [tabs-count] [editors-count] [app-cpu] [app-memory] [system-cpu] [system-memory] [command-palette] [settings]'

[status]
enabled = true
left = '[vim-mode] [surface] [vim-detail] [active-item]'
center = '[vim-keys]'
right = '[editor-language] [editor-position] [editor-dirty] [editor-diagnostics] [git-branch] [git-changes] [agent-state] [ssh] [update]'
```

During the one-time local Device migration, a legacy `[bars]` section supplies defaults only when
there is no standalone `bars.toml`. The standalone layout is preserved without rewriting the
legacy Host settings file.

Editor font family, font size, line height, soft wrap, and line numbers update all open files
without replacing their text or saved baseline. `vim.mode` accepts `"global"`, `"editor"`, or
`"disabled"`. Global mode uses one window-level Vim state across project editors, terminals,
project trees, settings, panes, tabs, and palettes; Editor mode limits Vim to project editors.
Each editor still keeps its own cursor and selection state. `tab_size` applies to files opened
after the change; reopen an existing file to apply it. Changing autosave to `off` cancels pending
delayed saves. `default_open` affects new project sessions. Editing `width` updates the selected
project and the default for future projects, while other open projects retain their own widths.
Valid width ranges are 200–520 px for the right tree and 160–420 px for the left sidebar.

Open **Settings → Appearance → Window & status bars → Edit bars TOML** to edit both bars
in the dedicated text editor. Its searchable component directory inserts into the selected
left/center/right region; the preview uses current application data without changing the live bars
or executing actions. Save (Cmd-S on macOS, Ctrl-S elsewhere) validates and persists Device
preferences; Cancel or closing the editor discards unsaved changes. Opening or previewing does
not create a file. Device bar preferences remain editable without shared Host control.

The component directory groups Project, Editor & Vim, Terminal, Agent, Performance, and
Layout & actions entries. Search accepts module IDs and localized names/descriptions. Each entry
shows its template syntax and a description; preview hints explain hidden components, such as
an absent editor, an unreported Agent model, a clean working tree, or unavailable performance samples.
The source is parsed when edited, while runtime values continue to refresh in the preview.

**Recommended**, **Minimal**, **Development**, and **Agent** presets replace the six region
templates and status-bar visibility in the draft, retaining valid per-module overrides.
**Recommended** uses the built-in defaults below. **Minimal** keeps the status bar enabled for
Vim mode, Agent waits, editor diagnostics, and terminal exits. **Development** extends Recommended
with editor tab width and soft-wrap state, without performance metrics. **Agent** emphasizes the
active pane's model, active children, state duration, waits, and terminal exit; project-wide Agent
state remains in the window bar.

**Restore Defaults** resets the whole draft, including overrides. Both presets and defaults can
recover an invalid draft; neither writes a file nor changes the live bars until Save.
Cancel discards these changes.
 
The built-in layout keeps project identity and Git information at the top, with mode, active-pane
details, and attention/error cues at the bottom. It omits performance metrics, counters, and
duplicated paths or tab names:

```toml
[window]
left = '[project-name] [Space: 2] [git-branch] [git-changes]'
center = ''
right = '[agent-state] [Space: 2] [update] [command-palette] [settings]'

[status]
enabled = true
left = '[vim-mode] [vim-detail] [vim-keys] [Space: 2] [agent-waiting] [agent-children]'
center = ''
right = '[editor-language] [editor-position] [editor-selection] [agent-model] [Space: 2] [editor-diagnostics] [terminal-exit]'
```

These defaults apply when no bar preferences exist. Existing explicit templates, empty regions,
status visibility, and per-module overrides are preserved; new default components are not appended
to custom templates. Choose Recommended and Save to adopt the layout while retaining overrides.

Both bars use independent `left`, `center`, and `right` template strings. Project identity and Git
information are ordinary configurable modules; an explicit empty string hides that region.
Native window controls and the essential local/remote Profile control surface are not template
items. `status.enabled = false` hides configured status content but retains the Profile control
surface while connected to a Host.

Legacy module arrays are read without rewriting the file. Legacy window layouts inherit the former
fixed identity prefix; saving writes template strings only. Repeated modules are allowed.

```toml
[status]
right = '[app-memory] [Space: 5] [text:ssssss] [icon:settings] [|] [update]'
```

- `[module-id]` renders a dynamic module with its existing icon, tooltip and action.
- `[Space: number]`, for example `[Space: 5]`, inserts 1–256 font-relative space widths.
  Change the integer to adjust the gap in the live preview; Save applies it to the actual bars.
  Explicit spaces replace the default gap at that position rather than adding another gap on
  either side. `[Space]` and `[Space*N]` are no longer accepted; replace them with `[Space: 1]`
  and `[Space: N]` respectively.
- `[text:literal text]` preserves text and internal whitespace. Use `\[`, `\]`, and `\\`
  for literal brackets and backslashes; TOML single-quoted strings avoid double escaping.
- `[icon:name]` is a static icon, not an action. `[icon:settings]` is decorative;
  `[settings]` opens Settings.
- `[|]` is a vertical separator. Leading, trailing and redundant separators disappear when
  adjacent dynamic modules have no data. A region containing only spaces/separators collapses.
- Whitespace outside tokens is for readability only. Unknown modules/icons, invalid escapes,
  unclosed tokens and invalid space counts are errors.

Available static icon names: `settings`, `info`, `cpu`, `memory-stick`, `search`, `palette`,
`folder`, `folder-open`, `file`, `square-terminal`, `github`, `network`, `globe`, `user`, `bot`,
`bell`, `calendar`, `chart-pie`, `hard-drive`, `battery`, `triangle-alert`, `circle-check`,
`circle-x`, `play`, and `pause`. Icons are bundled assets; arbitrary paths and URLs are not accepted.

Application CPU/memory describe the local GUI process, not remote Host or terminal subprocesses.
System metrics describe this device. A single sampler shared by all workbench windows collects
application and system CPU/memory about once per second in the background, even when no metric
components are displayed. It retains only the latest sample, not a history.

Bar templates control display, not collection. Only windows displaying metric components and
bar-editor previews using them refresh on samples; unrelated windows do not refresh for sampling.
Unsaved previews read the same live cache immediately, without enabling a setting or saving first.
Closing a workbench window does not stop sampling for the remaining windows.

The former `general.performance_metrics_enabled` and `general.system_performance_metrics_enabled`
settings have been removed. Existing keys are ignored when loading, including `false` values;
the next Device settings save omits them. Existing bar templates remain unchanged.

Available module IDs are:

- Workspace: `project-name`, `project-path`, `active-item`, `surface`
- Vim: `vim-mode`, `vim-detail`, `vim-keys`
- Editor: `editor-language`, `editor-position`, `editor-dirty`, `editor-diagnostics`,
  `editor-selection`, `editor-tab-size`, `editor-wrap`
- Terminal: `terminal-title`, `terminal-state`, `terminal-exit`, `terminal-size`
- Agent: `agent-state`, `agent-waiting`, `agent-model`, `agent-children`, `agent-state-duration`
- Repository and runtime: `git-branch`, `git-changes`, `ssh`, `update`
- Performance: `projects-count`, `terminals-count`, `tabs-count`, `editors-count`, `app-cpu`,
  `app-memory`, `system-cpu`, `system-memory`
- Actions: `command-palette`, `settings`

The additional Agent modules describe the **active terminal pane**, not an arbitrary Agent in the
project. The existing `agent-state` remains a project-level aggregate:

| Module | Meaning |
| --- | --- |
| `agent-waiting` | Current wait reason; the full waiting message is available in the tooltip. Hidden when not waiting. |
| `agent-model` | Model reported by the active Agent session. Missing or blank models are hidden rather than inferred. |
| `agent-children` | Number of tracked child Agents currently working or waiting. Completed, interrupted, failed and idle children do not count; this is not a lifetime total. |
| `agent-state-duration` | Time since the active Agent's current state began, refreshed once per second while requested. Not total task duration; clock skew is clamped at zero. |
| `editor-selection` | Selected Unicode scalar characters and lines, excluding an unselected line after a trailing newline. Code and Markdown editors are supported; Markdown uses the corresponding source range, including any selected syntax. |
| `editor-tab-size` | Active document's configured tab width, not indentation detected from file contents. |
| `editor-wrap` | Active editor's soft-wrap setting. |
| `terminal-exit` | Final process exit code (when available) and exit reason. Hidden before exit and cleared when restarting. |
| `terminal-size` | Actual terminal viewport columns × rows, updated on resize rather than taken from the initial spawn settings. |

For example:

```toml
[status]
left = '[agent-waiting] [Space: 5] [agent-model]'
center = '[agent-children] [agent-state-duration]'
right = '[editor-selection] [editor-tab-size] [editor-wrap] [terminal-exit] [terminal-size]'
```

Modules without data for the active surface are omitted. Per-module width and empty-state behavior
can be overridden with a module table; width accepts 24–640 px:

```toml
[status.modules.active-item]
max_width = 320
hide_when_empty = true

[window.modules.app-memory]
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

Place user themes in `<device config>/themes/*.toml` and select one with `[theme].name` in
`<device config>/settings.toml`.

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

With `[theme].ui_style = "zed"`, an empty `general.ui_font_family` uses bundled **IBM Plex Sans**;
the Rounded style retains the system UI font. An explicit family overrides either fallback.
UI controls and picker geometry scale with `general.ui_font_size`. In the Zed style, terminal
row height is `terminal.font_size × terminal.line_height`; Rounded retains font-metric-based
row height. Terminal grid origins are snapped to device pixels in the Zed style.

Imported Zed themes resolve missing or `null` color roles from Zed's dark/light defaults.
Muted, placeholder, disabled, and icon colors remain independent, as do the title bar,
inactive title bar, toolbar, and status bar surfaces.

**Open SSH Project** groups recent remote paths and connection actions under each saved server.
Search matches server names, connection details, and paths while retaining the matching server
heading. Press `Escape` to close the picker without opening a connection.

## Icon Themes

Set `[theme].icon_theme` to an icon package directory, icon-theme family, or individual theme
name. The Appearance settings page offers **Icon theme** with **Built-in** plus all installed
package theme names. Choosing a theme saves `[theme].icon_theme` and immediately updates
project-tree, file-tab, and editor-header icons. Packages use Zed-compatible JSON and SVG paths:

```text
<device config>/themes/icons/<package>/
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
<device config>/keybindings.toml
```

Open its editor with the following command; the first explicit save creates a missing file:

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
- the configured command basename is one of the six onboarding Agents: `codex`, `claude`, `grok`
  (including the `groky` alias), `opencode`, `pi`, or `omp`; or
- a Host-owned shell pane has a live process-tree match for one of those commands. This last path
  detects an Agent started manually after opening a new tab or pane.

Host-owned local process discovery samples all pane roots in one shared monitor, recognizes native
executables plus the Node/Bun package paths used by the script-backed CLIs, and chooses the
nearest matching descendant. Two missed samples end the detected run, avoiding false completion
during launcher handoff while still detecting a hard-killed Agent whose parent shell remains alive.
Remote workspace panes use the remote Host's process tree, so the same discovery and hard-exit
cleanup apply without inspecting processes on the desktop machine.

All six built-in Agents have managed provider adapters. Hook provisioning is an explicit,
idempotent initialization step after the Client has connected with Host control; constructing an
Agent manager or attaching an observer never installs hooks. For configured command panes, yttt:

1. waits for successful Host-side provider initialization without replacing unrelated user configuration;
2. creates a stable Agent instance for the Project/Tab/Pane scope;
3. injects a per-launch instance ID, generation, and random authentication token; and
4. receives bounded events through the local hook server or authenticated terminal-title frames.

Initialization failure exposes a retry action. Ordinary shells and attachment to existing
Host terminal/Agent sessions do not wait for new Agent provisioning.
Grok's native hooks use `snake_case` event names. When Grok's Claude-compatibility loader also
re-exports yttt's Claude hook, the adapter discards that duplicate before HTTP delivery so the Host
receives one native Grok lifecycle stream.

Provider hooks and OMP extensions are installed in the active Host user's home directory.
Provider hooks, not process names or terminal text, are authoritative for the session,
active task, tool action, waiting reason, turn completion, and child-agent lifecycle. Process
discovery and start/exit events remain the fallback for startup, interruption, failure, and
manually launched Agents in either environment.

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
- File editing is limited to regular UTF-8 files up to 6 MiB. Continuous filesystem watching is
  limited to the active project; inactive projects refresh when selected.
- Remote Clients currently require SSH and a Linux/macOS Host; raw public-network Host listeners are not exposed.
- A desktop restart reattaches to a surviving Host, but a Host or machine restart cannot resurrect an existing PTY child process.
- Provider-level task and tool progress requires the managed hook or extension installed for the
  selected agent; commands without one retain process-level fallback status.
- No GUI layout editor.
- Native system notifications and notification click routing are not implemented.
- macOS packages are ad-hoc signed; Developer ID signing and notarization require release
  credentials.
