# Changelog

## Unreleased

### Added

- Added a unified, live-reloadable GPUI keymap covering commands, palettes, project tree, Git diff, terminal, editor Vim, and modal UI actions, with contextual sequences and per-action unbinding.
- Added a single `Global` / `Editor only` / `Disabled` Vim setting backed by one window-level mode controller and a persistent mode/context status bar; Global mode spans editors, terminals, project trees, settings, panes, tabs, and command palettes.
- Added configurable Vim leader expansion, multi-keystroke shortcut recording, alternative shortcut sequences, and an in-app quick-start guide for the unified keymap.
- Added transient pressed-key feedback to the Vim status bar for normal-mode commands and pending multi-key sequences without echoing insert or terminal text.
- Added configurable neo-tree-style Global Vim controls for the Projects list (`j/k`, `gg/G`) and project files (`j/k`, `h/l`, `gg/G`, Enter/`o`, create, rename, delete, copy, cut, paste, collapse-all, hidden-file, refresh, finder, and panel-close actions).
- Added centered, extensible Project Panel icon tabs with an accent-highlighted Files tab, muted Search/Git/Terminal placeholders, and Vim page navigation (`[ p`, `] p`, `g p f`) scoped independently from file-tree commands.
- Added persistent global terminal environment variables that are automatically injected into newly started local and SSH shells and CLI commands.
- Added local process-tree discovery for Codex, Claude Code, Grok, OpenCode, Pi, and Oh My Pi
  launched manually from an existing shell tab or pane.
- Added in-app and optional native desktop notifications when hook-backed agents need input,
  complete a task, or fail.
- Added first-class Grok Build integration, including the `groky` fork alias and shared Grok icon,
  native personal hooks, live task/tool/subagent state, local session discovery, and `--resume`
  restoration.
- Added a cross-platform Permissions settings page for reviewing core and optional desktop access,
  opening native macOS and Windows settings, and explaining Linux desktop/portal-managed
  authorization.
- Added an authenticated, profile-isolated headless Host process with versioned local IPC, resource catalogs, terminal checkpoints, lease-controlled input, reconnect recovery, and semantic terminal mirrors.
- Added one desktop shell and native tray/menu-bar owner per profile, with window reopening, Host
  status and resource counts, safe start/stop/restart controls, log access, and separate desktop
  versus all-process quit actions.
- Added tray-independent Host lifecycle CLI commands for status, start, safe stop, restart, force
  stop, and opening profile logs.
- Added explicit, profile-scoped **Start Host at login** controls with macOS `SMAppService`,
  Windows current-user Run-key registration, Linux systemd-user/XDG fallback, CLI management,
  approval status, and secret-free startup arguments.

### Changed

- Unified workbench controls, dialogs, overlays, rows, panels, tabs, sidebars, notifications, and split handles behind the `yttt-ui` primitive layer; one live appearance runtime now drives application colors, typography, spacing, radii, shadows, density, and the complete `gpui-component` theme bridge.
- Reworked the Zed UI style around imported Zed semantic state colors and compact control geometry, with a full-size flat settings layout, precise button/select/menu states, focused project-tree rows, and native active/inactive tab surfaces.
- Removed decorative outlines from work-area groups, including terminal surfaces, and from every icon-button variant while retaining structural separators and focus indicators.
- Moved the tab-bar baseline behind tab items so the selected tab covers its segment and merges visually with the active content surface, matching Zed.
- Replaced the Project Panel title/action toolbar with icon tabs and a stateful right-side-panel toggle in the workbench tab bar; create, refresh, hidden-file, and project-layout actions now live in row or empty-area context menus.
- Clippy boundary rules now reject direct construction of style-sensitive `gpui-component` controls in business UI modules.
- Legacy workspace, settings, editor, and terminal Vim toggles now migrate to the least restrictive equivalent unified mode, and legacy `WorkspaceVim` keybinding contexts migrate to the Global scope.
- Consolidated all built-in agent adapters and embedded hook sources into the extensible `yttt-agent-providers` crate instead of keeping Oh My Pi in a separate crate.
- Restored running Claude, Codex, Grok, OpenCode, Pi, and Oh My Pi sessions with provider-specific resume commands after workspace restart; provider titles or stable first-prompt titles now persist with the session while custom pane titles remain authoritative.
- The Permissions page now detects native authorization where the operating system supports it,
  requests macOS Notifications, Accessibility, and Screen Recording access in place, refreshes
  after returning from system settings, and reports unsupported status checks explicitly.
