mod language;
mod state;
mod view;

pub use language::register_builtin_editor_languages;
pub use state::{
    CodeEditorConfig, CodeEditorState, EditorDiagnostic, EditorDiagnosticSeverity,
    EditorLanguageService, EditorRange,
};
pub use view::code_editor_input_state;
