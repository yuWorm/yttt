# Windows titlebar verification

Run this on a Windows desktop using a binary built from the titlebar fix.
Quit any previously running yttt instance, including the tray application, before
starting the test build.

```powershell
cargo test --locked -p yttt --test ui_state titlebar_action_buttons_open_command_picker_and_settings
cargo run --locked --bin yttt
```

The GPUI test checks command and settings button actions. It does not exercise
Windows native caption hit testing or prove that a real window moves.

## Native window behavior

- [ ] With the main window restored (not maximized), drag a blank part of the
  titlebar horizontally and vertically. The window must follow the pointer and
  stop moving when the left button is released.
- [ ] Double-click the same blank area to maximize, then double-click to restore.
  Drag again after restoring.
- [ ] Drag the maximized window down from the blank titlebar area. It must restore
  and continue moving with the pointer.
- [ ] Check the minimize, maximize/restore, and close controls. Each must perform
  its own action instead of initiating a drag. Reopen the window after closing.
- [ ] Resize the window from its edges and corners; resizing must still work.

## Titlebar content

- [ ] Click the command picker and settings buttons. Each must open its intended
  interface without moving the window.
- [ ] Check clickable text modules in a customized titlebar as well as icon
  buttons. They must remain clickable; blank space between modules must drag.
- [ ] Open a project and repeat the blank-area drag with project and Git metadata
  present. Repeat with a narrow window and with each configured UI style.

## Other windows sharing the titlebar

For each available window below, test blank-area dragging, double-click
maximize/restore, and the native window controls:

- [ ] Settings.
- [ ] Remote services.
- [ ] Layout editor.
- [ ] Remote connection window (before establishing a connection).

Record the Windows version, display scaling, UI style, tested build, and any
failed item. A passing button test alone is not a passing native drag test.

## Implementation constraint

Windows needs `WindowControlArea::Drag`, which GPUI maps to `HTCAPTION` during
`WM_NCHITTEST`. The pinned GPUI Windows backend does not implement
`start_window_move()`. Titlebar action controls use `occlude()` to keep their
hitboxes from exposing the caption area underneath; retain this when adding
interactive titlebar content.
