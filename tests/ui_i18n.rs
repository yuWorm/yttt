use yttt::ui::i18n::{Locale, UiText, UiTextKey};

#[test]
fn ui_text_returns_default_shell_labels() {
    let text = UiText::new(Locale::English);

    assert_eq!(text.get(UiTextKey::OpenDirectory), "Open Directory");
    assert_eq!(text.get(UiTextKey::OpenRecent), "Open Recent");
    assert_eq!(text.get(UiTextKey::CommandPalette), "Command Palette");
    assert_eq!(text.get(UiTextKey::Projects), "Projects");
}

#[test]
fn ui_text_supports_chinese_shell_labels() {
    let text = UiText::new(Locale::Chinese);

    assert_eq!(text.get(UiTextKey::OpenDirectory), "打开目录");
    assert_eq!(text.get(UiTextKey::OpenRecent), "打开最近项目");
    assert_eq!(text.get(UiTextKey::CommandPalette), "命令面板");
}
