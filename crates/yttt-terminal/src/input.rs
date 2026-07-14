//! Framework-neutral terminal keyboard events and legacy/Kitty encoding.

use alacritty_terminal::term::TermMode;
use bytes::{Bytes, BytesMut};
use gpui::{KeyDownEvent, KeyUpEvent, Keystroke};
use smallvec::SmallVec;
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TerminalModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub super_key: bool,
}

impl TerminalModifiers {
    fn bits(self) -> u8 {
        u8::from(self.shift)
            | (u8::from(self.alt) << 1)
            | (u8::from(self.control) << 2)
            | (u8::from(self.super_key) << 3)
    }

    fn is_empty(self) -> bool {
        self.bits() == 0
    }

    fn kitty_parameter(self) -> u8 {
        self.bits() + 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyState {
    Pressed,
    Repeated,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TerminalNamedKey {
    Escape,
    Enter,
    Tab,
    Backspace,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TerminalKey {
    Character(String),
    Named(TerminalNamedKey),
    Function(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalKeyEvent {
    pub key: TerminalKey,
    pub base_key: Option<char>,
    pub text: Option<String>,
    pub modifiers: TerminalModifiers,
    pub state: KeyState,
    pub prefer_character_input: bool,
}

impl TerminalKeyEvent {
    pub fn from_key_down(event: &KeyDownEvent) -> Option<Self> {
        Self::from_gpui_keystroke(
            &event.keystroke,
            if event.is_held {
                KeyState::Repeated
            } else {
                KeyState::Pressed
            },
            event.prefer_character_input,
        )
    }

    pub fn from_key_up(event: &KeyUpEvent) -> Option<Self> {
        Self::from_gpui_keystroke(&event.keystroke, KeyState::Released, false)
    }

    pub fn from_gpui_keystroke(
        keystroke: &Keystroke,
        state: KeyState,
        prefer_character_input: bool,
    ) -> Option<Self> {
        let normalized = keystroke.key.to_ascii_lowercase();
        let named = match normalized.as_str() {
            "escape" | "esc" => Some(TerminalNamedKey::Escape),
            "enter" | "return" => Some(TerminalNamedKey::Enter),
            "tab" => Some(TerminalNamedKey::Tab),
            "backspace" | "back" => Some(TerminalNamedKey::Backspace),
            "insert" => Some(TerminalNamedKey::Insert),
            "delete" | "del" => Some(TerminalNamedKey::Delete),
            "home" => Some(TerminalNamedKey::Home),
            "end" => Some(TerminalNamedKey::End),
            "pageup" | "page_up" => Some(TerminalNamedKey::PageUp),
            "pagedown" | "page_down" => Some(TerminalNamedKey::PageDown),
            "up" | "arrowup" => Some(TerminalNamedKey::ArrowUp),
            "down" | "arrowdown" => Some(TerminalNamedKey::ArrowDown),
            "left" | "arrowleft" => Some(TerminalNamedKey::ArrowLeft),
            "right" | "arrowright" => Some(TerminalNamedKey::ArrowRight),
            _ => None,
        };

        let (key, base_key, text) = if let Some(named) = named {
            let text = match named {
                TerminalNamedKey::Enter => Some("\r".to_string()),
                TerminalNamedKey::Tab => Some("\t".to_string()),
                TerminalNamedKey::Backspace => Some("\x7f".to_string()),
                TerminalNamedKey::Escape => Some("\x1b".to_string()),
                _ => None,
            };
            (TerminalKey::Named(named), None, text)
        } else if normalized == "space" {
            (
                TerminalKey::Character(" ".to_string()),
                Some(' '),
                Some(
                    keystroke
                        .key_char
                        .clone()
                        .unwrap_or_else(|| " ".to_string()),
                ),
            )
        } else if let Some(function) = normalized
            .strip_prefix('f')
            .and_then(|number| number.parse::<u8>().ok())
            .filter(|number| (1..=35).contains(number))
        {
            (TerminalKey::Function(function), None, None)
        } else if keystroke.key.chars().count() == 1 {
            let base_key = keystroke.key.chars().next();
            (
                TerminalKey::Character(keystroke.key.clone()),
                base_key,
                Some(
                    keystroke
                        .key_char
                        .clone()
                        .unwrap_or_else(|| keystroke.key.clone()),
                ),
            )
        } else {
            return None;
        };

        Some(Self {
            key,
            base_key,
            text,
            modifiers: TerminalModifiers {
                shift: keystroke.modifiers.shift,
                alt: keystroke.modifiers.alt,
                control: keystroke.modifiers.control,
                super_key: keystroke.modifiers.platform,
            },
            state,
            prefer_character_input,
        })
    }

    pub fn identity(&self) -> TerminalKey {
        self.key.clone()
    }
}

pub fn encode_key(event: &TerminalKeyEvent, mode: TermMode) -> Option<SmallVec<[u8; 32]>> {
    if event.prefer_character_input
        && event.state != KeyState::Released
        && !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC)
    {
        return event
            .text
            .as_deref()
            .filter(|text| !text.is_empty())
            .map(|text| SmallVec::from_slice(text.as_bytes()));
    }
    if event.state != KeyState::Released {
        if mode.contains(TermMode::APP_CURSOR)
            && event.modifiers.is_empty()
            && matches!(
                event.key,
                TerminalKey::Named(
                    TerminalNamedKey::Home
                        | TerminalNamedKey::End
                        | TerminalNamedKey::ArrowUp
                        | TerminalNamedKey::ArrowDown
                        | TerminalNamedKey::ArrowLeft
                        | TerminalNamedKey::ArrowRight
                )
            )
        {
            return encode_legacy(event, mode);
        }
        if matches!(event.key, TerminalKey::Function(1..=4))
            && !mode.intersects(TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::DISAMBIGUATE_ESC_CODES)
        {
            return encode_legacy(event, mode);
        }
    }

    if mode.intersects(
        TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::DISAMBIGUATE_ESC_CODES
            | TermMode::REPORT_EVENT_TYPES,
    ) {
        encode_with_kitty(event, mode)
    } else {
        encode_legacy(event, mode)
    }
}

pub(crate) fn encode_text_input(text: &str, mode: TermMode) -> Option<Bytes> {
    if text.is_empty() {
        return None;
    }

    if !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC)
        || !mode.contains(TermMode::REPORT_ASSOCIATED_TEXT)
        || contains_control_character(text)
    {
        return Some(Bytes::copy_from_slice(text.as_bytes()));
    }

    let mut payload = String::with_capacity(text.len().saturating_mul(2).saturating_add(7));
    payload.push_str("\x1b[0;;");
    append_kitty_codepoints(&mut payload, text);
    payload.push('u');
    Some(Bytes::from(payload.into_bytes()))
}
fn encode_with_kitty(event: &TerminalKeyEvent, mode: TermMode) -> Option<SmallVec<[u8; 32]>> {
    if event.state == KeyState::Released {
        if !mode.contains(TermMode::REPORT_EVENT_TYPES) {
            return None;
        }
        if matches!(
            event.key,
            TerminalKey::Named(
                TerminalNamedKey::Enter | TerminalNamedKey::Tab | TerminalNamedKey::Backspace
            )
        ) && !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC)
        {
            return None;
        }
    }

    if !should_build_kitty_sequence(event, mode) {
        return encode_legacy(event, mode);
    }

    let associated_text = event.text.as_deref().filter(|text| {
        mode.contains(TermMode::REPORT_ASSOCIATED_TEXT)
            && event.state != KeyState::Released
            && !text.is_empty()
            && !contains_control_character(text)
    });
    let (base, terminator) = kitty_base(event, mode, associated_text.is_some())?;
    let report_event_type = mode.contains(TermMode::REPORT_EVENT_TYPES)
        && matches!(event.state, KeyState::Repeated | KeyState::Released);

    let mut payload = String::with_capacity(32);
    payload.push_str("\x1b[");
    payload.push_str(&base);
    if report_event_type || !event.modifiers.is_empty() || associated_text.is_some() {
        let _ = write!(payload, ";{}", event.modifiers.kitty_parameter());
    }
    if report_event_type {
        payload.push(':');
        payload.push(match event.state {
            KeyState::Repeated => '2',
            KeyState::Released => '3',
            KeyState::Pressed => '1',
        });
    }
    if let Some(text) = associated_text {
        payload.push(';');
        append_kitty_codepoints(&mut payload, text);
    }
    payload.push(terminator);
    Some(SmallVec::from_slice(payload.as_bytes()))
}

fn should_build_kitty_sequence(event: &TerminalKeyEvent, mode: TermMode) -> bool {
    if mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) {
        return true;
    }
    if event.state == KeyState::Released && mode.contains(TermMode::REPORT_EVENT_TYPES) {
        return true;
    }

    let shift_only = event.modifiers
        == (TerminalModifiers {
            shift: true,
            ..TerminalModifiers::default()
        });
    let disambiguated_control = matches!(
        event.key,
        TerminalKey::Named(
            TerminalNamedKey::Tab | TerminalNamedKey::Enter | TerminalNamedKey::Backspace
        )
    );
    let disambiguate = mode.contains(TermMode::DISAMBIGUATE_ESC_CODES)
        && (matches!(event.key, TerminalKey::Named(TerminalNamedKey::Escape))
            || (!event.modifiers.is_empty() && (!shift_only || disambiguated_control)));
    if disambiguate {
        return true;
    }

    matches!(
        event.key,
        TerminalKey::Named(
            TerminalNamedKey::Insert
                | TerminalNamedKey::Delete
                | TerminalNamedKey::Home
                | TerminalNamedKey::End
                | TerminalNamedKey::PageUp
                | TerminalNamedKey::PageDown
                | TerminalNamedKey::ArrowUp
                | TerminalNamedKey::ArrowDown
                | TerminalNamedKey::ArrowLeft
                | TerminalNamedKey::ArrowRight
        ) | TerminalKey::Function(_)
    ) || matches!(&event.key, TerminalKey::Character(_) if event.text.as_deref().unwrap_or_default().is_empty())
}

fn kitty_base(
    event: &TerminalKeyEvent,
    mode: TermMode,
    has_associated_text: bool,
) -> Option<(String, char)> {
    if let TerminalKey::Function(function) = event.key {
        if function == 3 {
            return Some(("13".to_string(), '~'));
        }
        if (13..=35).contains(&function) {
            return Some(((57363_u32 + u32::from(function)).to_string(), 'u'));
        }
    }

    let one_based = if event.modifiers.is_empty()
        && !matches!(event.state, KeyState::Repeated | KeyState::Released)
        && !has_associated_text
    {
        ""
    } else {
        "1"
    };
    match event.key {
        TerminalKey::Named(named) => {
            let base = match named {
                TerminalNamedKey::PageUp => ("5", '~'),
                TerminalNamedKey::PageDown => ("6", '~'),
                TerminalNamedKey::Insert => ("2", '~'),
                TerminalNamedKey::Delete => ("3", '~'),
                TerminalNamedKey::Home => (one_based, 'H'),
                TerminalNamedKey::End => (one_based, 'F'),
                TerminalNamedKey::ArrowLeft => (one_based, 'D'),
                TerminalNamedKey::ArrowRight => (one_based, 'C'),
                TerminalNamedKey::ArrowUp => (one_based, 'A'),
                TerminalNamedKey::ArrowDown => (one_based, 'B'),
                TerminalNamedKey::Tab => ("9", 'u'),
                TerminalNamedKey::Enter => ("13", 'u'),
                TerminalNamedKey::Escape => ("27", 'u'),
                TerminalNamedKey::Backspace => ("127", 'u'),
            };
            return Some((base.0.to_string(), base.1));
        }
        TerminalKey::Function(function) => {
            let base = match function {
                1 => (one_based, 'P'),
                2 => (one_based, 'Q'),
                3 => unreachable!(),
                4 => (one_based, 'S'),
                5 => ("15", '~'),
                6 => ("17", '~'),
                7 => ("18", '~'),
                8 => ("19", '~'),
                9 => ("20", '~'),
                10 => ("21", '~'),
                11 => ("23", '~'),
                12 => ("24", '~'),
                _ => return None,
            };
            return Some((base.0.to_string(), base.1));
        }
        TerminalKey::Character(_) => {}
    }

    let TerminalKey::Character(ref key) = event.key else {
        return None;
    };
    let unicode_key = event.base_key.or_else(|| key.chars().next());
    let alternate_key = event
        .text
        .as_deref()
        .filter(|text| text.chars().count() == 1)
        .and_then(|text| text.chars().next());
    let Some(unicode_key) = unicode_key else {
        if mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) && has_associated_text {
            return Some(("0".to_string(), 'u'));
        }
        return None;
    };
    let unicode_code = u32::from(unicode_key);
    let alternate_code = alternate_key.map(u32::from).unwrap_or(unicode_code);
    let base = if mode.contains(TermMode::REPORT_ALTERNATE_KEYS) && alternate_code != unicode_code {
        format!("{unicode_code}:{alternate_code}")
    } else {
        unicode_code.to_string()
    };
    Some((base, 'u'))
}

fn encode_legacy(event: &TerminalKeyEvent, mode: TermMode) -> Option<SmallVec<[u8; 32]>> {
    if event.state == KeyState::Released {
        return None;
    }
    if event.prefer_character_input {
        return event
            .text
            .as_deref()
            .filter(|text| !text.is_empty())
            .map(|text| SmallVec::from_slice(text.as_bytes()));
    }

    let modifiers = event.modifiers;
    match event.key {
        TerminalKey::Named(named) => encode_legacy_named(named, modifiers, mode),
        TerminalKey::Function(function) => encode_legacy_function(function, modifiers),
        TerminalKey::Character(ref key) => {
            if modifiers.super_key {
                return None;
            }
            let mut bytes = SmallVec::<[u8; 32]>::new();
            if modifiers.alt {
                bytes.push(b'\x1b');
            }
            if modifiers.control {
                let character = event.base_key.or_else(|| key.chars().next())?;
                bytes.push(control_byte(character)?);
            } else {
                let text = event.text.as_deref().unwrap_or(key);
                if text.is_empty() {
                    return None;
                }
                bytes.extend_from_slice(text.as_bytes());
            }
            Some(bytes)
        }
    }
}

fn encode_legacy_named(
    key: TerminalNamedKey,
    modifiers: TerminalModifiers,
    mode: TermMode,
) -> Option<SmallVec<[u8; 32]>> {
    let mut bytes = SmallVec::<[u8; 32]>::new();
    match key {
        TerminalNamedKey::Escape
        | TerminalNamedKey::Enter
        | TerminalNamedKey::Tab
        | TerminalNamedKey::Backspace => {
            if modifiers.alt {
                bytes.push(b'\x1b');
            }
            match key {
                TerminalNamedKey::Escape => bytes.push(b'\x1b'),
                TerminalNamedKey::Enter => bytes.push(b'\r'),
                TerminalNamedKey::Backspace => bytes.push(0x7f),
                TerminalNamedKey::Tab if modifiers.shift => bytes.extend_from_slice(b"\x1b[Z"),
                TerminalNamedKey::Tab => bytes.push(b'\t'),
                _ => unreachable!(),
            }
            return Some(bytes);
        }
        _ => {}
    }

    let final_character = match key {
        TerminalNamedKey::Home => 'H',
        TerminalNamedKey::End => 'F',
        TerminalNamedKey::ArrowUp => 'A',
        TerminalNamedKey::ArrowDown => 'B',
        TerminalNamedKey::ArrowRight => 'C',
        TerminalNamedKey::ArrowLeft => 'D',
        TerminalNamedKey::Insert
        | TerminalNamedKey::Delete
        | TerminalNamedKey::PageUp
        | TerminalNamedKey::PageDown
        | TerminalNamedKey::Escape
        | TerminalNamedKey::Enter
        | TerminalNamedKey::Tab
        | TerminalNamedKey::Backspace => '\0',
    };
    if final_character != '\0' {
        if modifiers.is_empty() && mode.contains(TermMode::APP_CURSOR) {
            let sequence = format!("\x1bO{final_character}");
            return Some(SmallVec::from_slice(sequence.as_bytes()));
        }
        if modifiers.is_empty() {
            let sequence = format!("\x1b[{final_character}");
            return Some(SmallVec::from_slice(sequence.as_bytes()));
        }
        let sequence = format!("\x1b[1;{}{final_character}", modifiers.kitty_parameter());
        return Some(SmallVec::from_slice(sequence.as_bytes()));
    }

    let number = match key {
        TerminalNamedKey::Insert => 2,
        TerminalNamedKey::Delete => 3,
        TerminalNamedKey::PageUp => 5,
        TerminalNamedKey::PageDown => 6,
        _ => return None,
    };
    let sequence = if modifiers.is_empty() {
        format!("\x1b[{number}~")
    } else {
        format!("\x1b[{number};{}~", modifiers.kitty_parameter())
    };
    Some(SmallVec::from_slice(sequence.as_bytes()))
}

fn encode_legacy_function(
    function: u8,
    modifiers: TerminalModifiers,
) -> Option<SmallVec<[u8; 32]>> {
    if (1..=4).contains(&function) {
        let final_character = char::from(b'P' + function - 1);
        let sequence = if modifiers.is_empty() {
            format!("\x1bO{final_character}")
        } else {
            format!("\x1b[1;{}{final_character}", modifiers.kitty_parameter())
        };
        return Some(SmallVec::from_slice(sequence.as_bytes()));
    }
    let number = match function {
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        12 => 24,
        13 => 25,
        14 => 26,
        15 => 28,
        16 => 29,
        17 => 31,
        18 => 32,
        19 => 33,
        20 => 34,
        _ => return None,
    };
    let sequence = if modifiers.is_empty() {
        format!("\x1b[{number}~")
    } else {
        format!("\x1b[{number};{}~", modifiers.kitty_parameter())
    };
    Some(SmallVec::from_slice(sequence.as_bytes()))
}

/// Prepare text for a terminal paste without intermediate payload allocations.
pub fn paste(text: &str, bracketed: bool, mode: TermMode) -> Bytes {
    if !bracketed {
        return Bytes::copy_from_slice(text.as_bytes());
    }

    let bracketed_mode = mode.contains(TermMode::BRACKETED_PASTE);
    let wrapper_bytes = if bracketed_mode { 12 } else { 0 };
    let mut payload = BytesMut::with_capacity(text.len() + wrapper_bytes);

    if bracketed_mode {
        payload.extend_from_slice(b"\x1b[200~");
        payload.extend(
            text.as_bytes()
                .iter()
                .copied()
                .filter(|byte| !matches!(byte, b'\x1b' | b'\x03')),
        );
        payload.extend_from_slice(b"\x1b[201~");
        return payload.freeze();
    }

    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                payload.extend_from_slice(b"\r");
                index += 2;
            }
            b'\n' => {
                payload.extend_from_slice(b"\r");
                index += 1;
            }
            byte => {
                payload.extend_from_slice(&[byte]);
                index += 1;
            }
        }
    }
    payload.freeze()
}

