//! The keys a phone keyboard cannot type, as the program in the terminal
//! expects them: the key bar shared by both apps sends these.

/// A key on the key bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TerminalKey {
    Escape,
    Tab,
    ShiftTab,
    Enter,
    Backspace,
    Up,
    Down,
    Left,
    Right,
}

/// The bytes for `key`. Arrows follow the cursor-key mode the program set:
/// `app_cursor` (DECCKM, `CSI ? 1 h`) sends `ESC O A` instead of `ESC [ A`.
pub fn encode(key: TerminalKey, app_cursor: bool) -> &'static [u8] {
    match (key, app_cursor) {
        (TerminalKey::Escape, _) => b"\x1b",
        (TerminalKey::Tab, _) => b"\t",
        (TerminalKey::ShiftTab, _) => b"\x1b[Z",
        (TerminalKey::Enter, _) => b"\r",
        (TerminalKey::Backspace, _) => b"\x7f",
        (TerminalKey::Up, false) => b"\x1b[A",
        (TerminalKey::Down, false) => b"\x1b[B",
        (TerminalKey::Right, false) => b"\x1b[C",
        (TerminalKey::Left, false) => b"\x1b[D",
        (TerminalKey::Up, true) => b"\x1bOA",
        (TerminalKey::Down, true) => b"\x1bOB",
        (TerminalKey::Right, true) => b"\x1bOC",
        (TerminalKey::Left, true) => b"\x1bOD",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_keys() {
        for app_cursor in [false, true] {
            assert_eq!(encode(TerminalKey::Escape, app_cursor), b"\x1b");
            assert_eq!(encode(TerminalKey::Tab, app_cursor), b"\t");
            assert_eq!(encode(TerminalKey::ShiftTab, app_cursor), b"\x1b[Z");
            assert_eq!(encode(TerminalKey::Enter, app_cursor), b"\r");
            assert_eq!(encode(TerminalKey::Backspace, app_cursor), b"\x7f");
        }
    }

    #[test]
    fn arrows_follow_the_cursor_key_mode() {
        let normal = [b"\x1b[A", b"\x1b[B", b"\x1b[C", b"\x1b[D"];
        let application = [b"\x1bOA", b"\x1bOB", b"\x1bOC", b"\x1bOD"];
        let keys = [
            TerminalKey::Up,
            TerminalKey::Down,
            TerminalKey::Right,
            TerminalKey::Left,
        ];
        for (index, key) in keys.into_iter().enumerate() {
            assert_eq!(encode(key, false), normal[index]);
            assert_eq!(encode(key, true), application[index]);
        }
    }
}
