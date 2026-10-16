<h1 align="center">
  <img src="assets/app-icon/png/128.png" alt="yttt app icon" width="64" valign="middle"> yttt
</h1>

<p align="center">macOS · Windows · Linux</p>

<p align="center"><sub><a href="README.zh-CN.md">中文</a> · <a href="README.md">English</a></sub></p>

<p align="center">
  <strong>A project-first, terminal-first workbench for CLI agents.</strong><br>
  Keep projects, terminals, files, and Agent sessions together.
</p>

<h3 align="center"><a href="https://github.com/yuWorm/yttt/releases">Download yttt</a></h3>

<p align="center">
  <img src="docs/images/readme-workbench.png" alt="yttt maximized workbench with five demo projects, Agent states, and split terminals" width="960">
</p>
<p align="center"><sub>Maximized app capture. Projects and Agent states are simulated; terminal text is composited demo content. No Agent CLI was launched.</sub></p>

## Features

<table>
<tr>
<td width="50%" valign="top">
<h3>Projects &amp; Agents</h3>
<p>Switch projects without losing each project's tabs, layout, or Agent status.</p>
<a href="docs/usage.md#product-model">Guide →</a>
</td>
<td width="50%" valign="top">
<h3>Split Terminals</h3>
<p>Split terminals inside tabs and keep multiple CLI workflows in view.</p>
<a href="docs/usage.md#first-launch">Guide →</a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>Files &amp; Editor</h3>
<p>Browse project files, edit text, and preview images alongside terminals.</p>
<a href="docs/usage.md#project-files-and-editor">Guide →</a>
</td>
<td width="50%" valign="top">
<h3>Local &amp; Remote Workspaces</h3>
<p>Connect to an existing desktop Host or open projects through SSH, with resources owned by their Host.</p>
<a href="docs/usage.md#connect-to-an-existing-desktop-host">Desktop Host →</a> · <a href="docs/usage.md#ssh-projects">SSH →</a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>Keys &amp; Appearance</h3>
<p>Customize shortcuts, Global Vim, themes, and bars without mixing device and Host settings.</p>
<a href="docs/usage.md#keybindings">Keybindings →</a> · <a href="docs/usage.md#configuration-targets">Settings →</a>
</td>
<td width="50%" valign="top">
<h3>Desktop CLI</h3>
<p>Use <code>yttt ctl</code> to inspect projects and Agent status, and manage tabs, panes, and terminal input.</p>
<a href="docs/cli-control.md">CLI guide →</a>
</td>
</tr>
</table>

## Agent workflows

