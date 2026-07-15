use std::sync::Arc;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{InputEvent, InputState, Position, Search},
};
use gpui_markdown_editor::{
    MarkdownEditor, MarkdownEditorEnvironment, MarkdownEditorEvent, MarkdownEditorMode,
    MarkdownEditorOptions, MarkdownEditorStrings, MarkdownEditorTheme,
};

use crate::{
    config::settings::EditorSettings,
    ui::i18n::{UiText, UiTextKey},
};

use super::{
    CodeEditorState, DiskFingerprint, DocumentId, EditorSymbol, breadcrumbs_at,
    code_editor_input_state, document_symbols, styled_code_editor_input,
};

#[derive(Clone, Debug, PartialEq)]
pub struct EditorAppearance {
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub soft_wrap: bool,
    pub line_numbers: bool,
}
impl EditorAppearance {
    pub(crate) fn resolved_font_family(&self) -> gpui::SharedString {
        if self.font_family.is_empty() {
            ".SystemUIFont".into()
        } else {
            self.font_family.clone().into()
        }
    }
}

impl Default for EditorAppearance {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 14.0,
            line_height: 1.4,
            soft_wrap: false,
            line_numbers: true,
        }
    }
}

impl From<&EditorSettings> for EditorAppearance {
    fn from(settings: &EditorSettings) -> Self {
        Self {
            font_family: settings.font_family.clone(),
            font_size: settings.font_size,
            line_height: settings.line_height,
            soft_wrap: settings.soft_wrap,
            line_numbers: settings.line_numbers,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectEditorDocumentEvent {
    Changed { generation: u64 },
    Focused,
    Blurred,
    Error { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectEditorSaveState {
    Idle,
    Saving { generation: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveRequest {
    pub document_id: DocumentId,
    pub generation: u64,
    pub text: String,
    pub expected_fingerprint: DiskFingerprint,
}

#[derive(Clone, Debug)]
pub struct ProjectEditorModel {
    document_id: DocumentId,
    editor: CodeEditorState,
    disk_fingerprint: DiskFingerprint,
    generation: u64,
    save_state: ProjectEditorSaveState,
    external_dirty: bool,
}

impl ProjectEditorModel {
    pub fn new(
        document_id: DocumentId,
        editor: CodeEditorState,
        disk_fingerprint: DiskFingerprint,
    ) -> Self {
        Self {
            document_id,
            editor,
            disk_fingerprint,
            generation: 0,
            save_state: ProjectEditorSaveState::Idle,
            external_dirty: false,
        }
    }

    pub fn document_id(&self) -> &DocumentId {
        &self.document_id
    }

    pub fn relocate(&mut self, document_id: DocumentId, title: impl Into<String>) {
        self.editor
            .relocate(document_id.canonical_path.clone(), title);
        self.document_id = document_id;
        self.save_state = ProjectEditorSaveState::Idle;
    }

    pub fn editor(&self) -> &CodeEditorState {
        &self.editor
    }

    pub fn editor_mut(&mut self) -> &mut CodeEditorState {
        &mut self.editor
    }

    pub fn value(&self) -> &str {
        self.editor.value()
    }

    pub fn saved_value(&self) -> &str {
        self.editor.saved_value()
    }

    pub fn is_dirty(&self) -> bool {
        self.editor.is_dirty() || self.external_dirty
    }

    pub fn disk_fingerprint(&self) -> &DiskFingerprint {
        &self.disk_fingerprint
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn save_state(&self) -> &ProjectEditorSaveState {
        &self.save_state
    }

    pub fn on_input_changed(&mut self, value: impl Into<String>) -> u64 {
        let value = value.into();
        if value == self.editor.value() {
            return self.generation;
        }
        self.editor.set_value(value);
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn on_external_changed(&mut self) -> u64 {
        self.external_dirty = true;
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn sync_external_value(&mut self, value: impl Into<String>) {
        self.editor.set_value(value);
    }

    pub fn begin_save(&mut self) -> SaveRequest {
        let request = SaveRequest {
            document_id: self.document_id.clone(),
            generation: self.generation,
            text: self.editor.value().to_string(),
            expected_fingerprint: self.disk_fingerprint.clone(),
        };
        self.save_state = ProjectEditorSaveState::Saving {
            generation: request.generation,
        };
        request
    }

    fn begin_save_with_text(&mut self, text: String) -> SaveRequest {
        self.sync_external_value(text);
        self.begin_save()
    }

    pub fn finish_save(
        &mut self,
        request: &SaveRequest,
        disk_fingerprint: DiskFingerprint,
    ) -> bool {
        if request.document_id != self.document_id {
            return false;
        }
        self.editor.mark_value_saved(request.text.clone());
        if request.generation == self.generation {
            self.external_dirty = false;
        }
        self.disk_fingerprint = disk_fingerprint;
        if self.save_state
            == (ProjectEditorSaveState::Saving {
                generation: request.generation,
            })
        {
            self.save_state = ProjectEditorSaveState::Idle;
        }
        true
    }

    pub fn fail_save(&mut self, request: &SaveRequest, error: impl Into<String>) -> bool {
        if request.document_id != self.document_id
            || self.save_state
                != (ProjectEditorSaveState::Saving {
                    generation: request.generation,
                })
        {
            return false;
        }
        self.save_state = ProjectEditorSaveState::Idle;
        self.editor.set_error(error);
        true
    }

    pub fn cancel_save(&mut self, request: &SaveRequest) -> bool {
        if request.document_id != self.document_id
            || self.save_state
                != (ProjectEditorSaveState::Saving {
                    generation: request.generation,
                })
        {
            return false;
        }
        self.save_state = ProjectEditorSaveState::Idle;
        true
    }

    pub fn replace_from_disk(
        &mut self,
        value: impl Into<String>,
        disk_fingerprint: DiskFingerprint,
    ) {
        self.editor.replace_from_disk(value);
        self.external_dirty = false;
        self.disk_fingerprint = disk_fingerprint;
        self.generation = self.generation.wrapping_add(1);
        self.save_state = ProjectEditorSaveState::Idle;
    }
}

#[derive(Clone)]
pub struct MarkdownDocumentConfig {
    theme: Arc<MarkdownEditorTheme>,
    strings: Arc<MarkdownEditorStrings>,
    ui_text: UiText,
}

impl MarkdownDocumentConfig {
    pub fn new(
        theme: Arc<MarkdownEditorTheme>,
        strings: Arc<MarkdownEditorStrings>,
        ui_text: UiText,
    ) -> Self {
        Self {
            theme,
            strings,
            ui_text,
        }
    }
}

impl Default for MarkdownDocumentConfig {
    fn default() -> Self {
        Self {
            theme: Arc::new(MarkdownEditorTheme::default_theme()),
            strings: Arc::new(MarkdownEditorStrings::en_us()),
            ui_text: UiText::english(),
        }
    }
}

enum ProjectEditorSurface {
    Code {
        input: Entity<InputState>,
        _input_subscription: Subscription,
        _input_observer: Subscription,
    },
    Markdown {
        editor: Entity<MarkdownEditor>,
        _editor_subscription: Subscription,
    },
}

pub struct ProjectEditorDocument {
    model: ProjectEditorModel,
    surface: ProjectEditorSurface,
    appearance: EditorAppearance,
    markdown_config: MarkdownDocumentConfig,
    breadcrumb_header: String,
    symbols: Vec<EditorSymbol>,
    breadcrumbs: Vec<EditorSymbol>,
    breadcrumb_cursor_line: usize,
}

impl ProjectEditorDocument {
    pub fn new(
        model: ProjectEditorModel,
        appearance: EditorAppearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_markdown_config(
            model,
            appearance,
            MarkdownDocumentConfig::default(),
            window,
            cx,
        )
    }

    pub fn new_with_markdown_config(
        model: ProjectEditorModel,
        appearance: EditorAppearance,
        markdown_config: MarkdownDocumentConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_markdown = model.editor().language_id() == super::EditorLanguageId::Markdown;
        let symbols = if is_markdown {
            Vec::new()
        } else {
            document_symbols(model.editor().language_id(), model.value())
        };
        let breadcrumb_header = model.editor().config().title().to_string();
        let breadcrumbs = breadcrumbs_at(&symbols, 0);
        let surface = Self::new_surface(&model, &appearance, &markdown_config, window, cx);
        Self {
            model,
            surface,
            appearance,
            markdown_config,
            breadcrumb_header,
            symbols,
            breadcrumbs,
            breadcrumb_cursor_line: 0,
        }
    }

    fn new_surface(
        model: &ProjectEditorModel,
        appearance: &EditorAppearance,
        markdown_config: &MarkdownDocumentConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ProjectEditorSurface {
        if model.editor().language_id() == super::EditorLanguageId::Markdown {
            let environment = Self::markdown_environment_for(model, appearance, markdown_config);
            let editor = cx.new(|cx| {
                MarkdownEditor::new(
                    model.value().to_string(),
                    MarkdownEditorOptions {
                        environment,
                        ..MarkdownEditorOptions::default()
                    },
                    cx,
                )
            });
            let editor_subscription =
                cx.subscribe_in(&editor, window, Self::on_markdown_editor_event);
            ProjectEditorSurface::Markdown {
                editor,
                _editor_subscription: editor_subscription,
            }
        } else {
            let input = cx.new(|cx| code_editor_input_state(window, cx, model.editor()));
            input.update(cx, |input, input_cx| {
                input.set_soft_wrap(appearance.soft_wrap, window, input_cx);
                input.set_line_number(appearance.line_numbers, window, input_cx);
            });
            let input_subscription = cx.subscribe_in(&input, window, Self::on_input_event);
            let input_observer = cx.observe_in(&input, window, Self::on_input_notify);
            ProjectEditorSurface::Code {
                input,
                _input_subscription: input_subscription,
                _input_observer: input_observer,
            }
        }
    }

    fn markdown_environment_for(
        model: &ProjectEditorModel,
        appearance: &EditorAppearance,
        config: &MarkdownDocumentConfig,
    ) -> MarkdownEditorEnvironment {
        MarkdownEditorEnvironment {
            theme: config.theme.clone(),
            strings: config.strings.clone(),
            document_base_dir: model
                .document_id()
                .canonical_path
                .parent()
                .map(ToOwned::to_owned),
            show_source_line_numbers: appearance.line_numbers,
            ..MarkdownEditorEnvironment::default()
        }
    }

    pub fn with_breadcrumb_header(mut self, breadcrumb_header: impl Into<String>) -> Self {
        self.breadcrumb_header = breadcrumb_header.into();
        self
    }

    pub fn breadcrumb_header(&self) -> &str {
        &self.breadcrumb_header
    }

    pub fn model(&self) -> &ProjectEditorModel {
        &self.model
    }

    pub fn model_mut(&mut self) -> &mut ProjectEditorModel {
        &mut self.model
    }

    pub fn is_markdown(&self) -> bool {
        matches!(self.surface, ProjectEditorSurface::Markdown { .. })
    }

    pub fn input(&self) -> &Entity<InputState> {
        self.code_input()
            .expect("Markdown documents do not expose a code-editor input")
    }

    pub fn code_input(&self) -> Option<&Entity<InputState>> {
        match &self.surface {
            ProjectEditorSurface::Code { input, .. } => Some(input),
            ProjectEditorSurface::Markdown { .. } => None,
        }
    }

    pub fn markdown_editor(&self) -> Option<&Entity<MarkdownEditor>> {
        match &self.surface {
            ProjectEditorSurface::Code { .. } => None,
            ProjectEditorSurface::Markdown { editor, .. } => Some(editor),
        }
    }

    pub fn relocate(
        &mut self,
        document_id: DocumentId,
        breadcrumb_header: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_markdown = self.is_markdown();
        let markdown = match &self.surface {
            ProjectEditorSurface::Markdown { editor, .. } => Some(editor.read(cx).markdown(cx)),
            ProjectEditorSurface::Code { .. } => None,
        };
        if let Some(markdown) = markdown {
            self.model.sync_external_value(markdown);
        }

        let title = document_id
            .canonical_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| document_id.canonical_path.display().to_string());
        self.model.relocate(document_id, title);
        self.breadcrumb_header = breadcrumb_header.into();

        let is_markdown = self.model.editor().language_id() == super::EditorLanguageId::Markdown;
        if was_markdown != is_markdown {
            self.surface = Self::new_surface(
                &self.model,
                &self.appearance,
                &self.markdown_config,
                window,
                cx,
            );
        } else if let ProjectEditorSurface::Code { input, .. } = &self.surface {
            let language = self.model.editor().language().to_string();
            input.update(cx, |input, input_cx| {
                input.set_highlighter(language, input_cx);
            });
        } else {
            let environment = Self::markdown_environment_for(
                &self.model,
                &self.appearance,
                &self.markdown_config,
            );
            if let ProjectEditorSurface::Markdown { editor, .. } = &self.surface {
                editor.update(cx, |editor, editor_cx| {
                    editor.set_environment(environment, editor_cx);
                });
            }
        }

        self.refresh_breadcrumbs(self.breadcrumb_cursor_line);
        cx.notify();
    }

    pub fn appearance(&self) -> &EditorAppearance {
        &self.appearance
    }

    pub fn symbols(&self) -> &[EditorSymbol] {
        &self.symbols
    }

    pub fn breadcrumbs(&self) -> &[EditorSymbol] {
        &self.breadcrumbs
    }

    pub fn set_appearance(
        &mut self,
        appearance: EditorAppearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_appearance_with_markdown_config(
            appearance,
            self.markdown_config.clone(),
            window,
            cx,
        );
    }

    pub fn set_appearance_with_markdown_config(
        &mut self,
        appearance: EditorAppearance,
        markdown_config: MarkdownDocumentConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance = appearance;
        self.markdown_config = markdown_config;
        match &self.surface {
            ProjectEditorSurface::Code { input, .. } => {
                input.update(cx, |input, input_cx| {
                    input.set_soft_wrap(self.appearance.soft_wrap, window, input_cx);
                    input.set_line_number(self.appearance.line_numbers, window, input_cx);
                });
            }
            ProjectEditorSurface::Markdown { editor, .. } => {
                let environment = Self::markdown_environment_for(
                    &self.model,
                    &self.appearance,
                    &self.markdown_config,
                );
                editor.update(cx, |editor, editor_cx| {
                    editor.set_environment(environment, editor_cx);
                });
            }
        }
        cx.notify();
    }

    pub fn begin_save(&mut self, cx: &mut Context<Self>) -> SaveRequest {
        match &self.surface {
            ProjectEditorSurface::Code { .. } => self.model.begin_save(),
            ProjectEditorSurface::Markdown { editor, .. } => {
                let markdown = editor.read(cx).markdown(cx);
                self.model.begin_save_with_text(markdown)
            }
        }
    }

    pub fn replace_from_disk(
        &mut self,
        value: impl Into<String>,
        disk_fingerprint: DiskFingerprint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.into();
        self.model
            .replace_from_disk(value.clone(), disk_fingerprint);
        match &self.surface {
            ProjectEditorSurface::Code { input, .. } => {
                input.update(cx, |input, input_cx| {
                    input.set_value(value, window, input_cx)
                });
                self.refresh_breadcrumbs(0);
            }
            ProjectEditorSurface::Markdown { editor, .. } => {
                editor.update(cx, |editor, editor_cx| {
                    editor.replace_markdown(value, editor_cx);
                });
                self.symbols.clear();
                self.breadcrumbs.clear();
                self.breadcrumb_cursor_line = 0;
            }
        }
        cx.notify();
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.surface {
            ProjectEditorSurface::Code { input, .. } => {
                input.update(cx, |input, input_cx| input.focus(window, input_cx));
            }
            ProjectEditorSurface::Markdown { editor, .. } => {
                let selection = editor.read(cx).source_selection(cx);
                editor.update(cx, |editor, editor_cx| {
                    editor.set_source_selection(selection, editor_cx);
                });
                cx.emit(ProjectEditorDocumentEvent::Focused);
            }
        }
    }

    fn on_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let (value, cursor_line) = {
                    let input = input.read(cx);
                    (
                        input.value().to_string(),
                        input.cursor_position().line as usize,
                    )
                };
                let previous_generation = self.model.generation();
                let generation = self.model.on_input_changed(value);
                if generation != previous_generation {
                    self.refresh_breadcrumbs(cursor_line);
                    cx.emit(ProjectEditorDocumentEvent::Changed { generation });
                    cx.notify();
                }
            }
            InputEvent::Focus => {
                cx.emit(ProjectEditorDocumentEvent::Focused);
            }
            InputEvent::Blur => {
                cx.emit(ProjectEditorDocumentEvent::Blurred);
            }
            InputEvent::PressEnter { .. } => {}
        }
    }

    fn on_markdown_editor_event(
        &mut self,
        _editor: &Entity<MarkdownEditor>,
        event: &MarkdownEditorEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MarkdownEditorEvent::Changed { .. } => {
                let generation = self.model.on_external_changed();
                cx.emit(ProjectEditorDocumentEvent::Changed { generation });
                cx.notify();
            }
            MarkdownEditorEvent::ModeChanged { .. } | MarkdownEditorEvent::SelectionChanged(_) => {
                cx.notify();
            }
            MarkdownEditorEvent::OpenLinkRequested(request) => {
                let target = request.open_target.as_str();
                if target.starts_with("https://")
                    || target.starts_with("http://")
                    || target.starts_with("mailto:")
                    || target.starts_with("file://")
                {
                    cx.open_url(target);
                }
            }
            MarkdownEditorEvent::Error { message } => {
                cx.emit(ProjectEditorDocumentEvent::Error {
                    message: message.clone(),
                });
            }
        }
    }

    fn on_input_notify(
        &mut self,
        input: Entity<InputState>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cursor_line = input.read(cx).cursor_position().line as usize;
        if cursor_line != self.breadcrumb_cursor_line {
            self.breadcrumb_cursor_line = cursor_line;
            self.breadcrumbs = breadcrumbs_at(&self.symbols, cursor_line);
            cx.notify();
        }
    }

    fn refresh_breadcrumbs(&mut self, cursor_line: usize) {
        self.breadcrumb_cursor_line = cursor_line;
        if self.is_markdown() {
            self.symbols.clear();
            self.breadcrumbs.clear();
        } else {
            self.symbols = document_symbols(self.model.editor().language_id(), self.model.value());
            self.breadcrumbs = breadcrumbs_at(&self.symbols, cursor_line);
        }
    }

    fn focus_symbol(&mut self, symbol: EditorSymbol, window: &mut Window, cx: &mut Context<Self>) {
        let ProjectEditorSurface::Code { input, .. } = &self.surface else {
            return;
        };
        input.update(cx, |input, input_cx| {
            input.set_cursor_position(
                Position::new(symbol.start_line as u32, symbol.start_column as u32),
                window,
                input_cx,
            );
        });
        self.breadcrumb_cursor_line = symbol.start_line;
        self.breadcrumbs = breadcrumbs_at(&self.symbols, symbol.start_line);
        cx.notify();
    }
}

impl EventEmitter<ProjectEditorDocumentEvent> for ProjectEditorDocument {}

impl Render for ProjectEditorDocument {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let breadcrumb_hover = cx.theme().foreground;
        let mut breadcrumb_items = div()
            .id("editor-breadcrumb-items")
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .text_color(cx.theme().muted_foreground)
            .child(self.breadcrumb_header.clone());
        for symbol in self.breadcrumbs.clone() {
            let symbol_id = format!(
                "editor-breadcrumb-{}-{}",
                symbol.start_line, symbol.start_column
            );
            breadcrumb_items = breadcrumb_items.child("›").child(
                div()
                    .id(symbol_id)
                    .flex_none()
                    .cursor_pointer()
                    .hover(move |style| style.text_color(breadcrumb_hover))
                    .child(symbol.name.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_symbol(symbol.clone(), window, cx);
                    })),
            );
        }

        let header_action: AnyElement = match &self.surface {
            ProjectEditorSurface::Code { input, .. } => {
                let input = input.clone();
                let search_hover = cx.theme().accent;
                div()
                    .id("editor-search")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size_7()
                    .rounded(px(4.0))
                    .text_color(cx.theme().muted_foreground)
                    .cursor_pointer()
                    .hover(move |style| style.bg(search_hover))
                    .child(Icon::new(IconName::Search).size_3())
                    .on_click(move |_, window, cx| {
                        input.update(cx, |input, input_cx| input.focus(window, input_cx));
                        window.dispatch_action(Box::new(Search), cx);
                    })
                    .into_any_element()
            }
            ProjectEditorSurface::Markdown { editor, .. } => {
                let editor = editor.clone();
                let mode = editor.read(cx).mode();
                let (icon, tooltip) = match mode {
                    MarkdownEditorMode::Rendered => (
                        IconName::EyeOff,
                        self.markdown_config
                            .ui_text
                            .get(UiTextKey::MarkdownShowSource),
                    ),
                    MarkdownEditorMode::Source => (
                        IconName::Eye,
                        self.markdown_config
                            .ui_text
                            .get(UiTextKey::MarkdownShowRendered),
                    ),
                };
                div()
                    .debug_selector(|| "markdown-mode-toggle".to_string())
                    .child(
                        Button::new("markdown-mode-toggle-button")
                            .ghost()
                            .xsmall()
                            .icon(icon)
                            .tooltip(tooltip)
                            .on_click(move |_, _, cx| {
                                editor.update(cx, |editor, editor_cx| {
                                    editor.toggle_mode(editor_cx);
                                });
                            }),
                    )
                    .into_any_element()
            }
        };
        let breadcrumbs = div()
            .id("editor-breadcrumbs")
            .debug_selector(|| "editor-breadcrumbs".to_string())
            .flex()
            .flex_none()
            .h_8()
            .items_center()
            .gap_2()
            .px_2()
            .overflow_hidden()
            .bg(cx.theme().tokens.popover)
            .text_sm()
            .child(breadcrumb_items)
            .child(header_action);

        let body: AnyElement = match &self.surface {
            ProjectEditorSurface::Code { input, .. } => {
                styled_code_editor_input(input, &self.appearance)
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flush_search_panel(true)
                    .into_any_element()
            }
            ProjectEditorSurface::Markdown { editor, .. } => div()
                .debug_selector(|| "markdown-editor".to_string())
                .flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .font_family(self.appearance.resolved_font_family())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_this, _, _window, cx| {
                        cx.emit(ProjectEditorDocumentEvent::Focused);
                    }),
                )
                .child(editor.clone())
                .into_any_element(),
        };

        div()
            .id("project-editor-document")
            .flex()
            .flex_col()
            .size_full()
            .text_color(cx.theme().foreground)
            .min_h_0()
            .child(breadcrumbs)
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui::{Modifiers, TestAppContext};

    use super::*;
    use crate::{model::ids::ProjectId, ui::theme::ThemeRuntime};

    #[gpui::test]
    fn markdown_document_renders_toggles_edits_and_serializes(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let path = PathBuf::from("/tmp/yttt/README.md");
        let document_id = DocumentId {
            project_id: ProjectId::new("project"),
            canonical_path: path.clone(),
        };
        let config = super::super::CodeEditorConfig::new(
            "README.md",
            super::super::CodeEditorLanguageMode::Auto,
        );
        let model = ProjectEditorModel::new(
            document_id,
            CodeEditorState::new(&path, config, "# Heading\n\nBody"),
            DiskFingerprint {
                exists: true,
                byte_len: 15,
                modified: None,
                content_hash: 1,
            },
        );
        let appearance = EditorAppearance::default();
        let markdown_config = MarkdownDocumentConfig::new(
            Arc::new(
                ThemeRuntime::default()
                    .to_markdown_editor_theme(appearance.font_size, appearance.line_height),
            ),
            Arc::new(MarkdownEditorStrings::en_us()),
            UiText::english(),
        );
        let (document, mut cx) = cx.add_window_view(move |window, cx| {
            ProjectEditorDocument::new_with_markdown_config(
                model,
                appearance,
                markdown_config,
                window,
                cx,
            )
        });

        cx.refresh().unwrap();
        assert!(cx.debug_bounds("markdown-editor").is_some());
        let toggle = cx.debug_bounds("markdown-mode-toggle").unwrap();
        cx.simulate_click(toggle.center(), Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, app| {
            let editor = document
                .read(app)
                .markdown_editor()
                .expect("Markdown document must use the dedicated editor")
                .clone();
            assert_eq!(editor.read(app).mode(), MarkdownEditorMode::Source);
            document.update(app, |document, cx| document.focus(window, cx));
        });
        cx.run_until_parked();

        cx.simulate_keystrokes("x");
        cx.run_until_parked();
        let request = cx.update(|_window, app| {
            document.update(app, |document, cx| {
                assert!(document.model().is_dirty());
                document.begin_save(cx)
            })
        });
        assert_ne!(request.text, "# Heading\n\nBody");
        assert!(request.text.contains('x'));
    }
}