fn control_byte(character: char) -> Option<u8> {
    match character {
        'a'..='z' | 'A'..='Z' => Some(character.to_ascii_uppercase() as u8 - b'@'),
        ' ' | '@' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '/' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn append_kitty_codepoints(payload: &mut String, text: &str) {
    for (index, character) in text.chars().enumerate() {
        if index > 0 {
            payload.push(':');
        }
        let _ = write!(payload, "{}", u32::from(character));
    }
}

fn contains_control_character(text: &str) -> bool {
    text.chars()
        .any(|codepoint| codepoint <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&codepoint))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        key: TerminalKey,
        text: Option<&str>,
        modifiers: TerminalModifiers,
    ) -> TerminalKeyEvent {
        TerminalKeyEvent {
            base_key: match &key {
                TerminalKey::Character(key) if key.chars().count() == 1 => key.chars().next(),
                _ => None,
            },
            key,
            text: text.map(ToOwned::to_owned),
            modifiers,
            state: KeyState::Pressed,
            prefer_character_input: false,
        }
    }

    fn encoded(event: &TerminalKeyEvent, mode: TermMode) -> Vec<u8> {
        encode_key(event, mode).unwrap().to_vec()
    }

    #[test]
    fn legacy_key_vectors_match_alacritty() {
        let none = TerminalModifiers::default();
        assert_eq!(
            encoded(
                &event(
                    TerminalKey::Named(TerminalNamedKey::Enter),
                    Some("\r"),
                    none
                ),
                TermMode::empty(),
            ),
            b"\r"
        );
        assert_eq!(
            encoded(
                &event(
                    TerminalKey::Character("c".to_string()),
                    Some("c"),
                    TerminalModifiers {
                        control: true,
                        ..none
                    },
                ),
                TermMode::empty(),
            ),
            b"\x03"
        );
        assert_eq!(
            encoded(
                &event(
                    TerminalKey::Character("x".to_string()),
                    Some("x"),
                    TerminalModifiers { alt: true, ..none },
                ),
                TermMode::empty(),
            ),
            b"\x1bx"
        );
        assert_eq!(
            encoded(
                &event(TerminalKey::Named(TerminalNamedKey::ArrowUp), None, none,),
                TermMode::APP_CURSOR,
            ),
            b"\x1bOA"
        );
        assert_eq!(
            encoded(
                &event(
                    TerminalKey::Named(TerminalNamedKey::ArrowUp),
                    None,
                    TerminalModifiers {
                        control: true,
                        ..none
                    },
                ),
                TermMode::APP_CURSOR,
            ),
            b"\x1b[1;5A"
        );
        assert_eq!(
            encoded(
                &event(TerminalKey::Function(1), None, none),
                TermMode::empty()
            ),
            b"\x1bOP"
        );
        assert_eq!(
            encoded(
                &event(
                    TerminalKey::Function(5),
                    None,
                    TerminalModifiers { alt: true, ..none },
                ),
                TermMode::empty(),
            ),
            b"\x1b[15;3~"
        );
        assert_eq!(
            encoded(
                &event(
                    TerminalKey::Named(TerminalNamedKey::Tab),
                    Some("\t"),
                    TerminalModifiers {
                        shift: true,
                        alt: true,
                        ..none
                    },
                ),
                TermMode::empty(),
            ),
            b"\x1b\x1b[Z"
        );
        assert_eq!(
            encoded(
                &event(TerminalKey::Character("1".to_string()), Some("1"), none),
                TermMode::APP_KEYPAD,
            ),
            b"1"
        );
        let mut alt_gr = event(
            TerminalKey::Character("e".to_string()),
            Some("€"),
            TerminalModifiers {
                alt: true,
                control: true,
                ..none
            },
        );
        alt_gr.prefer_character_input = true;
        assert_eq!(
            encoded(&alt_gr, TermMode::DISAMBIGUATE_ESC_CODES),
            "€".as_bytes()
        );
    }

    #[test]
    fn kitty_key_vectors_match_alacritty() {
        let none = TerminalModifiers::default();
        assert_eq!(
            encoded(
                &event(
                    TerminalKey::Named(TerminalNamedKey::Escape),
                    Some("\x1b"),
                    none
                ),
                TermMode::DISAMBIGUATE_ESC_CODES,
            ),
            b"\x1b[27u"
        );
        assert_eq!(
            encoded(
                &event(
                    TerminalKey::Character("a".to_string()),
                    Some("a"),
                    TerminalModifiers {
                        control: true,
                        ..none
                    },
                ),
                TermMode::DISAMBIGUATE_ESC_CODES,
            ),
            b"\x1b[97;5u"
        );
        let mut shifted = event(
            TerminalKey::Character("a".to_string()),
            Some("A"),
            TerminalModifiers {
                shift: true,
                ..none
            },
        );
        assert_eq!(
            encoded(
                &shifted,
                TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ALTERNATE_KEYS,
            ),
            b"\x1b[97:65;2u"
        );
        shifted.state = KeyState::Repeated;
        assert_eq!(
            encoded(
                &shifted,
                TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_EVENT_TYPES,
            ),
            b"\x1b[97;2:2u"
        );
        let mut arrow = event(TerminalKey::Named(TerminalNamedKey::ArrowUp), None, none);
        arrow.state = KeyState::Released;
        assert_eq!(
            encoded(&arrow, TermMode::REPORT_EVENT_TYPES),
            b"\x1b[1;1:3A"
        );
        assert_eq!(
            encoded(
                &event(TerminalKey::Function(3), None, none),
                TermMode::DISAMBIGUATE_ESC_CODES,
            ),
            b"\x1b[13~"
        );
        assert_eq!(
            encoded(
                &event(TerminalKey::Function(1), None, none),
                TermMode::REPORT_EVENT_TYPES,
            ),
            b"\x1bOP"
        );
        assert_eq!(
            encoded(
                &event(TerminalKey::Named(TerminalNamedKey::ArrowUp), None, none,),
                TermMode::APP_CURSOR | TermMode::REPORT_ALL_KEYS_AS_ESC,
            ),
            b"\x1bOA"
        );
        let associated = event(TerminalKey::Character("e".to_string()), Some("é"), none);
        assert_eq!(
            encoded(
                &associated,
                TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ASSOCIATED_TEXT,
            ),
            b"\x1b[101;1;233u"
        );
        let mut enter = event(
            TerminalKey::Named(TerminalNamedKey::Enter),
            Some("\r"),
            none,
        );
        assert_eq!(
            encoded(&enter, TermMode::REPORT_ALL_KEYS_AS_ESC),
            b"\x1b[13u"
        );
        enter.state = KeyState::Released;
        assert_eq!(
            encoded(
                &enter,
                TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_EVENT_TYPES,
            ),
            b"\x1b[13;1:3u"
        );
    }

    #[test]
    fn kitty_text_input_encodes_ime_commits_as_pure_text_events() {
        let mode = TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ASSOCIATED_TEXT;
        assert_eq!(
            encode_text_input("你好", mode).unwrap().as_ref(),
            b"\x1b[0;;20320:22909u"
        );
        assert_eq!(
            encode_text_input("你好", TermMode::DISAMBIGUATE_ESC_CODES)
                .unwrap()
                .as_ref(),
            "你好".as_bytes()
        );
        assert_eq!(
            encode_text_input("a\n", mode).unwrap().as_ref(),
            b"a\n",
            "control characters cannot be embedded as Kitty associated text"
        );
        assert!(encode_text_input("", mode).is_none());

        let mut character = event(
            TerminalKey::Character("a".to_string()),
            Some("å"),
            TerminalModifiers::default(),
        );
        character.prefer_character_input = true;
        assert_eq!(encoded(&character, mode), b"\x1b[97;1;229u");
    }

    #[test]
    fn paste_filters_and_normalizes_like_alacritty() {
        assert_eq!(
            paste(
                "safe\x1b[201~\x03text\r\nnext\nlast",
                true,
                TermMode::BRACKETED_PASTE,
            ),
            Bytes::from_static(b"\x1b[200~safe[201~text\r\nnext\nlast\x1b[201~")
        );
        assert_eq!(
            paste("one\r\ntwo\nthree\rfour", true, TermMode::empty()),
            Bytes::from_static(b"one\rtwo\rthree\rfour")
        );
        assert_eq!(
            paste("raw\n\x1b\x03€", false, TermMode::BRACKETED_PASTE),
            Bytes::copy_from_slice("raw\n\x1b\x03€".as_bytes())
        );
    }

    #[test]
    fn gpui_conversion_preserves_text_and_super_modifier() {
        let keystroke = Keystroke {
            key: "1".to_string(),
            key_char: Some("!".to_string()),
            modifiers: gpui::Modifiers {
                shift: true,
                platform: true,
                ..gpui::Modifiers::default()
            },
        };
        let event =
            TerminalKeyEvent::from_gpui_keystroke(&keystroke, KeyState::Pressed, false).unwrap();
        assert_eq!(event.base_key, Some('1'));
        assert_eq!(event.text.as_deref(), Some("!"));
        assert!(event.modifiers.super_key);
    }
}
