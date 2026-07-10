mod document;
mod file_io;
mod language;
mod language_catalog;
mod runtime;
mod state;
mod view;
mod workspace;

pub use document::{
    EditorAppearance, ProjectEditorDocument, ProjectEditorDocumentEvent, ProjectEditorModel,
    ProjectEditorSaveState, SaveRequest,
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
pub use runtime::{ProjectEditorRuntime, ProjectFileLoadRequest};
pub use state::{
    CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, EditorDiagnostic,
    EditorDiagnosticSeverity, EditorLanguageService, EditorRange,
};
pub use view::code_editor_input_state;
pub use workspace::{DocumentId, ProjectEditorWorkspaceState, ProjectWorkItemSession, WorkItemId};
