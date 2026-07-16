use std::{cmp, ops::Range};

use gpui::{
    Action, App, ClipboardItem, Context, Global, KeyBinding, KeyContext, KeystrokeEvent, Window,
    actions,
};
use gpui_component::input::{InputCursorShape, InputState, Rope, RopeExt as _, Search};

const VIM_EDITOR_CONTEXT: &str = "VimEditor";
const VIM_CONTROL_CONTEXT: &str = "VimControl";
const VIM_CONTROL_BINDING_CONTEXT: &str = "VimControl && !SearchPanel";
const VIM_EDITOR_BINDING_CONTEXT: &str = "VimEditor && !SearchPanel";
const NORMAL_CONTEXT: &str = "VimEditor && vim_mode == normal && !SearchPanel";
const INSERT_CONTEXT: &str = "VimEditor && vim_mode == insert && !SearchPanel";
const MAX_VIM_COUNT: usize = 999_999;
const MAX_REPEAT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VimMode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl VimMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual => "VISUAL",
            Self::VisualLine => "V-LINE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Motion {
    Left,
    Right,
    Up { display_lines: bool },
    Down { display_lines: bool },
    NextWordStart,
    NextWordEnd,
    PreviousWordStart,
    StartOfLine,
    FirstNonWhitespace,
    EndOfLine,
    StartOfDocument,
    EndOfDocument,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MotionKind {
    Exclusive,
    Inclusive,
    Linewise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Operator {
    Delete,
    Change,
    Yank,
}

impl Operator {
    fn label(self) -> &'static str {
        match self {
            Self::Delete => "d",
            Self::Change => "c",
            Self::Yank => "y",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InsertPlacement {
    Current,
    After,
    FirstNonWhitespace,
    EndOfLine,
    NewLineBelow,
    NewLineAbove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingCharacter {
    Replace,
}

#[derive(Action, Clone, Copy, Debug, PartialEq, Eq)]
#[action(namespace = vim, no_json)]
pub(super) struct MoveAction {
    pub motion: Motion,
}

#[derive(Action, Clone, Copy, Debug, PartialEq, Eq)]
#[action(namespace = vim, no_json)]
pub(super) struct Number(pub u8);

#[derive(Action, Clone, Copy, Debug, PartialEq, Eq)]
#[action(namespace = vim, no_json)]
pub(super) struct PushOperator {
    pub operator: Operator,
}

#[derive(Action, Clone, Copy, Debug, PartialEq, Eq)]
#[action(namespace = vim, no_json)]
pub(super) struct EnterInsert {
    pub placement: InsertPlacement,
}

#[derive(Action, Clone, Copy, Debug, PartialEq, Eq)]
#[action(namespace = vim, no_json)]
pub(super) struct Paste {
    pub before: bool,
}

actions!(
    vim,
    [
        Escape,
        Zero,
        ToggleVisual,
        ToggleVisualLine,
        DeleteCharacters,
        SubstituteCharacters,
        ReplaceCharacters,
        Undo,
        Redo,
        SearchForward,
        SearchNext,
        SearchPrevious,
    ]
);

#[derive(Clone, Debug, Default)]
struct Register {
    text: String,
    linewise: bool,
}

#[derive(Default)]
struct VimRegisters {
    unnamed: Option<Register>,
}

impl Global for VimRegisters {}

#[derive(Clone, Copy, Debug)]
struct MotionResult {
    target: usize,
    kind: MotionKind,
}

#[derive(Debug)]
pub struct VimState {
    mode: VimMode,
    operator: Option<Operator>,
    pending_character: Option<PendingCharacter>,
    pre_count: Option<usize>,
    post_count: Option<usize>,
    visual_anchor: Option<usize>,
    visual_head: Option<usize>,
    preferred_column: Option<usize>,
    history_group_open: bool,
}

impl Default for VimState {
    fn default() -> Self {
        Self {
            mode: VimMode::Normal,
            operator: None,
            pending_character: None,
            pre_count: None,
            post_count: None,
            visual_anchor: None,
            visual_head: None,
            preferred_column: None,
            history_group_open: false,
        }
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(VimRegisters::default());
    cx.bind_keys(default_keybindings());
}

fn default_keybindings() -> Vec<KeyBinding> {
    let control = Some(VIM_CONTROL_BINDING_CONTEXT);
    let normal = Some(NORMAL_CONTEXT);
    let insert = Some(INSERT_CONTEXT);
    let editor = Some(VIM_EDITOR_BINDING_CONTEXT);

    vec![
        KeyBinding::new(
            "h",
            MoveAction {
                motion: Motion::Left,
            },
            control,
        ),
        KeyBinding::new(
            "left",
            MoveAction {
                motion: Motion::Left,
            },
            control,
        ),
        KeyBinding::new(
            "l",
            MoveAction {
                motion: Motion::Right,
            },
            control,
        ),
        KeyBinding::new(
            "right",
            MoveAction {
                motion: Motion::Right,
            },
            control,
        ),
        KeyBinding::new(
            "j",
            MoveAction {
                motion: Motion::Down {
                    display_lines: false,
                },
            },
            control,
        ),
        KeyBinding::new(
            "down",
            MoveAction {
                motion: Motion::Down {
                    display_lines: false,
                },
            },
            control,
        ),
        KeyBinding::new(
            "k",
            MoveAction {
                motion: Motion::Up {
                    display_lines: false,
                },
            },
            control,
        ),
        KeyBinding::new(
            "up",
            MoveAction {
                motion: Motion::Up {
                    display_lines: false,
                },
            },
            control,
        ),
        KeyBinding::new(
            "g j",
            MoveAction {
                motion: Motion::Down {
                    display_lines: true,
                },
            },
            control,
        ),
        KeyBinding::new(
            "g k",
            MoveAction {
                motion: Motion::Up {
                    display_lines: true,
                },
            },
            control,
        ),
        KeyBinding::new(
            "w",
            MoveAction {
                motion: Motion::NextWordStart,
            },
            control,
        ),
        KeyBinding::new(
            "e",
            MoveAction {
                motion: Motion::NextWordEnd,
            },
            control,
        ),
        KeyBinding::new(
            "b",
            MoveAction {
                motion: Motion::PreviousWordStart,
            },
            control,
        ),
        KeyBinding::new("0", Zero, control),
        KeyBinding::new(
            "home",
            MoveAction {
                motion: Motion::StartOfLine,
            },
            control,
        ),
        KeyBinding::new(
            "^",
            MoveAction {
                motion: Motion::FirstNonWhitespace,
            },
            control,
        ),
        KeyBinding::new(
            "$",
            MoveAction {
                motion: Motion::EndOfLine,
            },
            control,
        ),
        KeyBinding::new(
            "end",
            MoveAction {
                motion: Motion::EndOfLine,
            },
            control,
        ),
        KeyBinding::new(
            "g g",
            MoveAction {
                motion: Motion::StartOfDocument,
            },
            control,
        ),
        KeyBinding::new(
            "shift-g",
            MoveAction {
                motion: Motion::EndOfDocument,
            },
            control,
        ),
        KeyBinding::new("1", Number(1), control),
        KeyBinding::new("2", Number(2), control),
        KeyBinding::new("3", Number(3), control),
        KeyBinding::new("4", Number(4), control),
        KeyBinding::new("5", Number(5), control),
        KeyBinding::new("6", Number(6), control),
        KeyBinding::new("7", Number(7), control),
        KeyBinding::new("8", Number(8), control),
        KeyBinding::new("9", Number(9), control),
        KeyBinding::new(
            "d",
            PushOperator {
                operator: Operator::Delete,
            },
            control,
        ),
        KeyBinding::new(
            "c",
            PushOperator {
                operator: Operator::Change,
            },
            control,
        ),
        KeyBinding::new(
            "y",
            PushOperator {
                operator: Operator::Yank,
            },
            control,
        ),
        KeyBinding::new(
            "i",
            EnterInsert {
                placement: InsertPlacement::Current,
            },
            normal,
        ),
        KeyBinding::new(
            "a",
            EnterInsert {
                placement: InsertPlacement::After,
            },
            normal,
        ),
        KeyBinding::new(
            "shift-i",
            EnterInsert {
                placement: InsertPlacement::FirstNonWhitespace,
            },
            normal,
        ),
        KeyBinding::new(
            "shift-a",
            EnterInsert {
                placement: InsertPlacement::EndOfLine,
            },
            normal,
        ),
        KeyBinding::new(
            "o",
            EnterInsert {
                placement: InsertPlacement::NewLineBelow,
            },
            normal,
        ),
        KeyBinding::new(
            "shift-o",
            EnterInsert {
                placement: InsertPlacement::NewLineAbove,
            },
            normal,
        ),
        KeyBinding::new("v", ToggleVisual, control),
        KeyBinding::new("shift-v", ToggleVisualLine, control),
        KeyBinding::new("x", DeleteCharacters, normal),
        KeyBinding::new("s", SubstituteCharacters, normal),
        KeyBinding::new("r", ReplaceCharacters, normal),
        KeyBinding::new("p", Paste { before: false }, normal),
        KeyBinding::new("shift-p", Paste { before: true }, normal),
        KeyBinding::new("u", Undo, normal),
        KeyBinding::new("ctrl-r", Redo, normal),
        KeyBinding::new("/", SearchForward, normal),
        KeyBinding::new("n", SearchNext, normal),
        KeyBinding::new("shift-n", SearchPrevious, normal),
        KeyBinding::new("escape", Escape, editor),
        KeyBinding::new("ctrl-[", Escape, editor),
        KeyBinding::new("escape", Escape, insert),
    ]
}

impl VimState {
    pub fn new(input: &mut InputState, cx: &mut Context<InputState>) -> Self {
        let state = Self::default();
        state.sync_input_mode(input, cx);
        state
    }

    pub fn mode(&self) -> VimMode {
        self.mode
    }

    pub fn status(&self) -> String {
        let mut status = self.mode.label().to_string();
        if let Some(operator) = self.operator {
            status.push(' ');
            status.push_str(operator.label());
        }
        if let Some(count) = self.active_count() {
            status.push_str(&count.to_string());
        }
        status
    }

    pub fn key_context(&self) -> KeyContext {
        let mut context = KeyContext::new_with_defaults();
        context.add(VIM_EDITOR_CONTEXT);
        let mode = if self.pending_character.is_some() {
            "waiting"
        } else if self.operator.is_some() {
            "operator"
        } else {
            match self.mode {
                VimMode::Normal => "normal",
                VimMode::Insert => "insert",
                VimMode::Visual | VimMode::VisualLine => "visual",
            }
        };
        context.set("vim_mode", mode);
        if matches!(
            self.mode,
            VimMode::Normal | VimMode::Visual | VimMode::VisualLine
        ) && self.pending_character.is_none()
        {
            context.add(VIM_CONTROL_CONTEXT);
        }
        context
    }

    pub fn disable(&mut self, input: &mut InputState, cx: &mut Context<InputState>) {
        self.finish_history_group(input);
        self.clear_pending();
        self.visual_anchor = None;
        self.visual_head = None;
        self.mode = VimMode::Insert;
        input.set_text_input_enabled(true, cx);
        input.set_cursor_shape(InputCursorShape::Bar, cx);
    }

    pub fn escape(
        &mut self,
        input: &mut InputState,
        _window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        let prior_mode = self.mode;
        let head = self.visual_head.unwrap_or_else(|| input.selection_head());
        self.clear_pending();
        self.visual_anchor = None;
        self.visual_head = None;
        self.preferred_column = None;
        if prior_mode == VimMode::Insert {
            self.finish_history_group(input);
            let point = input.text().offset_to_point(input.cursor());
            let line_start = input.text().line_start_offset(point.row);
            if input.cursor() > line_start {
                input.set_cursor_offset(input.previous_grapheme_boundary(input.cursor()), cx);
            }
        } else if matches!(prior_mode, VimMode::Visual | VimMode::VisualLine) {
            input.set_cursor_offset(head, cx);
        }
        self.mode = VimMode::Normal;
        self.sync_input_mode(input, cx);
    }

    fn sync_input_mode(&self, input: &mut InputState, cx: &mut Context<InputState>) {
        let insert = self.mode == VimMode::Insert;
        input.set_text_input_enabled(insert, cx);
        input.set_cursor_shape(
            if insert {
                InputCursorShape::Bar
            } else {
                InputCursorShape::Block
            },
            cx,
        );
    }

    fn clear_pending(&mut self) {
        self.operator = None;
        self.pending_character = None;
        self.pre_count = None;
        self.post_count = None;
    }

    fn active_count(&self) -> Option<usize> {
        if self.operator.is_some() {
            self.post_count.or(self.pre_count)
        } else {
            self.pre_count
        }
    }

    pub fn push_digit(&mut self, digit: u8) {
        let target = if self.operator.is_some() {
            &mut self.post_count
        } else {
            &mut self.pre_count
        };
        let count = target
            .unwrap_or(0)
            .saturating_mul(10)
            .saturating_add(digit as usize)
            .min(MAX_VIM_COUNT);
        *target = Some(count);
    }

    pub fn zero(
        &mut self,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        let is_count_digit = if self.operator.is_some() {
            self.post_count.is_some()
        } else {
            self.pre_count.is_some()
        };
        if is_count_digit {
            self.push_digit(0);
        } else {
            self.motion(Motion::StartOfLine, input, window, cx);
        }
    }

    fn take_count(&mut self) -> usize {
        let before = self.pre_count.take().unwrap_or(1);
        let after = self.post_count.take().unwrap_or(1);
        before.saturating_mul(after).clamp(1, MAX_VIM_COUNT)
    }

    pub fn motion(
        &mut self,
        motion: Motion,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        let explicit_count = self.active_count().is_some();
        let count = self.take_count();
        if matches!(
            motion,
            Motion::Up {
                display_lines: true
            } | Motion::Down {
                display_lines: true
            }
        ) && self.operator.is_none()
            && self.mode == VimMode::Normal
        {
            let lines = match motion {
                Motion::Up { .. } => -(count as isize),
                Motion::Down { .. } => count as isize,
                _ => unreachable!(),
            };
            input.move_display_lines(lines, window, cx);
            self.preferred_column = None;
            return;
        }

        let operator = self.operator.take();
        let effective_motion =
            if operator == Some(Operator::Change) && motion == Motion::NextWordStart {
                Motion::NextWordEnd
            } else {
                motion
            };
        let start = input.cursor();
        let result = self.motion_result(input, effective_motion, count, explicit_count);

        if matches!(self.mode, VimMode::Visual | VimMode::VisualLine) {
            self.update_visual_selection(result.target, input, cx);
            self.operator = operator;
            return;
        }

        if let Some(operator) = operator {
            let range = self.range_for_motion(input, start, result);
            self.apply_operator(
                operator,
                range,
                result.kind == MotionKind::Linewise,
                input,
                window,
                cx,
            );
        } else {
            input.set_cursor_offset(result.target, cx);
        }
    }

    fn motion_result(
        &mut self,
        input: &InputState,
        motion: Motion,
        count: usize,
        explicit_count: bool,
    ) -> MotionResult {
        let text = input.text();
        let mut target = if matches!(self.mode, VimMode::Visual | VimMode::VisualLine) {
            self.visual_head.unwrap_or_else(|| input.selection_head())
        } else {
            input.cursor()
        };
        let kind = match motion {
            Motion::Up { .. }
            | Motion::Down { .. }
            | Motion::StartOfDocument
            | Motion::EndOfDocument => MotionKind::Linewise,
            Motion::NextWordEnd | Motion::EndOfLine => MotionKind::Inclusive,
            _ => MotionKind::Exclusive,
        };

        if motion == Motion::StartOfDocument {
            target = text.line_start_offset(
                count
                    .saturating_sub(1)
                    .min(text.lines_len().saturating_sub(1)),
            );
        } else if motion == Motion::EndOfDocument {
            target = if explicit_count {
                text.line_start_offset(
                    count
                        .saturating_sub(1)
                        .min(text.lines_len().saturating_sub(1)),
                )
            } else {
                document_last_character(input)
            };
        } else if motion == Motion::EndOfLine {
            let row = text
                .offset_to_point(target)
                .row
                .saturating_add(count.saturating_sub(1))
                .min(text.lines_len().saturating_sub(1));
            target = line_last_character(input, text.line_start_offset(row));
        } else {
            for _ in 0..count {
                let previous = target;
                target = match motion {
                    Motion::Left => left_target(input, target),
                    Motion::Right => right_target(input, target),
                    Motion::Up { .. } => {
                        logical_vertical_target(input, target, -1, &mut self.preferred_column)
                    }
                    Motion::Down { .. } => {
                        logical_vertical_target(input, target, 1, &mut self.preferred_column)
                    }
                    Motion::NextWordStart => next_word_start(input, target),
                    Motion::NextWordEnd => next_word_end(input, target),
                    Motion::PreviousWordStart => previous_word_start(input, target),
                    Motion::StartOfLine => line_start(text, target),
                    Motion::FirstNonWhitespace => first_non_whitespace(input, target),
                    Motion::EndOfLine | Motion::StartOfDocument | Motion::EndOfDocument => {
                        unreachable!()
                    }
                };
                if target == previous && motion == Motion::NextWordEnd {
                    let next = input.next_grapheme_boundary(target);
                    if next != target {
                        target = next_word_end(input, next);
                    }
                }
                if target == previous {
                    break;
                }
            }
        }

        if !matches!(motion, Motion::Up { .. } | Motion::Down { .. }) {
            self.preferred_column = None;
        }
        MotionResult { target, kind }
    }

    fn range_for_motion(
        &self,
        input: &InputState,
        start: usize,
        result: MotionResult,
    ) -> Range<usize> {
        if result.kind == MotionKind::Linewise {
            let start_row = input.text().offset_to_point(start).row;
            let target_row = input.text().offset_to_point(result.target).row;
            return linewise_range(
                input.text(),
                cmp::min(start_row, target_row),
                cmp::max(start_row, target_row),
            );
        }

        let low = cmp::min(start, result.target);
        let high = cmp::max(start, result.target);
        if result.kind == MotionKind::Inclusive {
            low..input.next_grapheme_boundary(high)
        } else {
            low..high
        }
    }

    pub fn push_operator(
        &mut self,
        operator: Operator,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        if matches!(self.mode, VimMode::Visual | VimMode::VisualLine) {
            let range = if self.mode == VimMode::VisualLine {
                let selected = input.selected_range();
                let first = input.text().offset_to_point(selected.start).row;
                let last_offset = selected.end.saturating_sub(1);
                let last = input.text().offset_to_point(last_offset).row;
                linewise_range(input.text(), first, last)
            } else {
                input.selected_range()
            };
            self.apply_operator(
                operator,
                range,
                self.mode == VimMode::VisualLine,
                input,
                window,
                cx,
            );
            return;
        }

        if self.operator == Some(operator) {
            self.operator = None;
            let count = self.take_count();
            let row = input.text().offset_to_point(input.cursor()).row;
            let end_row = row.saturating_add(count.saturating_sub(1));
            let range = linewise_range(input.text(), row, end_row);
            self.apply_operator(operator, range, true, input, window, cx);
        } else {
            self.operator = Some(operator);
            self.post_count = None;
        }
    }

    fn apply_operator(
        &mut self,
        operator: Operator,
        range: Range<usize>,
        linewise: bool,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        if range.start >= range.end || range.end > input.text().len() {
            self.clear_pending();
            return;
        }
        let text = input.text().slice(range.clone()).to_string();
        let ends_with_newline = text.ends_with('\n');
        self.write_register(Register { text, linewise }, cx);
        let cursor = range.start;

        match operator {
            Operator::Yank => {
                input.set_cursor_offset(cursor, cx);
                self.mode = VimMode::Normal;
                self.clear_pending();
                self.visual_anchor = None;
                self.visual_head = None;
                self.sync_input_mode(input, cx);
            }
            Operator::Delete => {
                self.begin_history_group(input);
                input.replace_range(range, "", window, cx);
                self.finish_history_group(input);
                input.set_cursor_offset(normal_cursor_offset(input, cursor), cx);
                self.mode = VimMode::Normal;
                self.clear_pending();
                self.visual_anchor = None;
                self.visual_head = None;
                self.sync_input_mode(input, cx);
            }
            Operator::Change => {
                self.begin_history_group(input);
                let replacement = if linewise && ends_with_newline {
                    "\n"
                } else {
                    ""
                };
                input.replace_range(range, replacement, window, cx);
                input.set_cursor_offset(cursor.min(input.text().len()), cx);
                self.mode = VimMode::Insert;
                self.clear_pending();
                self.visual_anchor = None;
                self.visual_head = None;
                self.sync_input_mode(input, cx);
            }
        }
    }

    fn write_register(&self, register: Register, cx: &mut Context<InputState>) {
        cx.write_to_clipboard(ClipboardItem::new_string(register.text.clone()));
        cx.global_mut::<VimRegisters>().unnamed = Some(register);
    }

    pub fn toggle_visual(
        &mut self,
        linewise: bool,
        input: &mut InputState,
        cx: &mut Context<InputState>,
    ) {
        let requested = if linewise {
            VimMode::VisualLine
        } else {
            VimMode::Visual
        };
        if self.mode == requested {
            let head = input.selection_head();
            input.set_cursor_offset(head, cx);
            self.mode = VimMode::Normal;
            self.visual_anchor = None;
            self.visual_head = None;
            self.clear_pending();
            self.sync_input_mode(input, cx);
            return;
        }

        let cursor = input.cursor();
        self.mode = requested;
        self.visual_anchor = Some(cursor);
        self.visual_head = Some(cursor);
        self.clear_pending();
        self.update_visual_selection(cursor, input, cx);
        self.sync_input_mode(input, cx);
    }

    fn update_visual_selection(
        &mut self,
        target: usize,
        input: &mut InputState,
        cx: &mut Context<InputState>,
    ) {
        self.visual_head = Some(target);
        let anchor = self.visual_anchor.unwrap_or(input.cursor());
        if self.mode == VimMode::VisualLine {
            let anchor_row = input.text().offset_to_point(anchor).row;
            let target_row = input.text().offset_to_point(target).row;
            let range = linewise_range(
                input.text(),
                cmp::min(anchor_row, target_row),
                cmp::max(anchor_row, target_row),
            );
            if target_row < anchor_row {
                input.set_selection(range.end, range.start, cx);
            } else {
                input.set_selection(range.start, range.end, cx);
            }
        } else if target < anchor {
            input.set_selection(input.next_grapheme_boundary(anchor), target, cx);
        } else {
            input.set_selection(anchor, input.next_grapheme_boundary(target), cx);
        }
    }

    pub fn enter_insert(
        &mut self,
        placement: InsertPlacement,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        self.begin_history_group(input);
        let cursor = input.cursor();
        let point = input.text().offset_to_point(cursor);
        let row = point.row;
        let line_start = input.text().line_start_offset(row);
        let line_end = input.text().line_end_offset(row);
        let insertion_offset = match placement {
            InsertPlacement::Current => cursor,
            InsertPlacement::After => {
                if cursor < line_end {
                    input.next_grapheme_boundary(cursor)
                } else {
                    line_end
                }
            }
            InsertPlacement::FirstNonWhitespace => first_non_whitespace(input, cursor),
            InsertPlacement::EndOfLine => line_end,
            InsertPlacement::NewLineBelow => {
                let indent = leading_indent(input.text(), row);
                let inserted = format!("\n{indent}");
                input.replace_range(line_end..line_end, &inserted, window, cx);
                line_end + inserted.len()
            }
            InsertPlacement::NewLineAbove => {
                let indent = leading_indent(input.text(), row);
                let inserted = format!("{indent}\n");
                input.replace_range(line_start..line_start, &inserted, window, cx);
                line_start + indent.len()
            }
        };
        input.set_cursor_offset(insertion_offset, cx);
        self.mode = VimMode::Insert;
        self.visual_anchor = None;
        self.visual_head = None;
        self.clear_pending();
        self.sync_input_mode(input, cx);
    }

    pub fn delete_characters(
        &mut self,
        substitute: bool,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        let count = self.take_count();
        let start = input.cursor();
        let row = input.text().offset_to_point(start).row;
        let line_end = input.text().line_end_offset(row);
        let mut end = start;
        for _ in 0..count {
            if end >= line_end {
                break;
            }
            let next = input.next_grapheme_boundary(end).min(line_end);
            if next == end {
                break;
            }
            end = next;
        }
        if end == start {
            self.clear_pending();
            return;
        }
        let range = start..end;
        if substitute {
            self.apply_operator(Operator::Change, range, false, input, window, cx);
        } else {
            self.apply_operator(Operator::Delete, range, false, input, window, cx);
        }
    }

    pub fn begin_replace_character(&mut self) {
        self.pending_character = Some(PendingCharacter::Replace);
    }

    pub fn handle_pending_character(
        &mut self,
        event: &KeystrokeEvent,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> bool {
        let Some(PendingCharacter::Replace) = self.pending_character else {
            return false;
        };
        if event.action.is_some()
            || event.keystroke.modifiers.control
            || event.keystroke.modifiers.platform
            || event.keystroke.is_ime_in_progress()
        {
            return false;
        }
        let Some(text) = event
            .keystroke
            .key_char
            .as_deref()
            .filter(|text| !text.is_empty())
        else {
            return false;
        };
        let count = self.take_count();
        let start = input.cursor();
        let row = input.text().offset_to_point(start).row;
        let line_end = input.text().line_end_offset(row);
        let mut end = start;
        let mut replaced = 0;
        while replaced < count && end < line_end {
            let next = input.next_grapheme_boundary(end).min(line_end);
            if next == end {
                break;
            }
            end = next;
            replaced += 1;
        }
        if replaced == 0 {
            self.clear_pending();
            return true;
        }
        self.begin_history_group(input);
        let replacement = text.repeat(bounded_repeat_count(text.len(), replaced));
        input.replace_range(start..end, &replacement, window, cx);
        self.finish_history_group(input);
        let cursor = start + replacement.len().saturating_sub(text.len());
        input.set_cursor_offset(cursor, cx);
        self.clear_pending();
        self.mode = VimMode::Normal;
        self.sync_input_mode(input, cx);
        true
    }

    pub fn paste(
        &mut self,
        before: bool,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        let register = cx.global::<VimRegisters>().unnamed.clone().or_else(|| {
            cx.read_from_clipboard().and_then(|item| {
                item.text().map(|text| Register {
                    text,
                    linewise: false,
                })
            })
        });
        let Some(mut register) = register else {
            return;
        };
        let count = self.take_count();
        if register.linewise && !register.text.ends_with('\n') {
            register.text.push('\n');
        }
        let count = bounded_repeat_count(register.text.len(), count);
        if count > 1 {
            register.text = register.text.repeat(count);
        }

        self.begin_history_group(input);
        let cursor = input.cursor();
        let insertion_offset = if register.linewise {
            let row = input.text().offset_to_point(cursor).row;
            if before {
                input.text().line_start_offset(row)
            } else {
                let line_end = input.text().line_end_offset(row);
                if line_end < input.text().len() {
                    input.next_grapheme_boundary(line_end)
                } else {
                    line_end
                }
            }
        } else if before {
            cursor
        } else {
            input.next_grapheme_boundary(cursor)
        };

        let ends_in_newline = input.text().len() > 0
            && input
                .text()
                .char_at(input.previous_grapheme_boundary(input.text().len()))
                == Some('\n');
        let inserted = if register.linewise
            && insertion_offset == input.text().len()
            && input.text().len() > 0
            && !ends_in_newline
        {
            format!("\n{}", register.text)
        } else {
            register.text
        };
        input.replace_range(insertion_offset..insertion_offset, &inserted, window, cx);
        self.finish_history_group(input);
        let cursor = if register.linewise {
            insertion_offset + inserted.strip_prefix('\n').map_or(0, |_| 1)
        } else {
            insertion_offset + inserted.len().saturating_sub(1)
        };
        input.set_cursor_offset(normal_cursor_offset(input, cursor), cx);
        self.mode = VimMode::Normal;
        self.sync_input_mode(input, cx);
    }

    pub fn undo(
        &mut self,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        self.finish_history_group(input);
        if let Some(cursor) = input.undo_edit(window, cx) {
            input.set_cursor_offset(normal_cursor_offset(input, cursor), cx);
        }
        self.mode = VimMode::Normal;
        self.clear_pending();
        self.sync_input_mode(input, cx);
    }

    pub fn redo(
        &mut self,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        self.finish_history_group(input);
        if let Some(cursor) = input.redo_edit(window, cx) {
            input.set_cursor_offset(normal_cursor_offset(input, cursor), cx);
        }
        self.mode = VimMode::Normal;
        self.clear_pending();
        self.sync_input_mode(input, cx);
    }

    pub fn search(
        &mut self,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        self.clear_pending();
        input.focus(window, cx);
        window.dispatch_action(Box::new(Search), cx);
    }

    pub fn search_next(
        &mut self,
        backwards: bool,
        input: &mut InputState,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        let count = self.take_count();
        for _ in 0..count {
            input.move_to_search_match(backwards, window, cx);
        }
    }

    fn begin_history_group(&mut self, input: &mut InputState) {
        if !self.history_group_open {
            input.begin_history_group();
            self.history_group_open = true;
        }
    }

    fn finish_history_group(&mut self, input: &mut InputState) {
        if self.history_group_open {
            input.end_history_group();
            self.history_group_open = false;
        }
    }
}

fn line_start(text: &Rope, offset: usize) -> usize {
    text.line_start_offset(text.offset_to_point(offset).row)
}

fn line_last_character(input: &InputState, offset: usize) -> usize {
    let text = input.text();
    let row = text.offset_to_point(offset).row;
    let start = text.line_start_offset(row);
    let end = text.line_end_offset(row);
    if end > start {
        input.previous_grapheme_boundary(end)
    } else {
        start
    }
}

fn document_last_character(input: &InputState) -> usize {
    if input.text().len() == 0 {
        0
    } else {
        input.previous_grapheme_boundary(input.text().len())
    }
}

fn left_target(input: &InputState, offset: usize) -> usize {
    let start = line_start(input.text(), offset);
    if offset > start {
        input.previous_grapheme_boundary(offset)
    } else {
        offset
    }
}

fn right_target(input: &InputState, offset: usize) -> usize {
    let last = line_last_character(input, offset);
    if offset < last {
        input.next_grapheme_boundary(offset)
    } else {
        offset
    }
}

fn logical_vertical_target(
    input: &InputState,
    offset: usize,
    delta: isize,
    preferred_column: &mut Option<usize>,
) -> usize {
    let text = input.text();
    let point = text.offset_to_point(offset);
    let column = *preferred_column.get_or_insert(point.column);
    let max_row = text.lines_len().saturating_sub(1);
    let row = point.row.saturating_add_signed(delta).min(max_row);
    let start = text.line_start_offset(row);
    let end = text.line_end_offset(row);
    let last = if end > start {
        input.previous_grapheme_boundary(end)
    } else {
        start
    };
    start + column.min(last.saturating_sub(start))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharacterClass {
    Whitespace,
    Word,
    Punctuation,
}

fn character_class(ch: char) -> CharacterClass {
    if ch.is_whitespace() {
        CharacterClass::Whitespace
    } else if ch.is_alphanumeric() || ch == '_' {
        CharacterClass::Word
    } else {
        CharacterClass::Punctuation
    }
}

fn next_word_start(input: &InputState, offset: usize) -> usize {
    let len = input.text().len();
    if offset >= len {
        return document_last_character(input);
    }
    let mut cursor = offset;
    let class = input
        .text()
        .char_at(cursor)
        .map(character_class)
        .unwrap_or(CharacterClass::Whitespace);
    if class != CharacterClass::Whitespace {
        while cursor < len && input.text().char_at(cursor).map(character_class) == Some(class) {
            let next = input.next_grapheme_boundary(cursor);
            if next == cursor {
                break;
            }
            cursor = next;
        }
    }
    while cursor < len
        && input
            .text()
            .char_at(cursor)
            .is_some_and(|ch| ch.is_whitespace())
    {
        let next = input.next_grapheme_boundary(cursor);
        if next == cursor {
            break;
        }
        cursor = next;
    }
    normal_cursor_offset(input, cursor)
}

fn next_word_end(input: &InputState, offset: usize) -> usize {
    let len = input.text().len();
    if len == 0 {
        return 0;
    }
    let mut cursor = offset;
    while cursor < len
        && input
            .text()
            .char_at(cursor)
            .is_some_and(|ch| ch.is_whitespace())
    {
        cursor = input.next_grapheme_boundary(cursor);
    }
    let class = input
        .text()
        .char_at(cursor)
        .map(character_class)
        .unwrap_or(CharacterClass::Whitespace);
    loop {
        let next = input.next_grapheme_boundary(cursor);
        if next >= len || input.text().char_at(next).map(character_class) != Some(class) {
            break;
        }
        cursor = next;
    }
    normal_cursor_offset(input, cursor)
}

fn previous_word_start(input: &InputState, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let mut cursor = input.previous_grapheme_boundary(offset);
    while cursor > 0
        && input
            .text()
            .char_at(cursor)
            .is_some_and(|ch| ch.is_whitespace())
    {
        cursor = input.previous_grapheme_boundary(cursor);
    }
    let class = input
        .text()
        .char_at(cursor)
        .map(character_class)
        .unwrap_or(CharacterClass::Whitespace);
    while cursor > 0 {
        let previous = input.previous_grapheme_boundary(cursor);
        if input.text().char_at(previous).map(character_class) != Some(class) {
            break;
        }
        cursor = previous;
    }
    cursor
}

fn first_non_whitespace(input: &InputState, offset: usize) -> usize {
    let text = input.text();
    let row = text.offset_to_point(offset).row;
    let mut cursor = text.line_start_offset(row);
    let end = text.line_end_offset(row);
    while cursor < end
        && text
            .char_at(cursor)
            .is_some_and(|character| character.is_whitespace())
    {
        cursor = input.next_grapheme_boundary(cursor);
    }
    cursor.min(end)
}

fn linewise_range(text: &Rope, start_row: usize, end_row: usize) -> Range<usize> {
    let max_row = text.lines_len().saturating_sub(1);
    let start_row = start_row.min(max_row);
    let end_row = end_row.min(max_row);
    let start = text.line_start_offset(start_row);
    let next_row = end_row.saturating_add(1);
    let end = if next_row < text.lines_len() {
        text.line_start_offset(next_row)
    } else {
        text.len()
    };
    start..end
}

fn leading_indent(text: &Rope, row: usize) -> String {
    text.slice_line(row)
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .collect()
}

fn bounded_repeat_count(text_len: usize, requested: usize) -> usize {
    if text_len == 0 {
        return 1;
    }
    requested.min((MAX_REPEAT_BYTES / text_len).max(1))
}

fn normal_cursor_offset(input: &InputState, offset: usize) -> usize {
    let len = input.text().len();
    if len == 0 {
        return 0;
    }
    let offset = offset.min(len);
    let row = input.text().offset_to_point(offset).row;
    let start = input.text().line_start_offset(row);
    let end = input.text().line_end_offset(row);
    if end > start && offset >= end {
        input.previous_grapheme_boundary(end)
    } else {
        offset
    }
}
