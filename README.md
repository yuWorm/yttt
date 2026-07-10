# yttt

`yttt` is a Rust desktop terminal workbench built with GPUI, `gpui-component`, and a
project-owned `yttt-terminal` crate based on `alacritty_terminal`.

The product direction is project-first and terminal-first:

- Open a local project directory.
- Work in terminal tabs.
- Split panes inside a tab.
- Save personal layouts locally.
- Export shareable project layouts explicitly.
- Track process-level agent CLI exits for tools such as Codex and Claude Code.

It is not a file explorer, not an editor, and not a client/server tmux clone.

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
- Packaging and release automation are not implemented.
