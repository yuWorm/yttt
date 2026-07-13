# yttt

`yttt` is a Rust desktop terminal workbench built with GPUI, `gpui-component`, and a
project-owned `yttt-terminal` crate based on `alacritty_terminal`.

The product direction is project-first and terminal-first, with lightweight project-file
editing built into the same workbench:

- Open a local project directory.
- Work in unified terminal and file tabs.
- Browse project files from a lazy tree on the right.
- Edit and save UTF-8 text files without leaving the terminal workflow.
- Split panes inside a tab.
- Save personal layouts locally.
- Export shareable project layouts explicitly.
- Track process-level agent CLI exits for tools such as Codex and Claude Code.

It is not a client/server tmux clone or a full file manager. The editor is intentionally
focused on project text files and keeps terminal work as the primary workflow.

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
JavaScript, Kotlin, Lua, PHP, Python, Ruby, Rust, Scala, Swift, TypeScript, and Zig.
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

## Run

```bash
cargo run
```

Open a project on startup:

```bash
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

Build a release macOS application bundle:

```bash
scripts/build-macos-bundle.sh
open target/macos/yttt.app
```

The script runs a locked release build, copies the application icon, writes the native
`Info.plist`, and ad-hoc signs `target/macos/yttt.app`. Use `--help` to package an existing
binary, select another output path, or skip signing.

## Key Paths

Project layout:

```text
<project>/.yttt/layout.toml
```

The config root is `$XDG_CONFIG_HOME/yttt` when set, otherwise `~/.config/yttt`, with
`./.yttt` used only when neither location is available.

Global default layout:

```text
<app-config>/default-layout.toml
```

Personal project layout:

```text
<app-config>/projects/<encoded-project-path>/layout.toml
```

Projects without `.yttt/layout.toml` dynamically inherit the global default. Personal
project files use strict `version = 1` plus `mode = "patch" | "replace"`; unversioned
legacy files are rejected with a visible warning.

## More Docs

- [Usage](docs/usage.md)

## Current Limits

- Manual GPUI visual verification is still required for several phase gates.
- Real terminal input/output and resize should be smoke-tested in the launched app.
- Pointer split resize has code support, but still needs real GPUI smoke verification.
- Agent state is process-level only; output parsing is intentionally not implemented.
- System notification click routing is not wired to the OS yet.
- Sessions and running processes are not restored after app restart.
- Project editing accepts UTF-8 text files up to 10 MiB; binary and invalid UTF-8 files are
  rejected.
- Continuous filesystem watching is limited to the active project; inactive projects refresh
  when selected.
- The project tree does not create, rename, move, or delete files and directories.
- Packaging and release automation are not implemented.