- Moved local and SSH terminal processes, project file trees and writes, Git execution, project watchers, Agent hook ingress, SSH connections, and SSH credential access out of the GPUI process and into the Host; closing a window now detaches without terminating Host-owned resources.
- Release packages keep one executable with separate desktop and headless Host process roles on macOS, Windows, and Linux.
- Desktop startup now detects Host build/resource incompatibility before using the resource
  protocol, replaces an idle old Host through the lifecycle protocol, preserves a busy Host with
  typed blockers, isolates development builds into executable-scoped profile runtimes, and reports
  unrecoverable live-Host lock states instead of killing resources or spawning a duplicate Host.
- Production desktop shells use explicit quit semantics: closing the last window keeps the
  desktop control plane and its owned Host available through the tray, while quitting or losing
  the desktop shell terminates that Host and its resources. Explicit CLI/login-started background
  Hosts remain independent.
- Isolated each Host client into control, terminal-interactive, per-terminal data, and state-event
  connections; resource catalogs are now cached and refreshed from invalidation events instead of
  being embedded in viewport synchronization or fetched before every terminal creation.
- Reduced long-running Host overhead by moving terminal event and child monitoring from dedicated
  OS threads to lightweight runtime tasks and sampling process diagnostics every five seconds.

### Fixed

- Fixed the project-file panel crashing with `hover style already set` by defining menu icon-button hover state only through its custom button variant.
- Fixed schema-4 `ctrl-w` pane-close overrides shadowing the `ctrl-w h/j/k/l` sequence; migration now removes the obsolete single-key override so pending Vim prefixes resolve correctly.
- Fixed Vim status-bar key feedback to preserve printable key case, so `g` and `G` remain distinguishable.
- Fixed `ctrl-w h/j/k/l` navigation to cross the left Projects list, edge terminal panes, adjacent work-area groups, and the right project tree, with the standard held-Control variants `ctrl-w ctrl-h/j/k/l`; focus transitions now update in one frame and use restrained pane-edge, panel-header, and current-row indicators instead of stacked full-panel outlines.
- Fixed active and selected states losing contrast on translucent backgrounds by deriving interaction overlays from backdrop visibility and using focused foreground colors for keyboard-owned rows.
- Fixed project-file hover feedback to match the focused-row background and removed the focused-row side marker from the workbench file tree.
- Fixed platform text and IME composition reaching palette inputs or terminal panes while Global Vim is in Normal mode; Insert and Terminal modes continue to accept composed text.
- Fixed long Agent session titles expanding the project panel beyond its layout and preventing metadata tooltips from receiving hover input.
- Fixed Global Vim Terminal mode intercepting `Escape` and `Ctrl-[`; both now reach the terminal process, while `Ctrl-\ Ctrl-N` returns to yttt Normal mode.
- Fixed Zed-compatible icon themes falling back to the generic file icon for common extensions, including TypeScript, when the theme relies on Zed's built-in file associations.
- Open files deleted outside yttt now stay editable with a struck-through tab title and are recreated directly on save instead of blocking on a confirmation dialog.
- Restored Host-owned process-tree monitoring for manually launched Codex, Claude Code, Grok,
  OpenCode, Pi, and Oh My Pi CLIs. Two missed samples now end the live sidebar snapshot even when
  the parent shell remains running or the Agent cannot emit its `SessionEnd`/`session_shutdown`
  hook.
- Fixed closed and immediately recreated terminal tabs inheriting an old Agent identity by dropping
  pane caches and retained snapshots synchronously, rejecting late updates for absent tabs, and
  clearing the Host record before binding a new terminal incarnation.
- Fixed Grok detection in development builds by installing the shared stateless hook adapter outside
  the profile runtime, accepting Grok's native `snake_case` lifecycle events, and dropping the
  duplicate Claude-hook delivery that Grok's compatibility loader re-exports.
- Fixed Host terminal input feeling network-lagged by isolating slow project/file/Git requests from
  the terminal-interactive lane, making control and interactive frame readers cancellation-safe,
  coalescing semantic terminal data to a 16 ms frame cadence, and keeping terminal frames off
  generic GPUI event listeners.
- Fixed residual Host terminal UI stalls by detecting shell candidates once at Workbench startup
  instead of synchronously scanning every `PATH` entry during each GPUI render.
