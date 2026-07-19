# Changelog

## Unreleased

### Added

- Added saved SSH connection management with SSH agent, private-key, and password authentication.
- Added explicit host-key verification backed by yttt's own `ssh-host-keys.toml` store; OpenSSH `known_hosts` files are never modified.
- Added an SFTP project picker, lazy remote file tree, conflict-checked remote editing, remote terminal panes, and remote Git status, branch, and diff operations.
- Added operating-system credential-store integration for remembered SSH passwords and endpoint-bound credential metadata.
- Added drag-to-edge work-area splitting for terminal and file tabs, with independent tab groups and resizable dividers.
- Added a project-wide file finder with Git-ignore-aware local and SSH indexing, fuzzy path ranking, file previews, and `cmd-p`/`ctrl-p` shortcuts.

### Changed

- Missing or rejected SSH passwords now open a focused retry prompt with an explicit save-password choice.
- Remote directory picker rows now use the active icon theme, fill the available width, and keep directory names left-aligned.
- Long remote directory lists now use a bounded scroll viewport while short lists remain content-sized.
- Reconnecting an SSH project refreshes expanded remote directories and Git status.
- Redesigned the empty workspace as a responsive centered dashboard with app branding, stacked actions, icons, and aligned shortcut hints.

### Fixed

- Fixed Markdown IME composition updates to preserve and visibly highlight marked text across host focus requests while honoring GPUI document-space replacement ranges, preventing raw pinyin from accumulating beside committed Chinese candidates.
- Fixed file-finder previews to detect the selected file's language and apply syntax colors.
- Fixed active Markdown documents reclaiming focus from project-file create and rename inputs.
- Fixed notification popups to remain opaque when translucent window effects are enabled, matching the existing opaque dialog, panel, menu, and popover surfaces.