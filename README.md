# yttt

`yttt` is a Rust desktop terminal workbench built with GPUI, `gpui-component`, and a
project-owned `yttt-terminal` crate based on `alacritty_terminal`.

The product direction is project-first and terminal-first, with lightweight project-file
editing built into the same workbench:

- Open local projects, connect to an existing desktop Host, or deploy an independent Host through SSH.
- Work in unified terminal and file tabs.
- Browse project files from a lazy tree on the right.
- Edit and save UTF-8 text files without leaving the terminal workflow.
- Keep terminals, files, configuration, drafts, and Agent tasks on their owning Host.
- Split panes inside a tab.
- Save personal layouts in the active environment.
- Export shareable project layouts explicitly.
- Track process-level agent CLI exits for tools such as Codex and Claude Code.

The desktop is a client of a profile-isolated headless Host that owns terminal, project, SSH, and
Agent resources. Closing the last window keeps the native tray/menu-bar control plane available
without terminating those resources; the tray can reopen windows, show resource counts, manage
the Host lifecycle, and confirm the consequences of quitting. Desktop-owned Hosts stop with the desktop;
explicitly started independent Hosts can outlive it. Environments without a tray can use the
equivalent `--host-status`, `--start-host`, `--stop-host`, `--restart-host`, and
`--force-stop-host` commands. Login startup is opt-in under **Settings → Permissions → Background
Host**, with per-user registration on macOS, Windows, and Linux. See
[Usage](docs/usage.md#desktop-and-host-lifecycle).

## Project Files and Editor

Each project keeps its own terminal/file tabs, active work item, file-tree expansion state,
tree visibility, and tree width. Switching projects preserves those sessions. The folder
button fixed at the end of the tab bar toggles the right project tree and shows an active
state while the tree is open.

The left project sidebar and right file tree can both be resized by dragging their inner
edges. File tabs provide language-aware highlighting, structural breadcrumbs, gutter folding,
and in-file search alongside manual save, focus-change autosave, delayed autosave,
external-change conflicts, and dirty file/project/window close protection. See
[Usage](docs/usage.md) for commands, settings, and detailed behavior.

Supported source highlighting includes Bash, C, C#, C++, Fish, GDScript, Go, Java,
JavaScript, Kotlin, Lua, PHP, PowerShell, Python, Ruby, Rust, Scala, Swift, TypeScript, XML,
and Zig.
Breadcrumbs follow the cursor through enclosing declarations; select one to move to that
declaration. Fold multiline structures from the line-number gutter—the header and closing line
remain visible while only the body hides. Open in-file search with the **Find** toolbar control,
`⌘F` on macOS, or `Ctrl+F` elsewhere.

### Editor Smoke Check

- In a supported multiline source file, move into a nested declaration and select its breadcrumb;
  the path must update and the click must move the cursor to the declaration.
- Fold and unfold a multiline structural region in the line-number gutter; its header and closing
  line must remain visible.
- Open Find with the **Find** toolbar control, `⌘F` on macOS, and `Ctrl+F` elsewhere; confirm
  next and previous navigate highlighted matches.

## Keybindings and Vim Quick Start

Open **Settings → Keybindings** to search commands, record one or more shortcut sequences,
change the Vim leader key, and save. Changes take effect immediately. Under **Settings → Editor**,
set **Vim mode** to **Global** for one modal keymap across editors, terminals, project trees,
settings, panes, tabs, and palettes; choose **Editor only** to keep Vim behavior inside project
editors, or **Disabled** to turn it off.

For a minimal manual configuration, edit `<app-config>/keybindings.toml`:

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

`<leader>` expands to the configured leader key. User entries are sparse overrides layered over
the defaults; set `unbind = true` on an entry to remove its exact default assignment. The status
bar shows the current Vim mode and pending multi-key sequence. In the focused **Projects** list,
`j/k` and `gg/G` select opened projects. In the focused project tree, `j/k`, `h/l`, `gg/G`,
Enter/`o`, `a/A`, `r/d`, `y/x/p`, `z`, `H/R`, `/`, and `q` provide configurable
neo-tree-style navigation and file operations. `Ctrl-W h/j/k/l` traverses the Projects list,
work-area panes and groups, and the right project tree. See [Usage](docs/usage.md#keybindings) for
config paths and command IDs. In Global Vim Terminal mode, `Escape` and `Ctrl-[` remain process
input; use `Ctrl-\ Ctrl-N` to return to Normal mode.

## Connect to an Existing Desktop

On computer A, enable **Settings → Permissions → Remote access to this computer**. The listener
is off by default and initially binds only `127.0.0.1:43123`. Copy its connection information,
forward that TCP port with your preferred tunnel, then run **Connect to existing yttt** on B.
Enter B's forwarded address and A's connection information. This path uses TLS 1.3 and reuses
A's running desktop Host; it neither needs SSH login on A nor deploys `yttt-server`.

The remote Client opens separately from B's local workspace. It restores A's saved work windows
and drafts, and reads and writes A's original configuration and project files. Control belongs
to one Client session for the entire profile, not one terminal or window. Normal takeover asks
all of the previous Client's windows to publish before transferring input and write authority.
After five seconds, force takeover requires a separate confirmation and uses only confirmed state.

Connection information grants work access: share it privately. Optional remembered credentials
use the OS keychain with no plaintext fallback. Disable access or reset credentials on A to
disconnect all TCP channels without terminating existing tasks. Quitting A's desktop-owned Host
does terminate them; use an explicitly started independent Host when desktop-independent lifetime
is required. See [Usage](docs/usage.md#connect-to-an-existing-desktop-host).

## SSH Projects

Use **Open SSH Project** from the command palette, empty-workbench action, or project sidebar
menu. The picker lists saved connections and provides **New connection** for quick setup; a first
connection needs only its endpoint, authentication, and optional starting root. SSH agent,
private-key (including a non-persisted passphrase), and password authentication are supported.
Unknown host keys require an explicit trust decision. yttt stores remembered host keys in its own
`ssh-host-keys.toml` and never reads or updates OpenSSH's `~/.ssh/known_hosts`. When a saved key
changes, the confirmation shows both fingerprints and only replaces the saved key after explicit
approval. Remembered passwords use the operating-system credential store; `ssh-connections.toml`
contains only endpoint and credential-binding metadata.
If a saved password is missing or rejected, the project picker asks for it again and lets you
choose whether to replace the saved credential before retrying.
The separate **SSH connections** settings page remains available for managing saved endpoints.

After authentication, yttt opens a separate remote Client window, visibly labeled with its SSH
endpoint. It deploys the headless `yttt-server` for Linux/macOS on x86_64 or aarch64 and connects
through SSH to a private Unix socket—no public application port is needed. The remote folder
picker, project files, Git, terminals, Agent hooks/history, settings, keybindings, and layouts
all belong to that Host; local configuration is not copied into it.

The Host retains the workspace and unsaved editor drafts. Another Client can explicitly take
control and restore them; the previous controller can no longer mutate remote state. Closing the
remote Client leaves Host-owned processes running. After a Host or machine restart, layout and
drafts return, but lost terminal processes remain exited until explicitly started again.
Incompatible busy Hosts refuse automatic upgrade rather than terminating work.
Legacy recent SSH project entries are retained and launch the new remote Client; they are not
automatically mixed into local workspace restoration. See [Usage](docs/usage.md#ssh-projects).

## Run

```bash
cargo run
```

Open one or more projects on startup with positional paths or `--project`:

```bash
cargo run -- /path/to/project
cargo run -- --project /path/to/project
YTTT_OPEN_PROJECT=/path/to/project cargo run
```

Developer fixtures:

```bash
YTTT_DEV_FIXTURE=1 cargo run
YTTT_DEV_FIXTURE=agent-exit cargo run
```

For GPUI smoke testing on macOS, use the dev app wrapper. It creates
`target/dev-app/yttt.app` with a stable bundle id so local UI tools can identify the
window more reliably than the naked debug binary:

```bash
scripts/run-dev-app.sh --fixture dev
scripts/run-dev-app.sh --fixture agent
```

Build release packages on their native platforms:

```bash
# macOS: target/macos/yttt.dmg
scripts/build-macos-dmg.sh

# Windows: target/windows/yttt-setup.exe (requires Inno Setup 6)
pwsh scripts/build-windows-installer.ps1

# Linux: target/linux/yttt-<version>-linux-<architecture>.tar.gz
scripts/build-linux-tar.sh
```

Prepare and validate a release before creating its tag:

```bash
# Normal version bump.
python3 scripts/prepare_release.py <version>

# Use this instead when Cargo already has <version> and CHANGELOG contains
# an older section with that version.
python3 scripts/prepare_release.py <version> --repair-current

python3 scripts/test_release_tools.py
```

Review `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md`, then commit and push them. Wait for
`Validation / Required validation` to pass on the release commit before creating and pushing the
annotated `v<version>` tag. The release workflow checks out and validates that exact tag again
before any native packaging job starts. It rejects non-empty `Unreleased` notes, a mismatched
version section, and an existing GitHub Release instead of replacing published assets.

The workflow publishes the DMG, Inno Setup installer, Linux tarball, and SHA-256 checksums. The
macOS package is ad-hoc signed; production Developer ID signing and notarization still require
release credentials.

## Key Paths

Project layout:

```text
<project>/.yttt/layout.toml
```

`$XDG_CONFIG_HOME/yttt` overrides the platform default when set. Otherwise the config root is
`~/Library/Application Support/yttt` on macOS, `%APPDATA%\yttt` on Windows, and
`~/.config/yttt` on Linux. Existing macOS and Windows `~/.config/yttt` data is moved to the
native location on first launch when the native location does not already exist. `./.yttt` is
used only when no platform user directory is available.

Shareable Window Bar and Status Bar layout:

```text
<app-config>/bars.toml
```

Global default layout:

```text
<app-config>/default-layout.toml
```

SSH connections:

```text
<app-config>/ssh-connections.toml
```

SSH host keys:

```text
<app-config>/ssh-host-keys.toml
```

Personal project layout:

```text
<app-config>/projects/<encoded-project-path>/layout.toml
```

Projects without `.yttt/layout.toml` dynamically inherit the global default. The project layout
may omit `project.name`, in which case the project directory name is used. Personal project files
use strict `version = 1` plus `mode = "patch" | "replace"`; unversioned legacy files are rejected
with a visible warning.

## More Docs

- [Usage](docs/usage.md)

## Current Limits

- Manual GPUI visual verification is still required for several phase gates.
- Real terminal input/output and resize should be smoke-tested in the launched app.
- Pointer split resize has code support, but still needs real GPUI smoke verification.
- Agent status uses bounded provider hook and process metadata; terminal output parsing is intentionally not implemented.
- Native system notifications are intentionally left as a no-op placeholder.
- Project editing accepts UTF-8 text files up to 6 MiB; binary and invalid UTF-8 files are
  rejected.
- Continuous filesystem watching is local-only; remote documents are checked when their tree is
  refreshed.
- Copy/move paste operations involving an SSH project are not supported. Create, rename, and
  delete are supported for local and SSH project trees.
- SSH terminal and command startup currently requires a POSIX-compatible remote login shell.
