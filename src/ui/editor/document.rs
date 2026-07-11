use gpui::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    div, px, relative,
};
use gpui_component::ActiveTheme as _;
use gpui_component::input::{Input, InputEvent, InputState, Position, Search};

use crate::config::settings::EditorSettings;

use super::{
    CodeEditorState, DiskFingerprint, DocumentId, EditorSymbol, breadcrumbs_at,
    code_editor_input_state, document_symbols,
};

#[derive(Clone, Debug, PartialEq)]
pub struct EditorAppearance {
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub soft_wrap: bool,
    pub line_numbers: bool,
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
        }
    }

    pub fn document_id(&self) -> &DocumentId {
        &self.document_id
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
        self.editor.is_dirty()
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

    pub fn finish_save(
        &mut self,
        request: &SaveRequest,
        disk_fingerprint: DiskFingerprint,
    ) -> bool {
        if request.document_id != self.document_id {
            return false;
        }
        self.editor.mark_value_saved(request.text.clone());
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
        self.disk_fingerprint = disk_fingerprint;
        self.generation = self.generation.wrapping_add(1);
        self.save_state = ProjectEditorSaveState::Idle;
    }
}

pub struct ProjectEditorDocument {
    model: ProjectEditorModel,
    input: Entity<InputState>,
    appearance: EditorAppearance,
    symbols: Vec<EditorSymbol>,
    breadcrumbs: Vec<EditorSymbol>,
    breadcrumb_cursor_line: usize,
    _input_subscription: Subscription,
    _input_observer: Subscription,
}

impl ProjectEditorDocument {
    pub fn new(
        model: ProjectEditorModel,
        appearance: EditorAppearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| code_editor_input_state(window, cx, model.editor()));
        let symbols = document_symbols(model.editor().language_id(), model.value());
        let breadcrumbs = breadcrumbs_at(&symbols, 0);
        input.update(cx, |input, input_cx| {
            input.set_soft_wrap(appearance.soft_wrap, window, input_cx);
            input.set_line_number(appearance.line_numbers, window, input_cx);
        });
        let input_subscription = cx.subscribe_in(&input, window, Self::on_input_event);
        let input_observer = cx.observe_in(&input, window, Self::on_input_notify);
        Self {
            model,
            input,
            appearance,
            symbols,
            breadcrumbs,
            breadcrumb_cursor_line: 0,
            _input_subscription: input_subscription,
            _input_observer: input_observer,
        }
    }

    pub fn model(&self) -> &ProjectEditorModel {
        &self.model
    }

    pub fn model_mut(&mut self) -> &mut ProjectEditorModel {
        &mut self.model
    }

    pub fn input(&self) -> &Entity<InputState> {
        &self.input
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
        self.input.update(cx, |input, input_cx| {
            input.set_soft_wrap(appearance.soft_wrap, window, input_cx);
            input.set_line_number(appearance.line_numbers, window, input_cx);
        });
        self.appearance = appearance;
        cx.notify();
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
        self.input.update(cx, |input, input_cx| {
            input.set_value(value, window, input_cx)
        });
        self.refresh_breadcrumbs(0);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, input_cx| input.focus(window, input_cx));
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
        self.symbols = document_symbols(self.model.editor().language_id(), self.model.value());
        self.breadcrumbs = breadcrumbs_at(&self.symbols, cursor_line);
    }

    fn focus_symbol(&mut self, symbol: EditorSymbol, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, input_cx| {
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
        let mut breadcrumbs = div()
            .id("editor-breadcrumbs")
            .flex()
            .flex_none()
            .h(px(28.0))
            .items_center()
            .gap_1()
            .px_2()
            .overflow_hidden()
            .text_sm()
            .child(self.model.editor().config().title().to_string());
        let input = self.input.clone();
        breadcrumbs = breadcrumbs.child(
            div()
                .id("editor-search")
                .ml_auto()
                .cursor_pointer()
                .hover(|style| style.opacity(0.7))
                .child("Find")
                .on_click(move |_, window, cx| {
                    input.update(cx, |input, input_cx| input.focus(window, input_cx));
                    window.dispatch_action(Box::new(Search), cx);
                }),
        );
        for symbol in self.breadcrumbs.clone() {
            let symbol_id = format!(
                "editor-breadcrumb-{}-{}",
                symbol.start_line, symbol.start_column
            );
            breadcrumbs = breadcrumbs.child("›").child(
                div()
                    .id(symbol_id)
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.7))
                    .child(symbol.name.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_symbol(symbol.clone(), window, cx);
                    })),
            );
        }

        let input = Input::new(&self.input)
            .flex_1()
            .min_h_0()
            .w_full()
            .appearance(false)
            .text_size(px(self.appearance.font_size))
            .line_height(relative(self.appearance.line_height));
        let input = if self.appearance.font_family.is_empty() {
            input
        } else {
            input.font_family(self.appearance.font_family.clone())
        };

        div()
            .id("project-editor-document")
            .flex()
            .flex_col()
            .size_full()
            .text_color(cx.theme().foreground)
            .min_h_0()
            .child(breadcrumbs)
            .child(input)
    }
}
