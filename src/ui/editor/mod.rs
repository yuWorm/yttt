mod language;
mod language_catalog;
mod state;
mod view;

pub use language::register_builtin_editor_languages;
pub use language_catalog::{
    EditorLanguageCatalog, EditorLanguageDefinition, EditorLanguageId, EditorLanguageResolution,
    EditorLanguageResolutionSource,
};
pub use state::{
    CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, EditorDiagnostic,
    EditorDiagnosticSeverity, EditorLanguageService, EditorRange,
};
pub use view::code_editor_input_state;
