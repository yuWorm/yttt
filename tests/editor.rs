use gpui_component::highlighter::LanguageRegistry;
use yttt::ui::editor::{
    CodeEditorConfig, CodeEditorState, EditorDiagnostic, EditorDiagnosticSeverity,
    register_builtin_editor_languages,
};

#[test]
fn toml_language_registration_registers_toml_highlighter() {
    register_builtin_editor_languages();

    assert!(LanguageRegistry::singleton().language("toml").is_some());
}

#[test]
fn code_editor_state_tracks_value_dirty_and_errors() {
    let mut state = CodeEditorState::new(
        "/tmp/layout.toml",
        CodeEditorConfig::new("Edit layout TOML", "toml"),
        "name = \"yttt\"",
    );

    assert_eq!(state.path().to_string_lossy(), "/tmp/layout.toml");
    assert_eq!(state.language(), "toml");
    assert_eq!(state.value(), "name = \"yttt\"");
    assert!(!state.is_dirty());
    assert_eq!(state.error(), None);

    state.set_error("parse failed");
    assert_eq!(state.error(), Some("parse failed"));

    state.set_value("name = \"changed\"");
    assert_eq!(state.value(), "name = \"changed\"");
    assert!(state.is_dirty());
    assert_eq!(state.error(), None);

    state.mark_saved();
    assert!(!state.is_dirty());
}

#[test]
fn code_editor_state_tracks_and_clears_diagnostics() {
    let mut state = CodeEditorState::new(
        "/tmp/layout.toml",
        CodeEditorConfig::new("Edit layout TOML", "toml"),
        "name = \"yttt\"",
    );

    state.set_diagnostics(vec![EditorDiagnostic::new(
        EditorDiagnosticSeverity::Error,
        "toml",
        "failed to parse TOML",
    )]);

    assert_eq!(state.diagnostics().len(), 1);
    assert_eq!(
        state.diagnostics()[0].severity,
        EditorDiagnosticSeverity::Error
    );
    assert_eq!(state.diagnostics()[0].source, "toml");

    state.set_value("name = \"fixed\"");

    assert!(state.diagnostics().is_empty());
}
