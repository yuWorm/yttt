# Changelog

## Unreleased

### Added

- Added a unified, live-reloadable GPUI keymap covering commands, palettes, project tree, Git diff, terminal, editor Vim, and modal UI actions, with contextual sequences and per-action unbinding.
- Added a single `Global` / `Editor only` / `Disabled` Vim setting backed by one window-level mode controller and a persistent mode/context status bar; Global mode spans editors, terminals, project trees, settings, panes, tabs, and command palettes.
- Added configurable Vim leader expansion, multi-keystroke shortcut recording, alternative shortcut sequences, and an in-app quick-start guide for the unified keymap.
- Added transient pressed-key feedback to the Vim status bar for normal-mode commands and pending multi-key sequences without echoing insert or terminal text.
- Added configurable neo-tree-style Global Vim controls for the Projects list (`j/k`, `gg/G`) and project files (`j/k`, `h/l`, `gg/G`, Enter/`o`, create, rename, delete, copy, cut, paste, collapse-all, hidden-file, refresh, finder, and panel-close actions).

### Changed

- Legacy workspace, settings, editor, and terminal Vim toggles now migrate to the least restrictive equivalent unified mode, and legacy `WorkspaceVim` keybinding contexts migrate to the Global scope.

### Fixed

- Fixed schema-4 `ctrl-w` pane-close overrides shadowing the `ctrl-w h/j/k/l` sequence; migration now removes the obsolete single-key override so pending Vim prefixes resolve correctly.
- Fixed Vim status-bar key feedback to preserve printable key case, so `g` and `G` remain distinguishable.
- Fixed `ctrl-w h/j/k/l` navigation to cross the left Projects list, edge terminal panes, adjacent work-area groups, and the right project tree, with the standard held-Control variants `ctrl-w ctrl-h/j/k/l`; focus transitions now update in one frame and use restrained pane-edge, panel-header, and current-row indicators instead of stacked full-panel outlines.
- Fixed active and selected states losing contrast on translucent backgrounds by deriving interaction overlays from backdrop visibility and using focused foreground colors for keyboard-owned rows.
- Fixed project-file hover feedback to match the focused-row background and removed the focused-row side marker from the workbench file tree.
- Fixed platform text and IME composition reaching palette inputs or terminal panes while Global Vim is in Normal mode; Insert and Terminal modes continue to accept composed text.
- Fixed Global Vim Terminal mode intercepting `Escape` and `Ctrl-[`; both now reach the terminal process, while `Ctrl-\ Ctrl-N` returns to yttt Normal mode.
- Fixed Zed-compatible icon themes falling back to the generic file icon for common extensions, including TypeScript, when the theme relies on Zed's built-in file associations.

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