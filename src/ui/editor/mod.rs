mod breadcrumbs;
mod document;
mod file_io;
mod language;
mod language_catalog;
mod readonly_code_view;
mod runtime;
mod state;
mod view;
mod vim;
mod workspace;

pub use breadcrumbs::{EditorSymbol, EditorSymbolKind, breadcrumbs_at, document_symbols};
pub use document::{
    EditorAppearance, MarkdownDocumentConfig, ProjectEditorDocument, ProjectEditorDocumentEvent,
    ProjectEditorModel, ProjectEditorSaveState, SaveRequest,
};
pub use file_io::{
    CurrentDiskState, DiskFingerprint, LoadedProjectFile, MAX_PROJECT_FILE_BYTES,
    ProjectFileIoError, SaveMode, SaveProjectFileOutcome, project_relative_path, read_project_file,
    save_project_file,
};
pub use language::register_builtin_editor_languages;
pub use language_catalog::{
    EditorLanguageCatalog, EditorLanguageDefinition, EditorLanguageId, EditorLanguageResolution,
    EditorLanguageResolutionSource,
};
pub use readonly_code_view::{ReadonlyCodeRow, ReadonlyCodeRowKind, ReadonlyCodeView};
pub use runtime::{ProjectEditorRuntime, ProjectFileLoadRequest};
pub use state::{
    CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, EditorDiagnostic,
    EditorDiagnosticSeverity, EditorLanguageService, EditorRange,
};
pub use view::{code_editor_input_state, styled_code_editor_input};
pub use vim::{VimMode, init as init_vim_mode};
pub use workspace::{DocumentId, ProjectEditorWorkspaceState, ProjectWorkItemSession, WorkItemId};