Run the CLI agents you already use in terminal panes. yttt can show supported Agent lifecycle
status in the project sidebar; a terminal command and an Agent-status integration are distinct.
See [Agent status](docs/usage.md#agent-status).

## Installation

Get the desktop package for your platform from [GitHub Releases](https://github.com/yuWorm/yttt/releases):

- **macOS Apple Silicon:** download the `.dmg` and drag yttt to `/Applications` or `~/Applications`.
  On its first launch there, yttt registers `/usr/local/bin/yttt`, using native macOS authorization
  only when permission is required. It does not modify shell configuration.
- **Windows x86_64:** run the `.exe` installer. It adds the selected application directory to
  user `PATH` by default, or system `PATH` for an all-users installation. Open a new terminal afterward.
  The app launched by Setup receives the new command directory immediately. Installation paths
  containing `;` are rejected. Upgrade in place; uninstall first to change an installer-owned
  command's installation directory or scope. Per-user uninstall must run as the original user.
- **Linux x86_64:** extract the `.tar.gz`, then run the explicit per-user installer from the extracted
  directory:

  ```sh
  ./install.sh
  ```

  It copies yttt to `${XDG_DATA_HOME:-$HOME/.local/share}/yttt/desktop`, links
  `~/.local/bin/yttt`, and persists PATH even when the current terminal already contains that
  directory. Bash is configured for both interactive and login shells (`.bashrc` and the first
  existing `.bash_profile`, `.bash_login`, or `.profile`; `.bash_profile` is created if none exists).
  Zsh, Fish, and sh use their normal startup configuration. Open a new terminal afterward.
  The installer requires `flock`, serializes concurrent runs, preserves user configuration and
  unrelated commands, and can be retried after a failed install. HOME must not contain `:`.
  It does not call `sudo`; running the archive's `bin/yttt` directly remains portable and does not
  alter PATH or shell files.

Check the supplied SHA-256 sums. macOS packages are ad-hoc signed, not notarized.
Keep desktop, Host, and remote Server on matching versions.

<details>
<summary>Upgrading from the old 1.0.0 release</summary>

Development returned to pre-1.0 versioning after v1.0.0. Install a newer pre-1.0 release manually;
a SemVer-based updater will not treat it as an upgrade from 1.0.0.

</details>

## Get started

Open the app, choose a language and terminal font, then open a local project. Add terminal or
Agent tabs, split a tab into panes, and open project files from the right-hand tree. For remote
access and session restoration, follow the [usage guide](docs/usage.md).

To drive an already-running desktop from a script, start with `yttt ctl --help` or the
[CLI examples](docs/cli-control.md).

## Documentation

- [Usage guide](docs/usage.md) — UI, remote access, session restoration, and settings.
- [CLI control](docs/cli-control.md) — desktop commands and examples.
- [Host/Client architecture](docs/host-client-architecture.md) — process and resource ownership.
- [Changelog](CHANGELOG.md) — releases and upgrade notes (English and Chinese).

## Develop

Build and launch from source with Rust:

```bash
cargo run
cargo run -- /path/to/project
```

On macOS, `scripts/run-dev-app.sh` creates a development `.app` bundle for GPUI UI checks.
Use `scripts/run-dev-app.sh --fixture readme` to open five **simulated** projects for the
screenshot above; it does not launch Agent CLIs.

<details>
<summary>Release maintenance</summary>

Build packages on their native platforms with `scripts/build-macos-dmg.sh`,
`scripts/build-windows-installer.ps1`, and `scripts/build-linux-tar.sh`. Prepare a release
with `python3 scripts/prepare_release.py <version>` (or `--repair-current` when repairing
an existing version section), then run `python3 scripts/test_release_tools.py`.
Review the manifest, lockfile, changelog, and README before tagging; wait for **Build**
to succeed on the release commit on the default branch. Release reuses only a successful
default-branch push Build for the exact same SHA; otherwise it runs full validation.
All packages and Server binaries must succeed before publishing update metadata and checksums.

CI restores a local `sccache` snapshot keyed by OS, architecture, toolchain, and lockfile.
Only successful default-branch Build jobs save snapshots, with a 1 GiB disk limit and an
upload size guard; PRs and tags are restore-only. No `target/`, incremental state, or
downloaded source trees are uploaded. Debug/test symbols are disabled only in CI;
release optimization settings and local development builds are unchanged. The three
Build platforms use at most 3 GiB of uncompressed snapshots per dependency generation;
older generations remain subject to GitHub's separate default 10 GiB repository cache
limit and eviction. Do not raise the repository cache limit for this workflow.
The first build is cold; cache hits do not eliminate linking or test execution.
Linux ARM64 and Intel macOS Server builds have no matching default-branch cache writer.

Release staging artifacts and smoke metrics expire after one day. After successful
publication, a separate job deletes only that run's `release-*` staging artifacts,
not GitHub Release assets. Failed releases retain staging artifacts for one day;
after expiry, rerun all packaging jobs rather than only the failed publish job.
Historical artifacts are not deleted automatically by this change. GitHub Free lists
500 MB of artifact storage (shared with Packages), separately from the 10 GiB cache
allowance; short retention does not guarantee an account-wide storage ceiling.
See [GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions).

</details>
