use gpui::Keystroke;
use yttt_terminal::input::keystroke_to_bytes;

fn keystroke_with_platform_key(parse: &str, key: &str) -> Keystroke {
    let mut keystroke = Keystroke::parse(parse).unwrap();
    keystroke.key = key.to_string();
    keystroke
}

#[test]
fn terminal_input_accepts_platform_special_key_names() {
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("tab", "Tab"),
            Default::default()
        ),
        Some(b"\t".to_vec())
    );
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("enter", "Enter"),
            Default::default()
        ),
        Some(b"\r".to_vec())
    );
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("escape", "Escape"),
            Default::default()
        ),
        Some(b"\x1b".to_vec())
    );
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("up", "ArrowUp"),
            Default::default()
        ),
        Some(b"\x1b[A".to_vec())
    );
}

#[test]
fn terminal_input_accepts_uppercase_control_key_names() {
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("ctrl-c", "C"),
            Default::default()
        ),
        Some(vec![0x03])
    );
}

#[test]
fn terminal_input_ignores_platform_shortcuts() {
    let mut keystroke = keystroke_with_platform_key("cmd-s", "s");
    keystroke.key_char = None;

    assert_eq!(keystroke_to_bytes(&keystroke, Default::default()), None);
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("cmd-tab", "Tab"),
            Default::default()
        ),
        None
    );
}
