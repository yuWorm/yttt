use gpui::{
    Context, Div, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, Render,
    Window, div, prelude::*, rgb,
};

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::{
    commands::{
        CommandDispatchError, CommandId, CommandRegistry, default_registry,
        dispatch_workspace_command,
    },
    model::{
        layout::{LayoutNode, PaneConfig, ProjectLayout},
        workspace::{Workspace, WorkspaceError},
    },
    palette::{
        ActivePalette, PaletteItem, PaletteKind, RecentProject, command_palette_items,
        pane_palette_items, project_palette_items, tab_palette_items,
    },
    ui::{
        actions::{
            OpenCommandPalette, OpenPanePalette, OpenProjectPalette, OpenTabPalette, PaletteCancel,
            PaletteConfirm, PaletteSelectNext, PaletteSelectPrev, PaneClose, PaneSplitHorizontal,
            PaneSplitVertical, TabNext, TabPrev, WORKSPACE_CONTEXT,
        },
        palette::palette_overlay,
        sidebar::project_sidebar,
        split_view::split_view_for_layout,
        tabs::project_tabs,
        terminal_pane::TerminalPaneView,
    },
};

pub struct RootView {
    workspace: Workspace,
    active_palette: Option<ActivePalette>,
    command_registry: CommandRegistry,
    recent_projects: Vec<RecentProject>,
    focus_handle: Option<FocusHandle>,
    terminal_panes: HashMap<String, Entity<TerminalPaneView>>,
}