- Agent panes without an authoritative snapshot now display `Stale` instead of inferring `Working`
  from a live terminal process, and the Host snapshot bridge coalesces the latest sequenced update
  per terminal session instead of dropping final states when a bounded queue fills.
- Fixed residual rapid-input latency in Host terminals by sending terminal input as an ordered
  one-way resource-protocol v3 message, removing per-keystroke responses, and coalescing adjacent
  writer commands without copying their byte payloads.
- Fixed mouse selection in Host terminals by maintaining an ephemeral Client-side range over the
  authoritative semantic viewport; drag, word, and line selection now highlight and copy the same
  text as Direct terminals without a Host round trip, including soft-wrapped and wide-character
  rows.
- Host semantic capture now encodes Alacritty damage rows instead of rescanning every visible cell;
  the background receiver only queues immutable updates, while GPUI applies one bounded batch per
  redraw and serves key/text callbacks from foreground-owned mode state. Async mailbox delivery now
  defers refresh of the owning window until any in-flight draw completes, preventing the final
  terminal update from remaining behind a coalesced wakeup. Performance reports expose real input
  callback time, semantic queue age, render-state lock wait, queue high-water, and coalesced update
  counts in addition to end-to-end input-to-first-paint latency.
- Fixed Host-owned Agent panes remaining `running` after completion, restart, or delayed hook
  delivery. Hook events now carry a delivery stream and monotonic sequence, retry with bounded
  exponential backoff until acknowledged, buffer small gaps, ignore duplicates, and cannot
  overwrite a terminal's final exit state; desktop reconciliation remains keyed by terminal
  placement and clears state when the backing Host terminal is lost.
- Fixed desktop Host replacement leaving panes permanently bound to an old Host identity:
  missing `Bound`, `ClosePending`, and `Lost` placements now start a fresh session, and successful
  terminal-exit acknowledgements persist `Closed`.
- Fixed completed manual-restart Agent tabs refusing to close in Host mode: a terminal placement
  already acknowledged as `Closed` now makes repeated close preparation an idempotent no-op instead
  of failing with `NotBound` before the local tab can be removed.

## 0.2.0 - 2026-07-18

### Added

- Added saved SSH connection management with SSH agent, private-key, and password authentication.
- Added explicit host-key verification backed by yttt's own `ssh-host-keys.toml` store; OpenSSH `known_hosts` files are never modified.
- Added an SFTP project picker, lazy remote file tree, conflict-checked remote editing, remote terminal panes, and remote Git status, branch, and diff operations.
- Added operating-system credential-store integration for remembered SSH passwords and endpoint-bound credential metadata.
- Added drag-to-edge work-area splitting for terminal and file tabs, with independent tab groups and resizable dividers.
- Added a project-wide file finder with Git-ignore-aware local and SSH indexing, fuzzy path ranking, file previews, and `cmd-p`/`ctrl-p` shortcuts.
- Added non-blocking application update checks with daily caching, manual checks, localized settings, and platform-specific release downloads.
- Added release preparation and metadata tooling that generates changelog-backed GitHub Release notes, checksums, and the client update manifest.

### Changed

- Missing or rejected SSH passwords now open a focused retry prompt with an explicit save-password choice.
- Remote directory picker rows now use the active icon theme, fill the available width, and keep directory names left-aligned.
- Long remote directory lists now use a bounded scroll viewport while short lists remain content-sized.
- Reconnecting an SSH project refreshes expanded remote directories and Git status.
- Redesigned the empty workspace as a responsive centered dashboard with app branding, stacked actions, icons, and aligned shortcut hints.
- First-run terminal font selection now recommends the best installed fixed-width Nerd Font, or shows a Maple Mono NF installation link when none are available.

### Fixed

- Fixed Markdown IME composition updates to preserve and visibly highlight marked text across host focus requests while honoring GPUI document-space replacement ranges, preventing raw pinyin from accumulating beside committed Chinese candidates.
- Fixed file-finder previews to detect the selected file's language and apply syntax colors.
- Fixed active Markdown documents reclaiming focus from project-file create and rename inputs.
- Fixed notification popups to remain opaque when translucent window effects are enabled, matching the existing opaque dialog, panel, menu, and popover surfaces.
- Fixed Windows builds and SSH agent authentication by using the OpenSSH named pipe with a Pageant fallback.