use std::{cmp, ops::Range};

use gpui::{
    Action, App, ClipboardItem, Context, Global, KeyBinding, KeyContext, KeystrokeEvent, Window,
    actions,
};
use gpui_component::input::{InputCursorShape, InputState, Rope, RopeExt as _, Search};

pub const VIM_EDITOR_CONTEXT: &str = "VimEditor";
pub const VIM_CONTROL_CONTEXT: &str = "VimControl";
pub const VIM_CONTROL_BINDING_CONTEXT: &str = "VimControl && !SearchPanel";
pub const VIM_EDITOR_BINDING_CONTEXT: &str = "VimEditor && !SearchPanel";
pub const NORMAL_CONTEXT: &str = "VimEditor && vim_mode == normal && !SearchPanel";
pub const INSERT_CONTEXT: &str = "VimEditor && vim_mode == insert && !SearchPanel";
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

macro_rules! define_editor_vim_actions {
    (
        $(
            $variant:ident => {
                id: $id:literal,
                action: $action:expr,
                title: $title:literal,
                description: $description:literal,
            },
        )+
    ) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum EditorVimActionId {
            $( $variant, )+
        }

        impl EditorVimActionId {
            pub const ALL: &'static [Self] = &[$( Self::$variant, )+];

            pub fn from_str_id(id: &str) -> Option<Self> {
                match id {
                    $( $id => Some(Self::$variant), )+
                    _ => None,
                }
            }

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $id, )+
                }
            }

            pub const fn title(self) -> &'static str {
                match self {
                    $( Self::$variant => $title, )+
                }
            }

            pub const fn description(self) -> &'static str {
                match self {
                    $( Self::$variant => $description, )+
                }
            }

            pub fn build(self) -> Box<dyn Action> {
                match self {
                    $( Self::$variant => Box::new($action), )+
                }
            }

            pub fn key_binding(self, keys: &str, context: Option<&str>) -> KeyBinding {
                match self {
                    $( Self::$variant => KeyBinding::new(keys, $action, context), )+
                }
            }
        }
    };
}