impl RootView {
    pub fn new() -> Self {
        Self {
            workspace: Workspace::new(),
            active_palette: None,
            command_registry: default_registry(),
            recent_projects: Vec::new(),
            focus_handle: None,
            terminal_panes: HashMap::new(),
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn active_palette(&self) -> Option<&ActivePalette> {
        self.active_palette.as_ref()
    }

    pub fn open_palette(&mut self, kind: PaletteKind) {
        self.active_palette = Some(ActivePalette::new(kind));
    }

    pub fn close_palette(&mut self) {
        self.active_palette = None;
    }

    pub fn set_palette_query(&mut self, query: impl Into<String>) {
        if let Some(active_palette) = &mut self.active_palette {
            active_palette.query = query.into();
            active_palette.selected_index = 0;
        }
    }

    pub fn confirm_palette_selection(&mut self) -> Result<(), RootViewError> {
        let Some(active_palette) = self.active_palette.clone() else {
            return Ok(());
        };
        let items = self.palette_items(active_palette.kind);
        let Some(item) = active_palette.selected_item(&items).cloned() else {
            return Ok(());
        };

        match active_palette.kind {
            PaletteKind::Command => {
                dispatch_workspace_command(&mut self.workspace, item.command)?;
            }
            PaletteKind::Project => {
                let project_id = self
                    .workspace
                    .opened_projects()
                    .iter()
                    .find(|project| project.id.as_str() == item.id)
                    .map(|project| project.id.clone());
                if let Some(project_id) = project_id {
                    self.workspace.select_project(&project_id)?;
                }
            }
            PaletteKind::Tab => {
                self.workspace.select_tab(&item.id)?;
            }
            PaletteKind::Pane => {
                self.workspace.focus_pane(&item.id)?;
            }
        }

        self.close_palette();
        Ok(())
    }

    pub fn active_palette_items(&self) -> Vec<PaletteItem> {
        let Some(active_palette) = &self.active_palette else {
            return Vec::new();
        };

        self.palette_items(active_palette.kind)
    }

    pub fn visible_palette_titles(&self) -> Vec<String> {
        let Some(active_palette) = &self.active_palette else {
            return Vec::new();
        };
        let items = self.palette_items(active_palette.kind);

        active_palette
            .filtered_items(&items)
            .into_iter()
            .map(|item| item.title.clone())
            .collect()
    }

    pub fn dev_fixture() -> Self {
        let mut workspace = Workspace::new();
        workspace
            .open_project(PathBuf::from("/tmp/yttt"), dev_fixture_layout())
            .expect("dev fixture layout should be valid");
        Self {
            workspace,
            active_palette: None,
            command_registry: default_registry(),
            recent_projects: Vec::new(),
            focus_handle: None,
            terminal_panes: HashMap::new(),
        }
    }

    fn palette_items(&self, kind: PaletteKind) -> Vec<PaletteItem> {
        match kind {
            PaletteKind::Command => command_palette_items(&self.command_registry),
            PaletteKind::Project => project_palette_items(&self.workspace, &self.recent_projects),
            PaletteKind::Tab => tab_palette_items(&self.workspace).unwrap_or_default(),
            PaletteKind::Pane => pane_palette_items(&self.workspace).unwrap_or_default(),
        }
    }

    fn select_next_palette_item(&mut self) -> bool {
        let Some(kind) = self.active_palette.as_ref().map(|palette| palette.kind) else {
            return false;
        };
        let items = self.palette_items(kind);
        let Some(active_palette) = &mut self.active_palette else {
            return false;
        };

        active_palette.select_next(&items);
        true
    }

    fn select_prev_palette_item(&mut self) -> bool {
        let Some(kind) = self.active_palette.as_ref().map(|palette| palette.kind) else {
            return false;
        };
        let items = self.palette_items(kind);
        let Some(active_palette) = &mut self.active_palette else {
            return false;
        };

        active_palette.select_prev(&items);
        true
    }

    fn append_palette_query(&mut self, value: char) -> bool {
        let Some(active_palette) = &mut self.active_palette else {
            return false;
        };

        active_palette.query.push(value);
        active_palette.selected_index = 0;
        true
    }

    fn pop_palette_query(&mut self) -> bool {
        let Some(active_palette) = &mut self.active_palette else {
            return false;
        };

        active_palette.query.pop();
        active_palette.selected_index = 0;
        true
    }

    fn root_focus_handle(&mut self, cx: &mut Context<Self>) -> FocusHandle {
        if let Some(focus_handle) = &self.focus_handle {
            return focus_handle.clone();
        }

        let focus_handle = cx.focus_handle();
        self.focus_handle = Some(focus_handle.clone());
        focus_handle
    }

    fn active_terminal_split_view(&mut self, cx: &mut Context<Self>) -> Div {
        self.prune_terminal_panes();

        let Some((project_id, tab_id, layout)) = self.selected_tab_layout_clone() else {
            return div();
        };

        let mut render_pane = |pane: &PaneConfig, tab_id: &str| {
            self.render_terminal_pane(&project_id, pane, tab_id, cx)
        };

        div()
            .flex()
            .flex_1()
            .bg(rgb(0x0a0a0a))
            .text_color(rgb(0xf5f5f5))
            .p_2()
            .child(split_view_for_layout(&layout, &tab_id, &mut render_pane))
    }

    fn selected_tab_layout_clone(&self) -> Option<(String, String, LayoutNode)> {
        let selected_project_id = self.workspace.selected_project_id()?;
        let project = self.workspace.project(selected_project_id)?;
        let tab = project
            .layout
            .tabs
            .iter()
            .find(|tab| tab.id == project.selected_tab_id)?;

        Some((
            selected_project_id.as_str().to_string(),
            project.selected_tab_id.clone(),
            tab.layout.clone(),
        ))
    }

    fn render_terminal_pane(
        &mut self,
        project_id: &str,
        pane: &PaneConfig,
        tab_id: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let key = terminal_pane_key(project_id, tab_id, &pane.id);
        let pane_view = self
            .terminal_panes
            .entry(key)
            .or_insert_with(|| cx.new(|cx| TerminalPaneView::new(pane.clone(), cx)))
            .clone();

        div().flex().flex_1().child(pane_view)
    }

    fn prune_terminal_panes(&mut self) {
        let mut live_keys = HashSet::new();
        for project in self.workspace.opened_projects() {
            for tab in &project.layout.tabs {
                collect_terminal_pane_keys(
                    project.id.as_str(),
                    &tab.id,
                    &tab.layout,
                    &mut live_keys,
                );
            }
        }

        self.terminal_panes
            .retain(|key, _pane| live_keys.contains(key));
    }

    fn on_open_command_palette(
        &mut self,
        _: &OpenCommandPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Command);
        cx.notify();
    }

    fn on_open_project_palette(
        &mut self,
        _: &OpenProjectPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Project);
        cx.notify();
    }

