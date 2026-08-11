use gpui::KeyContext;

use crate::{
    config::settings::VimModeSetting,
    ui::{editor::VimMode as EditorVimMode, surface::WorkbenchSurface},
};

const MAX_KEY_FEEDBACK_ITEMS: usize = 6;

pub const VIM_CONTEXT: &str = "YtttVim";
pub const VIM_NORMAL_CONTEXT: &str =
    "YtttVim && yttt_vim_scope == global && yttt_vim_mode == normal";
pub const VIM_CONTROL_CONTEXT: &str =
    "YtttVim && yttt_vim_scope == global && yttt_vim_control == true";
pub const VIM_PROFILE_CONTEXT: &str =
    "(VimControl && !SearchPanel) || (YtttVim && yttt_vim_control == true)";
pub const VIM_ESCAPE_CONTEXT: &str = "YtttVim && yttt_vim_scope == global && yttt_vim_capture == inherit && yttt_vim_surface != editor && yttt_vim_surface != terminal";
pub const VIM_TERMINAL_CONTEXT: &str = "YtttVim && yttt_vim_scope == global && yttt_vim_mode == terminal && yttt_vim_surface == terminal";
pub const VIM_TERMINAL_NORMAL_CONTEXT: &str = "YtttVim && yttt_vim_scope == global && yttt_vim_mode == normal && yttt_vim_surface == terminal";
pub const VIM_PALETTE_NORMAL_CONTEXT: &str = "YtttVim && yttt_vim_scope == global && yttt_vim_mode == normal && yttt_vim_surface == palette && YtttPalette";
pub const VIM_SETTINGS_NORMAL_CONTEXT: &str = "YtttVim && yttt_vim_scope == global && yttt_vim_mode == normal && yttt_vim_surface == settings && !Input";
pub const VIM_PROJECTS_NORMAL_CONTEXT: &str = "YtttVim && yttt_vim_scope == global && yttt_vim_mode == normal && yttt_vim_surface == projects";
pub const VIM_PROJECT_TREE_NORMAL_CONTEXT: &str =
    "YtttVim && yttt_vim_scope == global && yttt_vim_mode == normal && yttt_vim_surface == tree";
pub const VIM_PROJECT_PANEL_NORMAL_CONTEXT: &str = "YtttVim && yttt_vim_scope == global && yttt_vim_mode == normal && yttt_vim_project_panel == true && !Input";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkbenchVimMode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
    Terminal,
}

impl WorkbenchVimMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual => "VISUAL",
            Self::VisualLine => "V-LINE",
            Self::Terminal => "TERMINAL",
        }
    }

    fn context_value(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Insert => "insert",
            Self::Visual | Self::VisualLine => "visual",
            Self::Terminal => "terminal",
        }
    }

    fn allows_control_commands(self) -> bool {
        matches!(self, Self::Normal | Self::Visual | Self::VisualLine)
    }
}