define_editor_vim_actions! {
    MoveLeft => {
        id: "editor.vim.motion.left",
        action: MoveAction { motion: Motion::Left },
        title: "Editor Vim: Move Left",
        description: "Move the Vim cursor left.",
    },
    MoveRight => {
        id: "editor.vim.motion.right",
        action: MoveAction { motion: Motion::Right },
        title: "Editor Vim: Move Right",
        description: "Move the Vim cursor right.",
    },
    MoveDown => {
        id: "editor.vim.motion.down",
        action: MoveAction { motion: Motion::Down { display_lines: false } },
        title: "Editor Vim: Move Down",
        description: "Move the Vim cursor down by logical lines.",
    },
    MoveUp => {
        id: "editor.vim.motion.up",
        action: MoveAction { motion: Motion::Up { display_lines: false } },
        title: "Editor Vim: Move Up",
        description: "Move the Vim cursor up by logical lines.",
    },
    MoveDisplayDown => {
        id: "editor.vim.motion.display_down",
        action: MoveAction { motion: Motion::Down { display_lines: true } },
        title: "Editor Vim: Move Display Line Down",
        description: "Move the Vim cursor down by display lines.",
    },
    MoveDisplayUp => {
        id: "editor.vim.motion.display_up",
        action: MoveAction { motion: Motion::Up { display_lines: true } },
        title: "Editor Vim: Move Display Line Up",
        description: "Move the Vim cursor up by display lines.",
    },
    MoveNextWordStart => {
        id: "editor.vim.motion.next_word_start",
        action: MoveAction { motion: Motion::NextWordStart },
        title: "Editor Vim: Next Word",
        description: "Move to the start of the next word.",
    },
    MoveNextWordEnd => {
        id: "editor.vim.motion.next_word_end",
        action: MoveAction { motion: Motion::NextWordEnd },
        title: "Editor Vim: Next Word End",
        description: "Move to the end of the next word.",
    },
    MovePreviousWordStart => {
        id: "editor.vim.motion.previous_word_start",
        action: MoveAction { motion: Motion::PreviousWordStart },
        title: "Editor Vim: Previous Word",
        description: "Move to the start of the previous word.",
    },
    MoveLineStart => {
        id: "editor.vim.motion.line_start",
        action: MoveAction { motion: Motion::StartOfLine },
        title: "Editor Vim: Line Start",
        description: "Move to the start of the current line.",
    },
    MoveFirstNonWhitespace => {
        id: "editor.vim.motion.first_non_whitespace",
        action: MoveAction { motion: Motion::FirstNonWhitespace },
        title: "Editor Vim: First Non-Whitespace",
        description: "Move to the first non-whitespace character.",
    },
    MoveLineEnd => {
        id: "editor.vim.motion.line_end",
        action: MoveAction { motion: Motion::EndOfLine },
        title: "Editor Vim: Line End",
        description: "Move to the end of the current line.",
    },
    MoveDocumentStart => {
        id: "editor.vim.motion.document_start",
        action: MoveAction { motion: Motion::StartOfDocument },
        title: "Editor Vim: Document Start",
        description: "Move to the start of the document.",
    },
    MoveDocumentEnd => {
        id: "editor.vim.motion.document_end",
        action: MoveAction { motion: Motion::EndOfDocument },
        title: "Editor Vim: Document End",
        description: "Move to the end of the document.",
    },
    CountZero => {
        id: "editor.vim.count.zero",
        action: Zero,
        title: "Editor Vim: Zero",
        description: "Enter zero in a count or move to line start.",
    },
    CountOne => {
        id: "editor.vim.count.one",
        action: Number(1),
        title: "Editor Vim: Count 1",
        description: "Append 1 to the Vim count.",
    },
    CountTwo => {
        id: "editor.vim.count.two",
        action: Number(2),
        title: "Editor Vim: Count 2",
        description: "Append 2 to the Vim count.",
    },
    CountThree => {
        id: "editor.vim.count.three",
        action: Number(3),
        title: "Editor Vim: Count 3",
        description: "Append 3 to the Vim count.",
    },
    CountFour => {
        id: "editor.vim.count.four",
        action: Number(4),
        title: "Editor Vim: Count 4",
        description: "Append 4 to the Vim count.",
    },
    CountFive => {
        id: "editor.vim.count.five",
        action: Number(5),
        title: "Editor Vim: Count 5",
        description: "Append 5 to the Vim count.",
    },
    CountSix => {
        id: "editor.vim.count.six",
        action: Number(6),
        title: "Editor Vim: Count 6",
        description: "Append 6 to the Vim count.",
    },
    CountSeven => {
        id: "editor.vim.count.seven",
        action: Number(7),
        title: "Editor Vim: Count 7",
        description: "Append 7 to the Vim count.",
    },
    CountEight => {
        id: "editor.vim.count.eight",
        action: Number(8),
        title: "Editor Vim: Count 8",
        description: "Append 8 to the Vim count.",
    },
    CountNine => {
        id: "editor.vim.count.nine",
        action: Number(9),
        title: "Editor Vim: Count 9",
        description: "Append 9 to the Vim count.",
    },
    DeleteOperator => {
        id: "editor.vim.operator.delete",
        action: PushOperator { operator: Operator::Delete },
        title: "Editor Vim: Delete Operator",
        description: "Begin a Vim delete operation.",
    },
    ChangeOperator => {
        id: "editor.vim.operator.change",
        action: PushOperator { operator: Operator::Change },
        title: "Editor Vim: Change Operator",
        description: "Begin a Vim change operation.",
    },
    YankOperator => {
        id: "editor.vim.operator.yank",
        action: PushOperator { operator: Operator::Yank },
        title: "Editor Vim: Yank Operator",
        description: "Begin a Vim yank operation.",
    },
    InsertCurrent => {
        id: "editor.vim.insert.current",
        action: EnterInsert { placement: InsertPlacement::Current },
        title: "Editor Vim: Insert",
        description: "Enter insert mode at the cursor.",
    },
    InsertAfter => {
        id: "editor.vim.insert.after",
        action: EnterInsert { placement: InsertPlacement::After },
        title: "Editor Vim: Append",
        description: "Enter insert mode after the cursor.",
    },
    InsertFirstNonWhitespace => {
        id: "editor.vim.insert.first_non_whitespace",
        action: EnterInsert { placement: InsertPlacement::FirstNonWhitespace },
        title: "Editor Vim: Insert at Indentation",
        description: "Enter insert mode at the first non-whitespace character.",
    },
    InsertLineEnd => {
        id: "editor.vim.insert.line_end",
        action: EnterInsert { placement: InsertPlacement::EndOfLine },
        title: "Editor Vim: Append at Line End",
        description: "Enter insert mode at the end of the line.",
    },
    InsertLineBelow => {
        id: "editor.vim.insert.line_below",
        action: EnterInsert { placement: InsertPlacement::NewLineBelow },
        title: "Editor Vim: Open Line Below",
        description: "Open a new line below and enter insert mode.",
    },
    InsertLineAbove => {
        id: "editor.vim.insert.line_above",
        action: EnterInsert { placement: InsertPlacement::NewLineAbove },
        title: "Editor Vim: Open Line Above",
        description: "Open a new line above and enter insert mode.",
    },
    ToggleVisual => {
        id: "editor.vim.visual.toggle",
        action: ToggleVisual,
        title: "Editor Vim: Toggle Visual",
        description: "Enter or leave character-wise visual mode.",
    },
    ToggleVisualLine => {
        id: "editor.vim.visual.toggle_line",
        action: ToggleVisualLine,
        title: "Editor Vim: Toggle Visual Line",
        description: "Enter or leave line-wise visual mode.",
    },
    DeleteCharacters => {
        id: "editor.vim.delete_characters",
        action: DeleteCharacters,
        title: "Editor Vim: Delete Characters",
        description: "Delete characters under the cursor.",
    },
    SubstituteCharacters => {
        id: "editor.vim.substitute_characters",
        action: SubstituteCharacters,
        title: "Editor Vim: Substitute Characters",
        description: "Delete characters and enter insert mode.",
    },
    ReplaceCharacters => {
        id: "editor.vim.replace_characters",
        action: ReplaceCharacters,
        title: "Editor Vim: Replace Characters",
        description: "Replace characters under the cursor.",
    },
    PasteAfter => {
        id: "editor.vim.paste_after",
        action: Paste { before: false },
        title: "Editor Vim: Paste After",
        description: "Paste after the cursor.",
    },
    PasteBefore => {
        id: "editor.vim.paste_before",
        action: Paste { before: true },
        title: "Editor Vim: Paste Before",
        description: "Paste before the cursor.",
    },
    Undo => {
        id: "editor.vim.undo",
        action: Undo,
        title: "Editor Vim: Undo",
        description: "Undo the previous edit.",
    },
    Redo => {
        id: "editor.vim.redo",
        action: Redo,
        title: "Editor Vim: Redo",
        description: "Redo the previous edit.",
    },
    SearchForward => {
        id: "editor.vim.search_forward",
        action: SearchForward,
        title: "Editor Vim: Search",
        description: "Open forward search.",
    },
    SearchNext => {
        id: "editor.vim.search_next",
        action: SearchNext,
        title: "Editor Vim: Next Search Match",
        description: "Move to the next search match.",
    },
    SearchPrevious => {
        id: "editor.vim.search_previous",
        action: SearchPrevious,
        title: "Editor Vim: Previous Search Match",
        description: "Move to the previous search match.",
    },
    Escape => {
        id: "editor.vim.escape",
        action: Escape,
        title: "Editor Vim: Escape",
        description: "Return to Vim normal mode.",
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorVimBindingSpec {
    pub keys: &'static str,
    pub action: EditorVimActionId,
    pub context: &'static str,
}

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
    rebind_keybindings(cx);
}

pub fn rebind_keybindings(cx: &mut App) {
    cx.bind_keys(default_keybindings());
}

fn default_keybindings() -> Vec<KeyBinding> {
    default_bindable_keybindings()
        .into_iter()
        .map(|binding| {
            binding
                .action
                .key_binding(binding.keys, Some(binding.context))
        })
        .collect()
}

pub fn default_bindable_keybindings() -> Vec<EditorVimBindingSpec> {
    vec![
        editor_binding(
            "h",
            EditorVimActionId::MoveLeft,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "left",
            EditorVimActionId::MoveLeft,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "l",
            EditorVimActionId::MoveRight,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "right",
            EditorVimActionId::MoveRight,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "j",
            EditorVimActionId::MoveDown,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "down",
            EditorVimActionId::MoveDown,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding("k", EditorVimActionId::MoveUp, VIM_CONTROL_BINDING_CONTEXT),
        editor_binding("up", EditorVimActionId::MoveUp, VIM_CONTROL_BINDING_CONTEXT),
        editor_binding(
            "g j",
            EditorVimActionId::MoveDisplayDown,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "g k",
            EditorVimActionId::MoveDisplayUp,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "w",
            EditorVimActionId::MoveNextWordStart,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "e",
            EditorVimActionId::MoveNextWordEnd,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "b",
            EditorVimActionId::MovePreviousWordStart,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "0",
            EditorVimActionId::CountZero,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "home",
            EditorVimActionId::MoveLineStart,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "^",
            EditorVimActionId::MoveFirstNonWhitespace,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "$",
            EditorVimActionId::MoveLineEnd,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "end",
            EditorVimActionId::MoveLineEnd,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "g g",
            EditorVimActionId::MoveDocumentStart,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "shift-g",
            EditorVimActionId::MoveDocumentEnd,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "1",
            EditorVimActionId::CountOne,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "2",
            EditorVimActionId::CountTwo,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "3",
            EditorVimActionId::CountThree,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "4",
            EditorVimActionId::CountFour,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "5",
            EditorVimActionId::CountFive,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "6",
            EditorVimActionId::CountSix,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "7",
            EditorVimActionId::CountSeven,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "8",
            EditorVimActionId::CountEight,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "9",
            EditorVimActionId::CountNine,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "d",
            EditorVimActionId::DeleteOperator,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "c",
            EditorVimActionId::ChangeOperator,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "y",
            EditorVimActionId::YankOperator,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding("i", EditorVimActionId::InsertCurrent, NORMAL_CONTEXT),
        editor_binding("a", EditorVimActionId::InsertAfter, NORMAL_CONTEXT),
        editor_binding(
            "shift-i",
            EditorVimActionId::InsertFirstNonWhitespace,
            NORMAL_CONTEXT,
        ),
        editor_binding("shift-a", EditorVimActionId::InsertLineEnd, NORMAL_CONTEXT),
        editor_binding("o", EditorVimActionId::InsertLineBelow, NORMAL_CONTEXT),
        editor_binding(
            "shift-o",
            EditorVimActionId::InsertLineAbove,
            NORMAL_CONTEXT,
        ),
        editor_binding(
            "v",
            EditorVimActionId::ToggleVisual,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding(
            "shift-v",
            EditorVimActionId::ToggleVisualLine,
            VIM_CONTROL_BINDING_CONTEXT,
        ),
        editor_binding("x", EditorVimActionId::DeleteCharacters, NORMAL_CONTEXT),
        editor_binding("s", EditorVimActionId::SubstituteCharacters, NORMAL_CONTEXT),
        editor_binding("r", EditorVimActionId::ReplaceCharacters, NORMAL_CONTEXT),
        editor_binding("p", EditorVimActionId::PasteAfter, NORMAL_CONTEXT),
        editor_binding("shift-p", EditorVimActionId::PasteBefore, NORMAL_CONTEXT),
        editor_binding("u", EditorVimActionId::Undo, NORMAL_CONTEXT),
        editor_binding("ctrl-r", EditorVimActionId::Redo, NORMAL_CONTEXT),
        editor_binding("/", EditorVimActionId::SearchForward, NORMAL_CONTEXT),
        editor_binding("n", EditorVimActionId::SearchNext, NORMAL_CONTEXT),
        editor_binding("shift-n", EditorVimActionId::SearchPrevious, NORMAL_CONTEXT),
        editor_binding(
            "escape",
            EditorVimActionId::Escape,
            VIM_EDITOR_BINDING_CONTEXT,
        ),
        editor_binding(
            "ctrl-[",
            EditorVimActionId::Escape,
            VIM_EDITOR_BINDING_CONTEXT,
        ),
        editor_binding("escape", EditorVimActionId::Escape, INSERT_CONTEXT),
    ]
}

fn editor_binding(
    keys: &'static str,
    action: EditorVimActionId,
    context: &'static str,
) -> EditorVimBindingSpec {
    EditorVimBindingSpec {
        keys,
        action,
        context,
    }
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