    fn on_open_tab_palette(
        &mut self,
        _: &OpenTabPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Tab);
        cx.notify();
    }

    fn on_open_pane_palette(
        &mut self,
        _: &OpenPanePalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Pane);
        cx.notify();
    }

    fn on_palette_select_next(
        &mut self,
        _: &PaletteSelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.select_next_palette_item() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn on_palette_select_prev(
        &mut self,
        _: &PaletteSelectPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.select_prev_palette_item() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn on_palette_confirm(
        &mut self,
        _: &PaletteConfirm,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_palette.is_some() {
            let _ = self.confirm_palette_selection();
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn on_palette_cancel(
        &mut self,
        _: &PaletteCancel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_palette.is_some() {
            self.close_palette();
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn on_tab_next(&mut self, _: &TabNext, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_workspace_action(CommandId::TabNext, cx);
    }

    fn on_tab_prev(&mut self, _: &TabPrev, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_workspace_action(CommandId::TabPrev, cx);
    }

    fn on_pane_split_vertical(
        &mut self,
        _: &PaneSplitVertical,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_workspace_action(CommandId::PaneSplitVertical, cx);
    }

    fn on_pane_split_horizontal(
        &mut self,
        _: &PaneSplitHorizontal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_workspace_action(CommandId::PaneSplitHorizontal, cx);
    }

    fn on_pane_close(&mut self, _: &PaneClose, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_workspace_action(CommandId::PaneClose, cx);
    }

    fn dispatch_workspace_action(&mut self, command_id: CommandId, cx: &mut Context<Self>) {
        if self.active_palette.is_some() {
            cx.propagate();
            return;
        }

        let _ = dispatch_workspace_command(&mut self.workspace, command_id);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.active_palette.is_none() {
            cx.propagate();
            return;
        }

        if event.keystroke.key == "backspace" {
            self.pop_palette_query();
            cx.stop_propagation();
            cx.notify();
            return;
        }

        let has_command_modifier = event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.function;
        if has_command_modifier {
            cx.propagate();
            return;
        }

        let Some(key_char) = event.keystroke.key_char.as_deref() else {
            cx.propagate();
            return;
        };
        let mut chars = key_char.chars();
        let Some(value) = chars.next() else {
            cx.propagate();
            return;
        };
        if chars.next().is_none() && !value.is_control() && self.append_palette_query(value) {
            cx.stop_propagation();
            cx.notify();
        } else {
            cx.propagate();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RootViewError {
    #[error("{0}")]
    Command(#[from] CommandDispatchError),
    #[error("{0}")]
    Workspace(#[from] WorkspaceError),
}

impl Default for RootView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus_handle = self.root_focus_handle(cx);

        let mut root = if self.workspace.opened_projects().is_empty() {
            empty_workspace()
        } else {
            let split_view = self.active_terminal_split_view(cx);

            div()
                .flex()
                .size_full()
                .relative()
                .bg(rgb(0x101010))
                .text_color(rgb(0xf5f5f5))
                .child(project_sidebar(&self.workspace))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .child(project_tabs(&self.workspace))
                        .child(split_view),
                )
        };

        if let Some(active_palette) = &self.active_palette {
            let items = self.palette_items(active_palette.kind);
            root = root.child(palette_overlay(active_palette, &items));
        }

        if !focus_handle.contains_focused(window, cx) {
            focus_handle.focus(window);
        }

        root.track_focus(&focus_handle)
            .key_context(WORKSPACE_CONTEXT)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::on_open_command_palette))
            .on_action(cx.listener(Self::on_open_project_palette))
            .on_action(cx.listener(Self::on_open_tab_palette))
            .on_action(cx.listener(Self::on_open_pane_palette))
            .on_action(cx.listener(Self::on_palette_select_next))
            .on_action(cx.listener(Self::on_palette_select_prev))
            .on_action(cx.listener(Self::on_palette_confirm))
            .on_action(cx.listener(Self::on_palette_cancel))
            .on_action(cx.listener(Self::on_tab_next))
            .on_action(cx.listener(Self::on_tab_prev))
            .on_action(cx.listener(Self::on_pane_split_vertical))
            .on_action(cx.listener(Self::on_pane_split_horizontal))
            .on_action(cx.listener(Self::on_pane_close))
    }
}

fn terminal_pane_key(project_id: &str, tab_id: &str, pane_id: &str) -> String {
    format!("{project_id}:{tab_id}:{pane_id}")
}

fn collect_terminal_pane_keys(
    project_id: &str,
    tab_id: &str,
    layout: &LayoutNode,
    keys: &mut HashSet<String>,
) {
    match layout {
        LayoutNode::Pane(pane) => {
            keys.insert(terminal_pane_key(project_id, tab_id, &pane.id));
        }
        LayoutNode::Split(split) => {
            collect_terminal_pane_keys(project_id, tab_id, &split.left, keys);
            collect_terminal_pane_keys(project_id, tab_id, &split.right, keys);
        }
    }
}

fn empty_workspace() -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .size_full()
        .relative()
        .justify_center()
        .items_center()
        .bg(rgb(0x101010))
        .text_color(rgb(0xf5f5f5))
        .child(div().text_xl().child("yttt"))
        .child("Open a directory or choose a recent project.")
        .child("Command Palette: Cmd/Ctrl+P")
}

fn dev_fixture_layout() -> ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "yttt"
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"

        [tabs.layout]
        type = "split"
        direction = "horizontal"
        ratio = 0.65
        left = { type = "pane", id = "server", title = "server", command = "$SHELL" }
        right = { type = "pane", id = "shell", title = "shell", command = "$SHELL" }

        [[tabs]]
        id = "agent"
        title = "Agent"
        layout = { type = "pane", id = "codex", title = "Codex", command = "codex", kind = "agent", notify_on_exit = true }
    "#,
    )
    .expect("static dev fixture TOML should parse")
}
