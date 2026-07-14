use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use crate::config::settings::EditorSettings;

use super::{
    EditorLanguageCatalog, EditorLanguageId, EditorLanguageResolution,
    EditorLanguageResolutionSource,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorRange {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl EditorRange {
    pub fn unknown() -> Self {
        Self {
            start_line: 0,
            start_column: 0,
            end_line: 0,
            end_column: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorDiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorDiagnostic {
    pub severity: EditorDiagnosticSeverity,
    pub source: String,
    pub message: String,
    pub range: EditorRange,
}

impl EditorDiagnostic {
    pub fn new(
        severity: EditorDiagnosticSeverity,
        source: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            source: source.into(),
            message: message.into(),
            range: EditorRange::unknown(),
        }
    }
}

pub trait EditorLanguageService {
    fn language(&self) -> &str;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeEditorLanguageMode {
    Auto,
    Explicit(EditorLanguageId),
}

impl CodeEditorLanguageMode {
    fn label(self) -> &'static str {
        match self {
            Self::Auto => "code",
            Self::Explicit(language_id) => language_id.as_str(),
        }
    }
}

impl From<EditorLanguageId> for CodeEditorLanguageMode {
    fn from(language_id: EditorLanguageId) -> Self {
        Self::Explicit(language_id)
    }
}

impl From<&str> for CodeEditorLanguageMode {
    fn from(language: &str) -> Self {
        EditorLanguageId::parse(language)
            .map(Self::Explicit)
            .unwrap_or(Self::Explicit(EditorLanguageId::PlainText))
    }
}

impl From<String> for CodeEditorLanguageMode {
    fn from(language: String) -> Self {
        Self::from(language.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeEditorConfig {
    title: String,
    language_mode: CodeEditorLanguageMode,
    placeholder: String,
    rows: usize,
    tab_size: usize,
    soft_wrap: bool,
    line_number: bool,
}

impl CodeEditorConfig {
    pub fn new(title: impl Into<String>, language: impl Into<CodeEditorLanguageMode>) -> Self {
        Self {
            title: title.into(),
            language_mode: language.into(),
            placeholder: String::new(),
            rows: 24,
            tab_size: 4,
            soft_wrap: false,
            line_number: true,
        }
    }

    pub fn with_editor_settings(mut self, settings: &EditorSettings) -> Self {
        self.tab_size = settings.tab_size;
        self.soft_wrap = settings.soft_wrap;
        self.line_number = settings.line_numbers;
        self
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn language_mode(&self) -> CodeEditorLanguageMode {
        self.language_mode
    }

    pub fn placeholder(&self) -> Cow<'_, str> {
        if self.placeholder.is_empty() {
            Cow::Owned(format!("Edit {}...", self.language_mode.label()))
        } else {
            Cow::Borrowed(&self.placeholder)
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn tab_size(&self) -> usize {
        self.tab_size
    }

    pub fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    pub fn line_number(&self) -> bool {
        self.line_number
    }

    pub fn placeholder_text(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn with_rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub fn with_tab_size(mut self, tab_size: usize) -> Self {
        self.tab_size = tab_size;
        self
    }

    pub fn with_soft_wrap(mut self, soft_wrap: bool) -> Self {
        self.soft_wrap = soft_wrap;
        self
    }

    pub fn with_line_number(mut self, line_number: bool) -> Self {
        self.line_number = line_number;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeEditorState {
    path: PathBuf,
    config: CodeEditorConfig,
    resolved_language: EditorLanguageResolution,
    value: String,
    saved_value: String,
    error: Option<String>,
    diagnostics: Vec<EditorDiagnostic>,
}

impl CodeEditorState {
    pub fn new(
        path: impl Into<PathBuf>,
        config: CodeEditorConfig,
        value: impl Into<String>,
    ) -> Self {
        Self::new_with_catalog(path, config, value, &EditorLanguageCatalog::builtin())
    }

    pub fn new_with_catalog(
        path: impl Into<PathBuf>,
        config: CodeEditorConfig,
        value: impl Into<String>,
        catalog: &EditorLanguageCatalog,
    ) -> Self {
        let path = path.into();
        let value = value.into();
        let resolved_language =
            resolve_editor_language(&path, &value, config.language_mode, catalog);
        Self {
            path,
            config,
            resolved_language,
            saved_value: value.clone(),
            value,
            error: None,
            diagnostics: Vec::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn config(&self) -> &CodeEditorConfig {
        &self.config
    }

    pub fn language(&self) -> &str {
        &self.resolved_language.highlighter_name
    }

    pub fn language_id(&self) -> EditorLanguageId {
        self.resolved_language.language_id
    }

    pub fn resolved_language(&self) -> &EditorLanguageResolution {
        &self.resolved_language
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn saved_value(&self) -> &str {
        &self.saved_value
    }

    pub fn is_dirty(&self) -> bool {
        self.value != self.saved_value
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn diagnostics(&self) -> &[EditorDiagnostic] {
        &self.diagnostics
    }

    pub fn relocate(&mut self, path: impl Into<PathBuf>, title: impl Into<String>) {
        self.path = path.into();
        self.config.title = title.into();
        self.resolved_language = resolve_editor_language(
            &self.path,
            &self.value,
            self.config.language_mode,
            &EditorLanguageCatalog::builtin(),
        );
    }

    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.error = None;
        self.diagnostics.clear();
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    pub fn set_diagnostics(&mut self, diagnostics: Vec<EditorDiagnostic>) {
        self.diagnostics = diagnostics;
    }

    pub fn clear_diagnostics(&mut self) {
        self.diagnostics.clear();
    }

    pub fn mark_saved(&mut self) {
        self.mark_value_saved(self.value.clone());
    }

    pub fn mark_value_saved(&mut self, value: impl Into<String>) {
        self.saved_value = value.into();
        self.clear_error();
        self.clear_diagnostics();
    }

    pub fn replace_from_disk(&mut self, value: impl Into<String>) {
        let value = value.into();
        self.value = value.clone();
        self.saved_value = value;
        self.clear_error();
        self.clear_diagnostics();
    }
}

fn resolve_editor_language(
    path: &Path,
    value: &str,
    mode: CodeEditorLanguageMode,
    catalog: &EditorLanguageCatalog,
) -> EditorLanguageResolution {
    match mode {
        CodeEditorLanguageMode::Auto => catalog.resolve_for_path(path, Some(value)),
        CodeEditorLanguageMode::Explicit(language_id) => {
            let mut resolution = catalog.resolve_explicit(language_id);
            resolution.source = EditorLanguageResolutionSource::Explicit;
            resolution
        }
    }
}
