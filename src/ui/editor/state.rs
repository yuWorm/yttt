use std::{
    borrow::Cow,
    path::{Path, PathBuf},
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeEditorConfig {
    title: String,
    language: String,
    placeholder: String,
    rows: usize,
    soft_wrap: bool,
    line_number: bool,
}

impl CodeEditorConfig {
    pub fn new(title: impl Into<String>, language: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            language: language.into(),
            placeholder: String::new(),
            rows: 24,
            soft_wrap: false,
            line_number: true,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn placeholder(&self) -> Cow<'_, str> {
        if self.placeholder.is_empty() {
            Cow::Owned(format!("Edit {}...", self.language))
        } else {
            Cow::Borrowed(&self.placeholder)
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
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
        let value = value.into();
        Self {
            path: path.into(),
            config,
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
        self.config.language()
    }

    pub fn value(&self) -> &str {
        &self.value
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
        self.saved_value = self.value.clone();
        self.clear_error();
        self.clear_diagnostics();
    }
}
