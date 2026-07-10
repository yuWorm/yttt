use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, Render, Styled as _, Subscription,
    Window, px, relative,
};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::config::settings::EditorSettings;

use super::{CodeEditorState, DiskFingerprint, DocumentId, code_editor_input_state};

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
    _input_subscription: Subscription,
}

impl ProjectEditorDocument {
    pub fn new(
        model: ProjectEditorModel,
        appearance: EditorAppearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| code_editor_input_state(window, cx, model.editor()));
        input.update(cx, |input, input_cx| {
            input.set_soft_wrap(appearance.soft_wrap, window, input_cx);
            input.set_line_number(appearance.line_numbers, window, input_cx);
        });
        let input_subscription = cx.subscribe_in(&input, window, Self::on_input_event);
        Self {
            model,
            input,
            appearance,
            _input_subscription: input_subscription,
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
                let previous_generation = self.model.generation();
                let generation = self
                    .model
                    .on_input_changed(input.read(cx).value().to_string());
                if generation != previous_generation {
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
}

impl EventEmitter<ProjectEditorDocumentEvent> for ProjectEditorDocument {}

impl Render for ProjectEditorDocument {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let input = Input::new(&self.input)
            .w_full()
            .h_full()
            .appearance(true)
            .text_size(px(self.appearance.font_size))
            .line_height(relative(self.appearance.line_height));
        if self.appearance.font_family.is_empty() {
            input
        } else {
            input.font_family(self.appearance.font_family.clone())
        }
    }
}