impl From<EditorVimMode> for WorkbenchVimMode {
    fn from(mode: EditorVimMode) -> Self {
        match mode {
            EditorVimMode::Normal => Self::Normal,
            EditorVimMode::Insert => Self::Insert,
            EditorVimMode::Visual => Self::Visual,
            EditorVimMode::VisualLine => Self::VisualLine,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VimCapture {
    #[default]
    Inherit,
    ForceInsert,
    Suspended,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimStatus {
    pub mode: WorkbenchVimMode,
    pub surface: WorkbenchSurface,
    pub detail: Option<String>,
    pub key_feedback: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct VimControllerState {
    support: VimModeSetting,
    mode: WorkbenchVimMode,
    surface: WorkbenchSurface,
    surface_initialized: bool,
    capture: VimCapture,
    detail: Option<String>,
    key_feedback: Vec<String>,
    key_feedback_generation: u64,
}

impl VimControllerState {
    pub fn new(support: VimModeSetting) -> Self {
        Self {
            support,
            mode: WorkbenchVimMode::Normal,
            surface: WorkbenchSurface::Workspace,
            capture: VimCapture::Inherit,
            surface_initialized: false,
            detail: None,
            key_feedback: Vec::new(),
            key_feedback_generation: 0,
        }
    }

    pub fn support(&self) -> VimModeSetting {
        self.support
    }

    pub fn set_support(&mut self, support: VimModeSetting) {
        self.support = support;
        self.mode = WorkbenchVimMode::Normal;
        self.clear_key_feedback();
        self.detail = None;
        self.capture = VimCapture::Inherit;
        self.surface_initialized = false;
    }

    pub fn surface(&self) -> WorkbenchSurface {
        self.surface
    }

    pub fn mode(&self) -> WorkbenchVimMode {
        self.mode
    }

    pub fn sync_surface(&mut self, surface: WorkbenchSurface) {
        let previous = self.surface;
        self.surface = surface;
        if !self.surface_initialized {
            self.surface_initialized = true;
        } else if previous == surface {
            return;
        }

        self.detail = None;
        if self.support != VimModeSetting::Global {
            return;
        }
        self.mode = match surface {
            WorkbenchSurface::Terminal => match self.mode {
                WorkbenchVimMode::Insert | WorkbenchVimMode::Terminal => WorkbenchVimMode::Terminal,
                WorkbenchVimMode::Visual | WorkbenchVimMode::VisualLine => WorkbenchVimMode::Normal,
                WorkbenchVimMode::Normal => WorkbenchVimMode::Normal,
            },
            WorkbenchSurface::Editor => match self.mode {
                WorkbenchVimMode::Terminal => WorkbenchVimMode::Insert,
                mode => mode,
            },
            WorkbenchSurface::Workspace
            | WorkbenchSurface::Projects
            | WorkbenchSurface::ProjectTree
            | WorkbenchSurface::Settings
            | WorkbenchSurface::Palette
            | WorkbenchSurface::GitDiff
            | WorkbenchSurface::Dialog => match self.mode {
                WorkbenchVimMode::Terminal => WorkbenchVimMode::Insert,
                WorkbenchVimMode::Visual | WorkbenchVimMode::VisualLine => WorkbenchVimMode::Normal,
                mode => mode,
            },
        };
    }

    pub fn sync_editor(&mut self, mode: EditorVimMode, detail: Option<String>) {
        if self.surface != WorkbenchSurface::Editor {
            return;
        }
        self.mode = mode.into();
        self.detail = detail;
    }

    pub fn enter_normal(&mut self) -> bool {
        if !self.enabled_for_surface() {
            return false;
        }
        let changed = self.mode != WorkbenchVimMode::Normal || self.detail.is_some();
        self.mode = WorkbenchVimMode::Normal;
        self.detail = None;
        changed
    }

    pub fn enter_insert(&mut self) -> bool {
        if !self.enabled_for_surface() {
            return false;
        }
        let changed = self.mode != WorkbenchVimMode::Insert || self.detail.is_some();
        self.mode = WorkbenchVimMode::Insert;
        self.detail = None;
        changed
    }

    pub fn enter_terminal(&mut self) -> bool {
        if self.support != VimModeSetting::Global || self.surface != WorkbenchSurface::Terminal {
            return false;
        }
        let changed = self.mode != WorkbenchVimMode::Terminal || self.detail.is_some();
        self.mode = WorkbenchVimMode::Terminal;
        self.detail = None;
        changed
    }

    pub fn set_capture(&mut self, capture: VimCapture) {
        self.capture = capture;
    }

    pub fn enabled_for_surface(&self) -> bool {
        match self.support {
            VimModeSetting::Global => true,
            VimModeSetting::Editor => self.surface == WorkbenchSurface::Editor,
            VimModeSetting::Disabled => false,
        }
    }

    pub fn effective_mode(&self, capture: VimCapture) -> Option<WorkbenchVimMode> {
        if !self.enabled_for_surface() || capture == VimCapture::Suspended {
            return None;
        }
        Some(match capture {
            VimCapture::ForceInsert => WorkbenchVimMode::Insert,
            VimCapture::Inherit => self.mode,
            VimCapture::Suspended => unreachable!("suspended Vim capture returned early"),
        })
    }

    pub fn key_context(&self, capture: VimCapture) -> KeyContext {
        let Some(mode) = self.effective_mode(capture) else {
            return KeyContext::new_with_defaults();
        };
        let mut context = KeyContext::new_with_defaults();
        context.add(VIM_CONTEXT);
        context.set(
            "yttt_vim_scope",
            match self.support {
                VimModeSetting::Global => "global",
                VimModeSetting::Editor => "editor",
                VimModeSetting::Disabled => "disabled",
            },
        );
        context.set("yttt_vim_mode", mode.context_value());
        context.set("yttt_vim_surface", self.surface.label());
        context.set(
            "yttt_vim_project_panel",
            if self.surface == WorkbenchSurface::ProjectTree {
                "true"
            } else {
                "false"
            },
        );
        context.set(
            "yttt_vim_capture",
            match capture {
                VimCapture::Inherit => "inherit",
                VimCapture::ForceInsert => "force-insert",
                VimCapture::Suspended => unreachable!("suspended Vim capture returned early"),
            },
        );
        context.set(
            "yttt_vim_control",
            if mode.allows_control_commands() {
                "true"
            } else {
                "false"
            },
        );
        context
    }

    pub fn current_key_context(&self) -> KeyContext {
        self.key_context(self.capture)
    }

    pub fn accepts_key_feedback(&self) -> bool {
        self.effective_mode(self.capture)
            .is_some_and(WorkbenchVimMode::allows_control_commands)
    }

    pub fn record_key_feedback(&mut self, key: String) -> Option<u64> {
        if !self.accepts_key_feedback() {
            self.clear_key_feedback();
            return None;
        }
        if self.key_feedback.len() >= MAX_KEY_FEEDBACK_ITEMS {
            self.key_feedback.remove(0);
        }
        self.key_feedback.push(key);
        self.key_feedback_generation = self.key_feedback_generation.wrapping_add(1);
        Some(self.key_feedback_generation)
    }

    pub fn replace_key_feedback(&mut self, mut keys: Vec<String>) -> Option<u64> {
        if !self.accepts_key_feedback() || keys.is_empty() {
            self.clear_key_feedback();
            return None;
        }
        if keys.len() > MAX_KEY_FEEDBACK_ITEMS {
            keys.drain(..keys.len() - MAX_KEY_FEEDBACK_ITEMS);
        }
        self.key_feedback = keys;
        self.key_feedback_generation = self.key_feedback_generation.wrapping_add(1);
        Some(self.key_feedback_generation)
    }

    pub fn clear_key_feedback(&mut self) -> bool {
        if self.key_feedback.is_empty() {
            return false;
        }
        self.key_feedback.clear();
        self.key_feedback_generation = self.key_feedback_generation.wrapping_add(1);
        true
    }

    pub fn expire_key_feedback(&mut self, generation: u64) -> bool {
        if generation != self.key_feedback_generation {
            return false;
        }
        self.clear_key_feedback()
    }

    pub fn status(&self, capture: VimCapture) -> Option<VimStatus> {
        if !self.enabled_for_surface() {
            return None;
        }
        let mode = match capture {
            VimCapture::ForceInsert => WorkbenchVimMode::Insert,
            VimCapture::Inherit | VimCapture::Suspended => self.mode,
        };
        Some(VimStatus {
            mode,
            surface: self.surface,
            detail: self.detail.clone(),
            key_feedback: self.key_feedback.clone(),
        })
    }

    pub fn current_status(&self) -> Option<VimStatus> {
        self.status(self.capture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_controller_tracks_one_mode_across_surfaces() {
        let mut vim = VimControllerState::new(VimModeSetting::Global);
        vim.sync_surface(WorkbenchSurface::Editor);
        vim.sync_editor(EditorVimMode::VisualLine, Some("2 lines".to_string()));
        assert_eq!(vim.mode(), WorkbenchVimMode::VisualLine);

        vim.sync_surface(WorkbenchSurface::Terminal);
        assert_eq!(vim.mode(), WorkbenchVimMode::Normal);
        assert!(vim.enter_terminal());
        assert_eq!(vim.mode(), WorkbenchVimMode::Terminal);

        vim.sync_surface(WorkbenchSurface::Settings);
        assert_eq!(vim.mode(), WorkbenchVimMode::Insert);
        assert!(vim.enter_normal());
        vim.sync_surface(WorkbenchSurface::Palette);
        assert!(vim.enter_insert());
        vim.sync_surface(WorkbenchSurface::Terminal);
        assert_eq!(vim.mode(), WorkbenchVimMode::Terminal);
    }

    #[test]
    fn capture_overrides_are_transient_and_can_suspend_vim() {
        let mut vim = VimControllerState::new(VimModeSetting::Global);
        vim.sync_surface(WorkbenchSurface::Settings);
        assert_eq!(
            vim.effective_mode(VimCapture::ForceInsert),
            Some(WorkbenchVimMode::Insert)
        );
        assert_eq!(vim.mode(), WorkbenchVimMode::Normal);
        assert_eq!(vim.effective_mode(VimCapture::Suspended), None);
        assert_eq!(
            vim.status(VimCapture::Suspended).map(|status| status.mode),
            Some(WorkbenchVimMode::Normal)
        );
    }

    #[test]
    fn project_panel_context_tracks_tree_surface_and_effective_mode() {
        let mut vim = VimControllerState::new(VimModeSetting::Global);
        vim.sync_surface(WorkbenchSurface::ProjectTree);
        let normal_context = vim.current_key_context();
        assert_eq!(
            normal_context
                .get("yttt_vim_project_panel")
                .map(AsRef::as_ref),
            Some("true")
        );
        assert_eq!(
            normal_context.get("yttt_vim_mode").map(AsRef::as_ref),
            Some("normal")
        );

        let insert_context = vim.key_context(VimCapture::ForceInsert);
        assert_eq!(
            insert_context
                .get("yttt_vim_project_panel")
                .map(AsRef::as_ref),
            Some("true")
        );
        assert_eq!(
            insert_context.get("yttt_vim_mode").map(AsRef::as_ref),
            Some("insert")
        );

        vim.sync_surface(WorkbenchSurface::Editor);
        assert_eq!(
            vim.current_key_context()
                .get("yttt_vim_project_panel")
                .map(AsRef::as_ref),
            Some("false")
        );
    }

    #[test]
    fn editor_only_and_disabled_scopes_do_not_leak_to_workspace() {
        let mut editor = VimControllerState::new(VimModeSetting::Editor);
        editor.sync_surface(WorkbenchSurface::Workspace);
        assert_eq!(editor.current_status(), None);
        editor.sync_surface(WorkbenchSurface::Editor);
        assert_eq!(
            editor.current_status().map(|status| status.mode),
            Some(WorkbenchVimMode::Normal)
        );
        let editor_context = editor.current_key_context();
        assert!(!editor_context.contains(VIM_NORMAL_CONTEXT));
        assert!(!editor_context.contains(VIM_CONTROL_CONTEXT));

        let mut disabled = VimControllerState::new(VimModeSetting::Disabled);
        disabled.sync_surface(WorkbenchSurface::Editor);
        assert_eq!(disabled.current_status(), None);
    }

    #[test]
    fn key_feedback_tracks_command_sequences_without_echoing_insert_input() {
        let mut vim = VimControllerState::new(VimModeSetting::Global);
        vim.sync_surface(WorkbenchSurface::Settings);

        let first = vim.record_key_feedback("G".to_string()).unwrap();
        let latest = vim.record_key_feedback("G".to_string()).unwrap();
        assert_eq!(vim.current_status().unwrap().key_feedback, vec!["G", "G"]);
        assert!(!vim.expire_key_feedback(first));
        assert!(vim.expire_key_feedback(latest));
        assert!(vim.current_status().unwrap().key_feedback.is_empty());

        vim.record_key_feedback("I".to_string()).unwrap();
        assert!(vim.enter_insert());
        assert_eq!(vim.record_key_feedback("secret".to_string()), None);
        assert!(vim.current_status().unwrap().key_feedback.is_empty());
    }
}
