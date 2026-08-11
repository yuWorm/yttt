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
- Added local process-tree discovery for Codex, Claude Code, OpenCode, Pi, and Oh My Pi launched
  manually from an existing shell tab or pane.
- Added in-app and optional native desktop notifications when hook-backed agents need input,
  complete a task, or fail.
- Added a cross-platform Permissions settings page for reviewing core and optional desktop access,
  opening native macOS and Windows settings, and explaining Linux desktop/portal-managed
  authorization.

### Changed

- Unified workbench controls, dialogs, overlays, rows, panels, tabs, sidebars, notifications, and split handles behind the `yttt-ui` primitive layer; one live appearance runtime now drives application colors, typography, spacing, radii, shadows, density, and the complete `gpui-component` theme bridge.
- Reworked the Zed UI style around imported Zed semantic state colors and compact control geometry, with a full-size flat settings layout, precise button/select/menu states, focused project-tree rows, and native active/inactive tab surfaces.
- Removed decorative outlines from work-area groups, including terminal surfaces, and from every icon-button variant while retaining structural separators and focus indicators.
- Moved the tab-bar baseline behind tab items so the selected tab covers its segment and merges visually with the active content surface, matching Zed.
- Replaced the Project Panel title/action toolbar with icon tabs and a stateful right-side-panel toggle in the workbench tab bar; create, refresh, hidden-file, and project-layout actions now live in row or empty-area context menus.
- Clippy boundary rules now reject direct construction of style-sensitive `gpui-component` controls in business UI modules.
- Legacy workspace, settings, editor, and terminal Vim toggles now migrate to the least restrictive equivalent unified mode, and legacy `WorkspaceVim` keybinding contexts migrate to the Global scope.
- Consolidated all built-in agent adapters and embedded hook sources into the extensible `yttt-agent-providers` crate instead of keeping Oh My Pi in a separate crate.
- Restored running Claude, Codex, OpenCode, Pi, and Oh My Pi sessions with provider-specific resume commands after workspace restart; provider titles or stable first-prompt titles now persist with the session while custom pane titles remain authoritative.
- The Permissions page now detects native authorization where the operating system supports it,
  requests macOS Notifications, Accessibility, and Screen Recording access in place, refreshes
  after returning from system settings, and reports unsupported status checks explicitly.

### Fixed

- Fixed the project-file panel crashing with `hover style already set` by defining menu icon-button hover state only through its custom button variant.
- Fixed schema-4 `ctrl-w` pane-close overrides shadowing the `ctrl-w h/j/k/l` sequence; migration now removes the obsolete single-key override so pending Vim prefixes resolve correctly.
- Fixed Vim status-bar key feedback to preserve printable key case, so `g` and `G` remain distinguishable.
- Fixed `ctrl-w h/j/k/l` navigation to cross the left Projects list, edge terminal panes, adjacent work-area groups, and the right project tree, with the standard held-Control variants `ctrl-w ctrl-h/j/k/l`; focus transitions now update in one frame and use restrained pane-edge, panel-header, and current-row indicators instead of stacked full-panel outlines.
- Fixed active and selected states losing contrast on translucent backgrounds by deriving interaction overlays from backdrop visibility and using focused foreground colors for keyboard-owned rows.
- Fixed project-file hover feedback to match the focused-row background and removed the focused-row side marker from the workbench file tree.
- Fixed platform text and IME composition reaching palette inputs or terminal panes while Global Vim is in Normal mode; Insert and Terminal modes continue to accept composed text.
- Fixed Global Vim Terminal mode intercepting `Escape` and `Ctrl-[`; both now reach the terminal process, while `Ctrl-\ Ctrl-N` returns to yttt Normal mode.
- Fixed Zed-compatible icon themes falling back to the generic file icon for common extensions, including TypeScript, when the theme relies on Zed's built-in file associations.
- Open files deleted outside yttt now stay editable with a struck-through tab title and are recreated directly on save instead of blocking on a confirmation dialog.
- Fixed manually launched agents remaining in the sidebar after their terminal process exits or is
  killed; detected-agent snapshots are now removed from memory and persisted state.
- Fixed late Oh My Pi hook deliveries recreating a sidebar session after the monitored CLI process had already exited.

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