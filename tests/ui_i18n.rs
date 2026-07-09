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

#[test]
fn ui_text_returns_close_project_dialog_labels() {
    let english = UiText::new(Locale::English);
    let chinese = UiText::new(Locale::Chinese);

    assert_eq!(english.get(UiTextKey::CloseProjectTitle), "Close project?");
    assert_eq!(
        english.get(UiTextKey::CloseProjectBody),
        "Running terminal processes will be stopped."
    );
    assert_eq!(english.get(UiTextKey::Cancel), "Cancel");
    assert_eq!(english.get(UiTextKey::CloseProjectAction), "Close Project");

    assert_eq!(chinese.get(UiTextKey::CloseProjectTitle), "关闭项目？");
    assert_eq!(
        chinese.get(UiTextKey::CloseProjectBody),
        "正在运行的终端进程会被停止。"
    );
    assert_eq!(chinese.get(UiTextKey::Cancel), "取消");
    assert_eq!(chinese.get(UiTextKey::CloseProjectAction), "关闭项目");
}

#[test]
fn ui_text_returns_rename_tab_dialog_labels() {
    let english = UiText::new(Locale::English);
    let chinese = UiText::new(Locale::Chinese);

    assert_eq!(english.get(UiTextKey::RenameTabTitle), "Rename tab");
    assert_eq!(english.get(UiTextKey::RenameTabAction), "Rename");
    assert_eq!(
        english.get(UiTextKey::RenameTabHint),
        "Enter to rename, Escape to cancel"
    );
    assert_eq!(chinese.get(UiTextKey::RenameTabTitle), "重命名标签页");
    assert_eq!(chinese.get(UiTextKey::RenameTabAction), "重命名");
    assert_eq!(
        chinese.get(UiTextKey::RenameTabHint),
        "回车重命名，Esc 取消"
    );
}

#[test]
fn ui_text_returns_project_empty_terminal_labels() {
    let english = UiText::english();
    let chinese = UiText::new(Locale::Chinese);

    assert_eq!(english.get(UiTextKey::NoTerminalTabs), "No terminal tabs");
    assert_eq!(english.get(UiTextKey::NewTab), "New Tab");
    assert_eq!(chinese.get(UiTextKey::NoTerminalTabs), "暂无终端标签页");
    assert_eq!(chinese.get(UiTextKey::NewTab), "新建标签页");
}

#[test]
fn ui_text_returns_notification_action_labels() {
    let english = UiText::english();
    let chinese = UiText::new(Locale::Chinese);

    assert_eq!(english.get(UiTextKey::OpenNotificationTarget), "Open");
    assert_eq!(chinese.get(UiTextKey::OpenNotificationTarget), "打开");
}
