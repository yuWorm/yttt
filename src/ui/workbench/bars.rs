use std::{
    collections::HashSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui::{
    AnyElement, Context, Font, FontFeatures, FontStyle, FontWeight, IntoElement as _, TextRun,
    Window, black, div, prelude::*, px,
};
use gpui_component::{Icon, IconName, StyledExt};
use yttt_agent_core::{AgentSnapshot, AgentTurnState, AgentViewState, WaitingReason};
use yttt_protocol::ssh::SshConnectionState as ConnectionState;

use super::{WorkbenchView, performance::PerformanceInfo, state::update::UpdateStatus};
use crate::{
    commands::CommandId,
    config::bars::{BarLayoutSettings, BarModuleSettings, ShellBarModule, bar_icon_path},
    model::project::ProjectLocation,
    ui::{
        editor::{EditorDiagnosticSeverity, WorkItemId},
        primitives::icon_button::{YtttIconButtonKind, yttt_icon_button},
        terminal::status::{agent_status_label, project_agent_status},
        theme::{UiStyle, WorkbenchTheme, current_ui_style, current_workbench_theme},
        vim::{VimStatus, WorkbenchVimMode},
        workbench::shell::bar::{BarHost, BarSections},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BarTone {
    #[default]
    Muted,
    Normal,
    Accent,
    Success,
    Warning,
    Danger,
}

struct BarModuleView {
    text: Option<String>,
    icon: Option<IconName>,
    icon_path: Option<&'static str>,
    tooltip: Option<String>,
    tone: BarTone,
    action: Option<CommandId>,
    empty: bool,
    emphasized: bool,
    highlighted: bool,
}

impl BarModuleView {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            icon: None,
            icon_path: None,
            tooltip: None,
            tone: BarTone::Muted,
            action: None,
            empty: false,
            emphasized: false,
            highlighted: false,
        }
    }
}

struct ActiveItemBarInfo {
    title: String,
    tooltip: String,
    icon: IconName,
}

struct EditorBarInfo {
    language: String,
    line: Option<usize>,
    character: Option<usize>,
    dirty: bool,
    diagnostics: (usize, usize, usize),
    selection: Option<crate::ui::editor::EditorSelectionInfo>,
    tab_size: usize,
    soft_wrap: bool,
}
struct TerminalBarInfo {
    title: String,
    running: bool,
    exit: Option<(Option<i32>, yttt_terminal::ExitReason)>,
    viewport_size: Option<(usize, usize)>,
}

struct AgentBarInfo {
    state: AgentViewState,
    waiting_reason: Option<WaitingReason>,
    waiting_message: Option<String>,
    model: Option<String>,
    active_children: usize,
    state_started_at: u64,
}

struct SshBarInfo {
    text: String,
    tooltip: String,
    tone: BarTone,
}

struct UpdateBarInfo {
    text: String,
    tooltip: String,
    tone: BarTone,
    empty: bool,
}

struct ShellBarData {
    project_name: String,
    project_path: Option<String>,
    active_item: Option<ActiveItemBarInfo>,
    editor: Option<EditorBarInfo>,
    terminal: Option<TerminalBarInfo>,
    git_branch: Option<String>,
    git_changes: Option<(String, bool)>,
    agent_state: Option<AgentViewState>,
    active_agent: Option<AgentBarInfo>,
    ssh: Option<SshBarInfo>,
    update: UpdateBarInfo,
    surface: &'static str,
    vim: Option<VimStatus>,
    performance: Option<PerformanceInfo>,
}

enum BarSectionEntry {
    Content(AnyElement),
    Space(AnyElement),
    Separator(AnyElement),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BarUnavailableReason {
    NoProject,
    NoEditor,
    NoCodeEditor,
    NoSelection,
    NoTerminal,
    TerminalNotExited,
    TerminalSizeUnavailable,
    NoAgent,
    AgentNotWaiting,
    AgentModelUnavailable,
    NoActiveChildren,
    NoGit,
    GitClean,
    NoSsh,
    VimDisabled,
    NoVimDetail,
    NoVimKeys,
    EditorClean,
    NoDiagnostics,
    PerformanceUnavailable,
    NoUpdate,
}

impl WorkbenchView {
    pub(super) fn shell_bar_sections(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (BarSections, Option<BarSections>) {
        self.sync_bar_state_duration_refresh(cx);
        let data = self.shell_bar_data(cx, None);
        let window_sections = self.render_bar_sections(
            BarHost::Window,
            &self.app_settings.bars.window.layout,
            &data,
            None,
            true,
            window,
            cx,
        );

        let status_enabled = self.app_settings.bars.status.enabled;
        let profile_control_required = self.terminal.host_runtime.is_some();
        let empty_layout = BarLayoutSettings::default();
        let status_layout = if status_enabled {
            &self.app_settings.bars.status.layout
        } else {
            &empty_layout
        };
        let status = if status_enabled || profile_control_required {
            let profile = profile_control_required
                .then(|| self.profile_control_banner(cx).into_any_element());
            Some(self.render_bar_sections(
                BarHost::Status,
                status_layout,
                &data,
                profile,
                true,
                window,
                cx,
            ))
        } else {
            None
        };
        (window_sections, status)
    }

    pub(super) fn shell_bar_preview_sections(
        &self,
        host: BarHost,
        layout: &BarLayoutSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> BarSections {
        let data = self.shell_bar_data(cx, Some(layout));
        self.render_bar_sections(host, layout, &data, None, false, window, cx)
    }

    fn render_bar_sections(
        &self,
        host: BarHost,
        layout: &BarLayoutSettings,
        data: &ShellBarData,
        left_prefix: Option<AnyElement>,
        interactive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> BarSections {
        BarSections {
            left: self.render_bar_section(
                host,
                "left",
                &layout.left,
                layout,
                data,
                left_prefix,
                interactive,
                window,
                cx,
            ),
            center: self.render_bar_section(
                host,
                "center",
                &layout.center,
                layout,
                data,
                None,
                interactive,
                window,
                cx,
            ),
            right: self.render_bar_section(
                host,
                "right",
                &layout.right,
                layout,
                data,
                None,
                interactive,
                window,
                cx,
            ),
        }
    }

    fn render_bar_section(
        &self,
        host: BarHost,
        section: &'static str,
        modules: &[ShellBarModule],
        layout: &BarLayoutSettings,
        data: &ShellBarData,
        prefix: Option<AnyElement>,
        interactive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let appearance = self.theme_runtime();
        let space_advance = modules
            .iter()
            .any(|module| matches!(module, ShellBarModule::Space(count) if *count > 0))
            .then(|| bar_space_advance(window, &appearance.typography.font_family));
        let mut entries = Vec::with_capacity(modules.len() + usize::from(prefix.is_some()));
        if let Some(prefix) = prefix {
            entries.push(BarSectionEntry::Content(prefix));
        }

        for (index, module) in modules.iter().enumerate() {
            let settings = layout.module_settings(module);
            match module {
                ShellBarModule::Space(count) if *count > 0 => {
                    entries.push(BarSectionEntry::Space(render_bar_space(
                        host,
                        section,
                        index,
                        space_advance.expect("space advance is calculated for visible spaces")
                            * f32::from(*count),
                    )));
                }
                ShellBarModule::Text(text) if !text.is_empty() => {
                    entries.push(BarSectionEntry::Content(render_bar_module(
                        host,
                        section,
                        index,
                        module,
                        settings,
                        BarModuleView::text(text.clone()),
                        interactive,
                        cx,
                    )));
                }
                ShellBarModule::Icon(name) => {
                    let Some(path) = bar_icon_path(name) else {
                        continue;
                    };
                    let mut view = BarModuleView::text("");
                    view.icon_path = Some(path);
                    entries.push(BarSectionEntry::Content(render_bar_module(
                        host,
                        section,
                        index,
                        module,
                        settings,
                        view,
                        interactive,
                        cx,
                    )));
                }
                ShellBarModule::Separator => {
                    entries.push(BarSectionEntry::Separator(render_bar_separator(
                        host,
                        section,
                        index,
                        appearance.ui,
                    )));
                }
                ShellBarModule::Space(_) | ShellBarModule::Text(_) => {}
                _ => {
                    let Some(view) = bar_module_view(module, data, settings) else {
                        continue;
                    };
                    entries.push(BarSectionEntry::Content(render_bar_module(
                        host,
                        section,
                        index,
                        module,
                        settings,
                        view,
                        interactive,
                        cx,
                    )));
                }
            }
        }

        normalized_bar_section(entries, appearance.style)
    }

    fn shell_bar_data(
        &self,
        cx: &gpui::App,
        preview_layout: Option<&BarLayoutSettings>,
    ) -> ShellBarData {
        let selected_project = self
            .workspace
            .selected_project_id()
            .and_then(|project_id| self.workspace.project(project_id));
        let project_name = selected_project
            .map(|project| project.layout.project.name.clone())
            .unwrap_or_else(|| {
                self.ui_text
                    .get(crate::ui::i18n::UiTextKey::AppName)
                    .to_string()
            });
        let project_path = selected_project.map(|project| {
            super::shell::titlebar::display_path_for_titlebar(&project.location.display_path())
        });
        let git_status = self
            .workspace
            .selected_project_id()
            .and_then(|project_id| self.project.project_git_statuses.get(project_id));
        let git_branch = git_status.and_then(|status| status.branch.clone());
        let git_changes = git_status.map(|status| git_changes_label(&status.summary));
        let agent_state = selected_project.and_then(project_agent_status);
        let ssh = selected_project.and_then(|project| match &project.location {
            ProjectLocation::Ssh { connection_id, .. } => {
                let connection = self
                    .ssh
                    .connections
                    .connections
                    .iter()
                    .find(|connection| &connection.id == connection_id);
                let name = connection
                    .map(|connection| connection.name.as_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| connection_id.as_str());
                let host = connection
                    .map(|connection| format!("{}@{}", connection.user, connection.host))
                    .unwrap_or_else(|| connection_id.as_str().to_string());
                let state = self
                    .ssh
                    .statuses
                    .get(connection_id)
                    .map(|status| status.state)
                    .unwrap_or(ConnectionState::Disconnected);
                Some(SshBarInfo {
                    text: format!("{name} · {}", ssh_state_label(state)),
                    tooltip: host,
                    tone: ssh_state_tone(state),
                })
            }
            ProjectLocation::Local { .. } => None,
        });

        let active_document = self.active_editor_document();
        let editor = active_document.as_ref().map(|document| {
            let document = document.read(cx);
            let editor = document.model().editor();
            let position = document
                .code_input()
                .map(|input| input.read(cx).cursor_position());
            let diagnostics = editor.diagnostics().iter().fold(
                (0, 0, 0),
                |(errors, warnings, infos), diagnostic| match diagnostic.severity {
                    EditorDiagnosticSeverity::Error => (errors + 1, warnings, infos),
                    EditorDiagnosticSeverity::Warning => (errors, warnings + 1, infos),
                    EditorDiagnosticSeverity::Info | EditorDiagnosticSeverity::Hint => {
                        (errors, warnings, infos + 1)
                    }
                },
            );
            EditorBarInfo {
                language: editor.language().to_string(),
                line: position.map(|position| position.line as usize + 1),
                character: position.map(|position| position.character as usize + 1),
                dirty: document.model().is_dirty(),
                diagnostics,
                selection: document.bar_selection_info(),
                tab_size: editor.config().tab_size(),
                soft_wrap: document.appearance().soft_wrap,
            }
        });
        let terminal = self.active_terminal_pane().map(|pane| {
            let pane = pane.read(cx);
            TerminalBarInfo {
                title: pane.title().to_string(),
                running: pane.is_running(),
                exit: pane.terminal_exit(),
                viewport_size: pane.terminal_viewport_size(),
            }
        });
        let active_item = match self.active_work_item() {
            Some(WorkItemId::File(document_id)) => active_document.as_ref().map(|document| {
                let document = document.read(cx);
                ActiveItemBarInfo {
                    title: document.model().editor().config().title().to_string(),
                    tooltip: document_id.canonical_path.display().to_string(),
                    icon: IconName::File,
                }
            }),
            Some(WorkItemId::Terminal(tab_id)) => {
                terminal.as_ref().map(|terminal| ActiveItemBarInfo {
                    title: terminal.title.clone(),
                    tooltip: tab_id,
                    icon: IconName::SquareTerminal,
                })
            }
            None => None,
        };
        let performance = [
            ShellBarModule::ProjectsCount,
            ShellBarModule::TerminalsCount,
            ShellBarModule::TabsCount,
            ShellBarModule::EditorsCount,
            ShellBarModule::AppCpu,
            ShellBarModule::AppMemory,
            ShellBarModule::SystemCpu,
            ShellBarModule::SystemMemory,
        ]
        .iter()
        .any(|module| {
            self.app_settings.bars.contains(module)
                || preview_layout.is_some_and(|layout| layout.contains(module))
        })
        .then(|| self.visible_performance_info())
        .flatten();

        ShellBarData {
            project_name,
            project_path,
            active_item,
            editor,
            terminal,
            git_branch,
            git_changes,
            agent_state,
            active_agent: self.active_agent_bar_info(),
            ssh,
            update: update_bar_info(&self.update.status, &self.ui_text),
            surface: self.vim.surface().label(),
            vim: self.vim.current_status(),
            performance,
        }
    }

    fn active_agent_snapshot(&self) -> Option<&AgentSnapshot> {
        self.active_terminal_pane()?;
        let project = self
            .workspace
            .selected_project_id()
            .and_then(|project_id| self.workspace.project(project_id))?;
        let tab = project.tab_state(&project.selected_tab_id)?;
        let pane_id = tab.focused_pane_id.as_deref()?;
        tab.pane_states
            .iter()
            .find(|pane| pane.pane_id == pane_id)?
            .agent_snapshot
            .as_ref()
    }

    fn active_agent_bar_info(&self) -> Option<AgentBarInfo> {
        let snapshot = self.active_agent_snapshot()?;
        Some(AgentBarInfo {
            state: snapshot.view_state(),
            waiting_reason: snapshot.waiting_reason,
            waiting_message: snapshot
                .waiting_message
                .as_deref()
                .filter(|message| !message.trim().is_empty())
                .map(ToOwned::to_owned),
            model: snapshot
                .session
                .as_ref()
                .and_then(|session| session.model.as_deref())
                .filter(|model| !model.trim().is_empty())
                .map(ToOwned::to_owned),
            active_children: snapshot
                .children
                .iter()
                .filter(|child| {
                    matches!(
                        child.turn_state,
                        AgentTurnState::Working | AgentTurnState::Waiting
                    )
                })
                .count(),
            state_started_at: snapshot.state_started_at,
        })
    }

    fn agent_state_duration_requested(&self) -> bool {
        self.app_settings
            .bars
            .contains(&ShellBarModule::AgentStateDuration)
            || self
                .overlays
                .layout_toml_editor
                .as_ref()
                .and_then(|session| session.bars_preview())
                .is_some_and(|bars| bars.contains(&ShellBarModule::AgentStateDuration))
    }

    pub(super) fn sync_bar_state_duration_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.agent_state_duration_requested() || self.active_agent_snapshot().is_none() {
            self.bar_state_duration_refresh_task = None;
            return;
        }
        if self.bar_state_duration_refresh_task.is_some() {
            return;
        }
        self.bar_state_duration_refresh_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let _ = this.update(cx, |view, cx| {
                view.bar_state_duration_refresh_task = None;
                if view.agent_state_duration_requested() && view.active_agent_snapshot().is_some() {
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn bar_preview_unavailable(
        &self,
        layout: &BarLayoutSettings,
        cx: &gpui::App,
    ) -> Vec<(String, BarUnavailableReason)> {
        let data = self.shell_bar_data(cx, Some(layout));
        let mut seen = HashSet::new();
        layout
            .left
            .iter()
            .chain(&layout.center)
            .chain(&layout.right)
            .filter_map(|module| {
                if !seen.insert(module.as_str()) {
                    return None;
                }
                let reason = self.bar_unavailable_reason(module, &data)?;
                let settings = layout.module_settings(module);
                if settings.hide_when_empty || bar_module_view(module, &data, settings).is_none() {
                    Some((module.as_str().to_string(), reason))
                } else {
                    None
                }
            })
            .collect()
    }

    fn bar_unavailable_reason(
        &self,
        module: &ShellBarModule,
        data: &ShellBarData,
    ) -> Option<BarUnavailableReason> {
        match module {
            ShellBarModule::ProjectPath => data
                .project_path
                .is_none()
                .then_some(BarUnavailableReason::NoProject),
            ShellBarModule::ActiveItem => data
                .active_item
                .is_none()
                .then_some(BarUnavailableReason::NoEditor),
            ShellBarModule::VimMode => data
                .vim
                .is_none()
                .then_some(BarUnavailableReason::VimDisabled),
            ShellBarModule::VimDetail => {
                let Some(status) = data.vim.as_ref() else {
                    return Some(BarUnavailableReason::VimDisabled);
                };
                status
                    .detail
                    .as_ref()
                    .filter(|detail| !detail.eq_ignore_ascii_case(status.mode.label()))
                    .is_none()
                    .then_some(BarUnavailableReason::NoVimDetail)
            }
            ShellBarModule::VimKeys => {
                let Some(status) = data.vim.as_ref() else {
                    return Some(BarUnavailableReason::VimDisabled);
                };
                status
                    .key_feedback
                    .is_empty()
                    .then_some(BarUnavailableReason::NoVimKeys)
            }
            ShellBarModule::EditorLanguage
            | ShellBarModule::EditorDirty
            | ShellBarModule::EditorDiagnostics
            | ShellBarModule::EditorSelection
            | ShellBarModule::EditorTabSize
            | ShellBarModule::EditorWrap => {
                let Some(editor) = data.editor.as_ref() else {
                    return Some(BarUnavailableReason::NoEditor);
                };
                match module {
                    ShellBarModule::EditorDirty => {
                        (!editor.dirty).then_some(BarUnavailableReason::EditorClean)
                    }
                    ShellBarModule::EditorDiagnostics => (editor.diagnostics == (0, 0, 0))
                        .then_some(BarUnavailableReason::NoDiagnostics),
                    ShellBarModule::EditorSelection => editor
                        .selection
                        .is_none()
                        .then_some(BarUnavailableReason::NoSelection),
                    _ => None,
                }
            }
            ShellBarModule::EditorPosition => {
                let Some(editor) = data.editor.as_ref() else {
                    return Some(BarUnavailableReason::NoEditor);
                };
                (editor.line.is_none() || editor.character.is_none())
                    .then_some(BarUnavailableReason::NoCodeEditor)
            }
            ShellBarModule::TerminalTitle | ShellBarModule::TerminalState => data
                .terminal
                .is_none()
                .then_some(BarUnavailableReason::NoTerminal),
            ShellBarModule::TerminalExit => {
                let Some(terminal) = data.terminal.as_ref() else {
                    return Some(BarUnavailableReason::NoTerminal);
                };
                terminal
                    .exit
                    .is_none()
                    .then_some(BarUnavailableReason::TerminalNotExited)
            }
            ShellBarModule::TerminalSize => {
                let Some(terminal) = data.terminal.as_ref() else {
                    return Some(BarUnavailableReason::NoTerminal);
                };
                terminal
                    .viewport_size
                    .is_none()
                    .then_some(BarUnavailableReason::TerminalSizeUnavailable)
            }
            ShellBarModule::GitBranch | ShellBarModule::GitChanges => {
                if matches!(module, ShellBarModule::GitChanges)
                    && data.git_changes.as_ref().is_some_and(|(_, clean)| *clean)
                {
                    return Some(BarUnavailableReason::GitClean);
                }
                let git = match module {
                    ShellBarModule::GitBranch => data.git_branch.is_some(),
                    ShellBarModule::GitChanges => data.git_changes.is_some(),
                    _ => unreachable!(),
                };
                (!git).then_some(BarUnavailableReason::NoGit)
            }
            ShellBarModule::AgentState => data
                .agent_state
                .is_none()
                .then_some(BarUnavailableReason::NoAgent),
            ShellBarModule::AgentWaiting => {
                let Some(agent) = data.active_agent.as_ref() else {
                    return Some(BarUnavailableReason::NoAgent);
                };
                (agent.state != AgentViewState::Waiting)
                    .then_some(BarUnavailableReason::AgentNotWaiting)
            }
            ShellBarModule::AgentModel => {
                let Some(agent) = data.active_agent.as_ref() else {
                    return Some(BarUnavailableReason::NoAgent);
                };
                agent
                    .model
                    .is_none()
                    .then_some(BarUnavailableReason::AgentModelUnavailable)
            }
            ShellBarModule::AgentChildren => {
                let Some(agent) = data.active_agent.as_ref() else {
                    return Some(BarUnavailableReason::NoAgent);
                };
                (agent.active_children == 0).then_some(BarUnavailableReason::NoActiveChildren)
            }
            ShellBarModule::AgentStateDuration => data
                .active_agent
                .is_none()
                .then_some(BarUnavailableReason::NoAgent),
            ShellBarModule::Ssh => data.ssh.is_none().then_some(BarUnavailableReason::NoSsh),
            ShellBarModule::Update => data.update.empty.then_some(BarUnavailableReason::NoUpdate),
            ShellBarModule::ProjectsCount
            | ShellBarModule::TerminalsCount
            | ShellBarModule::TabsCount
            | ShellBarModule::EditorsCount
            | ShellBarModule::AppCpu
            | ShellBarModule::AppMemory => data
                .performance
                .as_ref()
                .and_then(|performance| performance.application.as_ref())
                .is_none()
                .then_some(BarUnavailableReason::PerformanceUnavailable),
            ShellBarModule::SystemCpu | ShellBarModule::SystemMemory => data
                .performance
                .as_ref()
                .and_then(|performance| performance.system.as_ref())
                .is_none()
                .then_some(BarUnavailableReason::PerformanceUnavailable),
            ShellBarModule::ProjectName
            | ShellBarModule::Surface
            | ShellBarModule::CommandPalette
            | ShellBarModule::Settings
            | ShellBarModule::Space(_)
            | ShellBarModule::Text(_)
            | ShellBarModule::Icon(_)
            | ShellBarModule::Separator
            | ShellBarModule::Unknown(_) => None,
        }
    }
}

impl BarSectionEntry {
    fn is_content(&self) -> bool {
        matches!(self, Self::Content(_))
    }

    fn is_space(&self) -> bool {
        matches!(self, Self::Space(_))
    }

    fn into_element(self) -> AnyElement {
        match self {
            Self::Content(element) | Self::Space(element) | Self::Separator(element) => element,
        }
    }
}

fn normalized_bar_section(entries: Vec<BarSectionEntry>, ui_style: UiStyle) -> Vec<AnyElement> {
    if !entries.iter().any(BarSectionEntry::is_content) {
        return Vec::new();
    }

    let mut content_after = vec![false; entries.len()];
    let mut has_content_after = false;
    for (index, entry) in entries.iter().enumerate().rev() {
        content_after[index] = has_content_after;
        has_content_after |= entry.is_content();
    }

    let mut retained = Vec::with_capacity(entries.len());
    let mut has_content_before = false;
    let mut separator_pending = false;
    for (index, entry) in entries.into_iter().enumerate() {
        match entry {
            BarSectionEntry::Content(element) => {
                has_content_before = true;
                separator_pending = false;
                retained.push(BarSectionEntry::Content(element));
            }
            BarSectionEntry::Space(element) => retained.push(BarSectionEntry::Space(element)),
            BarSectionEntry::Separator(element)
                if has_content_before && content_after[index] && !separator_pending =>
            {
                separator_pending = true;
                retained.push(BarSectionEntry::Separator(element));
            }
            BarSectionEntry::Separator(_) => {}
        }
    }

    let mut elements = Vec::with_capacity(retained.len().saturating_mul(2));
    let mut previous_was_space = true;
    for entry in retained {
        let is_space = entry.is_space();
        if !elements.is_empty() && !previous_was_space && !is_space {
            elements.push(
                div()
                    .flex_none()
                    .w(ui_style.shell.bar_module_gap)
                    .into_any_element(),
            );
        }
        elements.push(entry.into_element());
        previous_was_space = is_space;
    }
    elements
}

fn bar_space_advance(window: &mut Window, font_family: &str) -> f32 {
    let font = Font {
        family: font_family.to_string().into(),
        features: FontFeatures::disable_ligatures(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    let run = TextRun {
        len: " ".len(),
        font,
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let font_size = window.rem_size() * 0.875;
    window
        .text_system()
        .shape_line(" ".into(), font_size, &[run], None)
        .width
        .as_f32()
}

fn render_bar_space(host: BarHost, section: &'static str, index: usize, width: f32) -> AnyElement {
    let host_name = match host {
        BarHost::Window => "window",
        BarHost::Status => "status",
    };
    let selector = format!("{host_name}-bar-space");
    let id = format!("{selector}-{section}-{index}");
    div()
        .id(id)
        .debug_selector(move || selector.clone())
        .flex_none()
        .w(px(width))
        .into_any_element()
}

fn render_bar_separator(
    host: BarHost,
    section: &'static str,
    index: usize,
    theme: WorkbenchTheme,
) -> AnyElement {
    let host_name = match host {
        BarHost::Window => "window",
        BarHost::Status => "status",
    };
    let selector = format!("{host_name}-bar-separator");
    let id = format!("{selector}-{section}-{index}");
    div()
        .id(id)
        .debug_selector(move || selector.clone())
        .flex_none()
        .w(px(1.0))
        .h(gpui::rems(0.75))
        .bg(theme.text_subtle)
        .into_any_element()
}

fn bar_module_view(
    module: &ShellBarModule,
    data: &ShellBarData,
    settings: BarModuleSettings,
) -> Option<BarModuleView> {
    let mut view = match module {
        ShellBarModule::ProjectName => {
            let mut view = BarModuleView::text(data.project_name.clone());
            view.tone = BarTone::Normal;
            view.emphasized = true;
            view
        }
        ShellBarModule::ProjectPath => {
            let path = data.project_path.as_ref()?;
            let mut view = BarModuleView::text(path.clone());
            view.tooltip = Some(path.clone());
            view
        }
        ShellBarModule::ActiveItem => {
            let item = data.active_item.as_ref()?;
            let mut view = BarModuleView::text(item.title.clone());
            view.icon = Some(item.icon.clone());
            view.tooltip = Some(item.tooltip.clone());
            view
        }
        ShellBarModule::Surface => BarModuleView::text(data.surface),
        ShellBarModule::VimMode => {
            let status = data.vim.as_ref()?;
            let mut view = BarModuleView::text(status.mode.label());
            view.tone = vim_mode_tone(status.mode);
            view.highlighted = true;
            view
        }
        ShellBarModule::VimDetail => {
            let status = data.vim.as_ref()?;
            let detail = status
                .detail
                .as_ref()
                .filter(|detail| !detail.eq_ignore_ascii_case(status.mode.label()))?;
            let mut view = BarModuleView::text(detail.clone());
            view.tone = BarTone::Normal;
            view
        }
        ShellBarModule::VimKeys => {
            let status = data.vim.as_ref()?;
            let mut view = BarModuleView::text(status.key_feedback.join(" "));
            view.empty = status.key_feedback.is_empty();
            view.tone = BarTone::Normal;
            view
        }
        ShellBarModule::EditorLanguage => {
            BarModuleView::text(data.editor.as_ref()?.language.clone())
        }
        ShellBarModule::EditorPosition => {
            let editor = data.editor.as_ref()?;
            let (line, character) = (editor.line?, editor.character?);
            let mut view = BarModuleView::text(format!("{line}:{character}"));
            view.tooltip = Some(format!("Line {line}, column {character}"));
            view
        }
        ShellBarModule::EditorDirty => {
            let editor = data.editor.as_ref()?;
            let mut view = BarModuleView::text(if editor.dirty {
                "● modified"
            } else {
                "saved"
            });
            view.empty = !editor.dirty;
            view.tone = if editor.dirty {
                BarTone::Warning
            } else {
                BarTone::Muted
            };
            view
        }
        ShellBarModule::EditorDiagnostics => {
            let editor = data.editor.as_ref()?;
            let (errors, warnings, infos) = editor.diagnostics;
            let mut parts = Vec::new();
            if errors > 0 {
                parts.push(format!("E{errors}"));
            }
            if warnings > 0 {
                parts.push(format!("W{warnings}"));
            }
            if infos > 0 {
                parts.push(format!("I{infos}"));
            }
            let mut view = BarModuleView::text(if parts.is_empty() {
                "no diagnostics".to_string()
            } else {
                parts.join(" ")
            });
            view.empty = parts.is_empty();
            view.tone = if errors > 0 {
                BarTone::Danger
            } else if warnings > 0 {
                BarTone::Warning
            } else {
                BarTone::Muted
            };
            view
        }
        ShellBarModule::EditorSelection => {
            let editor = data.editor.as_ref()?;
            let mut view = match editor.selection {
                Some(selection) => {
                    let label = format!(
                        "{} chars · {} {}",
                        selection.characters,
                        selection.lines,
                        if selection.lines == 1 {
                            "line"
                        } else {
                            "lines"
                        }
                    );
                    let mut view = BarModuleView::text(label);
                    view.tooltip = Some(format!(
                        "{} Unicode scalar characters across {} selected {}",
                        selection.characters,
                        selection.lines,
                        if selection.lines == 1 {
                            "line"
                        } else {
                            "lines"
                        }
                    ));
                    view
                }
                None => BarModuleView::text("no selection"),
            };
            view.empty = editor.selection.is_none();
            view
        }
        ShellBarModule::EditorTabSize => {
            let editor = data.editor.as_ref()?;
            let mut view = BarModuleView::text(format!("tab {}", editor.tab_size));
            view.tooltip = Some(format!("Tab size: {} spaces", editor.tab_size));
            view
        }
        ShellBarModule::EditorWrap => {
            let editor = data.editor.as_ref()?;
            let mut view = BarModuleView::text(if editor.soft_wrap { "wrap" } else { "no wrap" });
            view.tooltip = Some(
                if editor.soft_wrap {
                    "Soft wrapping enabled"
                } else {
                    "Soft wrapping disabled"
                }
                .to_string(),
            );
            view
        }
        ShellBarModule::TerminalTitle => BarModuleView::text(data.terminal.as_ref()?.title.clone()),
        ShellBarModule::TerminalState => {
            let terminal = data.terminal.as_ref()?;
            let mut view = BarModuleView::text(if terminal.running {
                "running"
            } else {
                "stopped"
            });
            view.tone = if terminal.running {
                BarTone::Success
            } else {
                BarTone::Danger
            };
            view
        }
        ShellBarModule::TerminalExit => {
            let terminal = data.terminal.as_ref()?;
            let (code, reason) = terminal.exit.as_ref()?;
            let mut view = BarModuleView::text(match code {
                Some(code) => format!("exit {code} · {}", terminal_exit_reason_label(reason)),
                None => format!("exit · {}", terminal_exit_reason_label(reason)),
            });
            view.tone = terminal_exit_reason_tone(reason);
            view
        }
        ShellBarModule::TerminalSize => {
            let terminal = data.terminal.as_ref()?;
            let mut view = match terminal.viewport_size {
                Some((cols, rows)) => BarModuleView::text(format!("{cols}×{rows}")),
                None => BarModuleView::text("—"),
            };
            view.tooltip = terminal
                .viewport_size
                .map(|(cols, rows)| format!("{cols} columns × {rows} rows"));
            view.empty = terminal.viewport_size.is_none();
            view
        }
        ShellBarModule::GitBranch => {
            let branch = data.git_branch.as_ref()?;
            let mut view = BarModuleView::text(format!("⎇ {branch}"));
            view.icon = Some(IconName::Network);
            view.tooltip = Some(branch.clone());
            view.action = Some(CommandId::GitBranchSwitch);
            view
        }
        ShellBarModule::GitChanges => {
            let (changes, clean) = data.git_changes.as_ref()?;
            let mut view = BarModuleView::text(changes.clone());
            view.empty = *clean;
            view.action = Some(CommandId::GitDiffOpen);
            view.tone = if *clean {
                BarTone::Muted
            } else {
                BarTone::Normal
            };
            view
        }
        ShellBarModule::AgentState => {
            let state = data.agent_state?;
            let mut view = BarModuleView::text(agent_status_label(state));
            view.icon = Some(IconName::Bot);
            view.tone = agent_state_tone(state);
            view
        }
        ShellBarModule::AgentWaiting => {
            let agent = data.active_agent.as_ref()?;
            if agent.state != AgentViewState::Waiting {
                return None;
            }
            let text = agent
                .waiting_reason
                .map(|reason| format!("waiting · {}", waiting_reason_label(reason)))
                .unwrap_or_else(|| "waiting".to_string());
            let mut view = BarModuleView::text(text);
            view.icon = Some(IconName::Bot);
            view.tooltip = agent.waiting_message.clone();
            view.tone = BarTone::Warning;
            view
        }
        ShellBarModule::AgentModel => {
            let model = data.active_agent.as_ref()?.model.as_ref()?;
            let mut view = BarModuleView::text(format!("model {model}"));
            view.icon = Some(IconName::Bot);
            view.tooltip = Some(format!("Agent model: {model}"));
            view
        }
        ShellBarModule::AgentChildren => {
            let agent = data.active_agent.as_ref()?;
            let mut view = BarModuleView::text(format!("children {}", agent.active_children));
            view.icon = Some(IconName::Bot);
            view.tooltip = Some(format!("{} active child agents", agent.active_children));
            view.empty = agent.active_children == 0;
            view
        }
        ShellBarModule::AgentStateDuration => {
            let agent = data.active_agent.as_ref()?;
            let elapsed = current_unix_millis().saturating_sub(agent.state_started_at);
            let mut view = BarModuleView::text(format_state_duration(elapsed));
            view.tooltip = Some("Current selected agent state duration".to_string());
            view.tone = agent_state_tone(agent.state);
            view
        }
        ShellBarModule::Ssh => {
            let ssh = data.ssh.as_ref()?;
            let mut view = BarModuleView::text(ssh.text.clone());
            view.icon = Some(IconName::Globe);
            view.tooltip = Some(ssh.tooltip.clone());
            view.tone = ssh.tone;
            view
        }
        ShellBarModule::Update => {
            let update = &data.update;
            let mut view = BarModuleView::text(update.text.clone());
            view.icon = Some(IconName::Info);
            view.tooltip = Some(update.tooltip.clone());
            view.tone = update.tone;
            view.empty = update.empty;
            view
        }
        ShellBarModule::ProjectsCount => performance_view(
            data.performance
                .as_ref()?
                .application
                .as_ref()?
                .projects
                .clone(),
            IconName::Folder,
        ),
        ShellBarModule::TerminalsCount => performance_view(
            data.performance
                .as_ref()?
                .application
                .as_ref()?
                .terminals
                .clone(),
            IconName::SquareTerminal,
        ),
        ShellBarModule::TabsCount => performance_view(
            data.performance
                .as_ref()?
                .application
                .as_ref()?
                .tabs
                .clone(),
            IconName::GalleryVerticalEnd,
        ),
        ShellBarModule::EditorsCount => performance_view(
            data.performance
                .as_ref()?
                .application
                .as_ref()?
                .editors
                .clone(),
            IconName::File,
        ),
        ShellBarModule::AppCpu => performance_view(
            data.performance.as_ref()?.application.as_ref()?.cpu.clone(),
            IconName::Cpu,
        ),
        ShellBarModule::AppMemory => performance_view(
            data.performance
                .as_ref()?
                .application
                .as_ref()?
                .memory
                .clone(),
            IconName::MemoryStick,
        ),
        ShellBarModule::SystemCpu => performance_view(
            data.performance.as_ref()?.system.as_ref()?.cpu.clone(),
            IconName::Cpu,
        ),
        ShellBarModule::SystemMemory => performance_view(
            data.performance.as_ref()?.system.as_ref()?.memory.clone(),
            IconName::MemoryStick,
        ),
        ShellBarModule::CommandPalette => {
            let mut view = BarModuleView::text("");
            view.icon = Some(IconName::Search);
            view.tooltip = Some("Open command palette".to_string());
            view.action = Some(CommandId::CommandPaletteOpen);
            view
        }
        ShellBarModule::Settings => {
            let mut view = BarModuleView::text("");
            view.icon = Some(IconName::Settings);
            view.tooltip = Some("Open settings".to_string());
            view.action = Some(CommandId::SettingsOpen);
            view
        }
        ShellBarModule::Space(_)
        | ShellBarModule::Text(_)
        | ShellBarModule::Icon(_)
        | ShellBarModule::Separator
        | ShellBarModule::Unknown(_) => return None,
    };

    if view.empty && settings.hide_when_empty {
        return None;
    }
    if view.empty && view.text.as_deref().is_none_or(str::is_empty) {
        view.text = Some("—".to_string());
    }
    Some(view)
}

fn render_bar_module(
    host: BarHost,
    section: &'static str,
    index: usize,
    module: &ShellBarModule,
    settings: BarModuleSettings,
    view: BarModuleView,
    interactive: bool,
    cx: &mut Context<WorkbenchView>,
) -> AnyElement {
    let host_name = match host {
        BarHost::Window => "window",
        BarHost::Status => "status",
    };
    let selector = format!("{host_name}-bar-{}", module.as_str());
    let id = format!("{selector}-{section}-{index}");
    let theme = current_workbench_theme(cx);
    let ui_style = current_ui_style(cx);
    let window_identity_meta = host == BarHost::Window
        && matches!(
            module,
            ShellBarModule::ProjectPath | ShellBarModule::GitBranch | ShellBarModule::GitChanges
        );
    let window_identity_branch =
        host == BarHost::Window && matches!(module, ShellBarModule::GitBranch);
    let window_identity_changes =
        host == BarHost::Window && matches!(module, ShellBarModule::GitChanges);

    if interactive
        && view.text.as_deref().is_none_or(str::is_empty)
        && let (Some(icon), Some(command)) = (view.icon.clone(), view.action)
    {
        let tooltip = view.tooltip.unwrap_or_default();
        let debug_selector = selector.clone();
        return yttt_icon_button(
            id,
            icon,
            YtttIconButtonKind::Toolbar,
            theme,
            ui_style,
            cx.listener(move |this, _, _window, cx| {
                let _ = this.run_command(command);
                cx.notify();
            }),
        )
        .debug_selector(move || debug_selector.clone())
        .when(host == BarHost::Status, |button| {
            button.size(ui_style.controls.button_height)
        })
        .when(host == BarHost::Window, |button| button.occlude())
        .tooltip(move |window, cx| {
            yttt_ui::primitives::tooltip::yttt_tooltip(
                tooltip.clone(),
                crate::ui::theme::current_workbench_theme(cx),
                crate::ui::theme::current_ui_style(cx),
            )
            .build(window, cx)
        })
        .into_any_element();
    }

    let color = tone_color(view.tone, theme);
    let tooltip = view.tooltip;
    let debug_selector = selector.clone();
    let max_width = settings
        .max_width
        .or_else(|| default_module_max_width(host, module));
    let text = view.text.unwrap_or_default();
    let action = view.action;
    let icon = if window_identity_branch {
        None
    } else {
        view.icon
    };
    let icon_path = view.icon_path;
    let mut element = div()
        .id(id)
        .debug_selector(move || debug_selector.clone())
        .flex()
        .flex_none()
        .items_center()
        .min_w_0()
        .when(host == BarHost::Window, |element| element.h_full())
        .when(host == BarHost::Status, |element| {
            element
                .min_h(ui_style.controls.button_height)
                .rounded(ui_style.radius.compact)
        })
        .gap(ui_style.spacing.xs)
        .whitespace_nowrap()
        .text_color(color)
        .when(view.emphasized, |element| element.font_semibold())
        .when(view.highlighted, |element| {
            element.px(ui_style.spacing.xs).bg(color.alpha(0.18))
        })
        .when(window_identity_meta, |element| {
            element.text_xs().text_color(theme.text_muted)
        })
        .when(window_identity_changes, |element| {
            element
                .rounded(ui_style.radius.compact)
                .border(ui_style.border.hairline)
                .border_color(theme.border)
        })
        .when_some(max_width, |element, width| {
            element.max_w(px(width)).overflow_hidden()
        })
        .when_some(icon, |element, icon| {
            element.child(Icon::new(icon).size_3().text_color(color))
        })
        .when_some(icon_path, |element, path| {
            element.child(Icon::default().path(path).size_3().text_color(color))
        })
        .when(!text.is_empty(), |element| {
            element.child(div().min_w_0().truncate().child(text))
        });
    if let Some(command) = action {
        element = element.px(ui_style.spacing.xs);
        if interactive {
            element = element
                .cursor_pointer()
                .when(host == BarHost::Window, |element| element.occlude())
                .hover(move |element| element.bg(ui_style.hover_background(theme)))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    let _ = this.run_command(command);
                    cx.notify();
                }));
        }
    }
    if let Some(tooltip) = tooltip {
        element = element.tooltip(move |window, cx| {
            yttt_ui::primitives::tooltip::yttt_tooltip(
                tooltip.clone(),
                crate::ui::theme::current_workbench_theme(cx),
                crate::ui::theme::current_ui_style(cx),
            )
            .build(window, cx)
        });
    }
    element.into_any_element()
}

fn performance_view(
    metric: super::performance::PerformanceMetricInfo,
    icon: IconName,
) -> BarModuleView {
    let mut view = BarModuleView::text(metric.value);
    view.icon = Some(icon);
    view.tooltip = Some(metric.tooltip);
    view
}

fn default_module_max_width(host: BarHost, module: &ShellBarModule) -> Option<f32> {
    match module {
        ShellBarModule::ProjectPath => Some(if host == BarHost::Window {
            480.0
        } else {
            240.0
        }),
        ShellBarModule::ProjectName => Some(220.0),
        ShellBarModule::ActiveItem => Some(280.0),
        ShellBarModule::VimDetail | ShellBarModule::VimKeys => Some(220.0),
        ShellBarModule::TerminalTitle => Some(240.0),
        ShellBarModule::AgentWaiting | ShellBarModule::AgentModel => Some(180.0),
        ShellBarModule::GitBranch => Some(180.0),
        ShellBarModule::Ssh => Some(220.0),
        ShellBarModule::Update => Some(180.0),
        _ => None,
    }
}

fn tone_color(tone: BarTone, theme: WorkbenchTheme) -> gpui::Rgba {
    match tone {
        BarTone::Muted => theme.text_muted,
        BarTone::Normal => theme.text,
        BarTone::Accent => theme.accent,
        BarTone::Success => theme.success,
        BarTone::Warning => theme.warning,
        BarTone::Danger => theme.danger,
    }
}

fn vim_mode_tone(mode: WorkbenchVimMode) -> BarTone {
    match mode {
        WorkbenchVimMode::Normal => BarTone::Accent,
        WorkbenchVimMode::Insert => BarTone::Success,
        WorkbenchVimMode::Visual | WorkbenchVimMode::VisualLine => BarTone::Warning,
        WorkbenchVimMode::Terminal => BarTone::Danger,
    }
}

fn agent_state_tone(state: AgentViewState) -> BarTone {
    match state {
        AgentViewState::Starting | AgentViewState::Working | AgentViewState::Waiting => {
            BarTone::Warning
        }
        AgentViewState::Completed => BarTone::Success,
        AgentViewState::Failed | AgentViewState::Interrupted => BarTone::Danger,
        AgentViewState::Idle | AgentViewState::Stale => BarTone::Muted,
    }
}

fn waiting_reason_label(reason: WaitingReason) -> &'static str {
    match reason {
        WaitingReason::Approval => "approval",
        WaitingReason::UserInput => "input",
        WaitingReason::External => "external",
    }
}

fn terminal_exit_reason_label(reason: &yttt_terminal::ExitReason) -> &'static str {
    match reason {
        yttt_terminal::ExitReason::Completed => "completed",
        yttt_terminal::ExitReason::Failed => "failed",
        yttt_terminal::ExitReason::KilledByUser => "killed",
    }
}

fn terminal_exit_reason_tone(reason: &yttt_terminal::ExitReason) -> BarTone {
    match reason {
        yttt_terminal::ExitReason::Completed => BarTone::Success,
        yttt_terminal::ExitReason::Failed => BarTone::Danger,
        yttt_terminal::ExitReason::KilledByUser => BarTone::Warning,
    }
}

fn current_unix_millis() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

fn format_state_duration(elapsed_millis: u64) -> String {
    let seconds = elapsed_millis / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m {}s", seconds % 60);
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h {}m", minutes % 60);
    }
    format!("{}d {}h", hours / 24, hours % 24)
}

fn ssh_state_label(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::VerifyingHostKey => "verifying",
        ConnectionState::Authenticating => "authenticating",
        ConnectionState::Connected => "connected",
        ConnectionState::Reconnecting => "reconnecting",
        ConnectionState::Failed => "failed",
    }
}

fn ssh_state_tone(state: ConnectionState) -> BarTone {
    match state {
        ConnectionState::Connected => BarTone::Success,
        ConnectionState::Connecting
        | ConnectionState::VerifyingHostKey
        | ConnectionState::Authenticating
        | ConnectionState::Reconnecting => BarTone::Warning,
        ConnectionState::Failed => BarTone::Danger,
        ConnectionState::Disconnected => BarTone::Muted,
    }
}

fn update_bar_info(status: &UpdateStatus, text: &crate::ui::i18n::UiText) -> UpdateBarInfo {
    use crate::ui::i18n::UiTextKey;

    match status {
        UpdateStatus::Idle => UpdateBarInfo {
            text: format!("v{}", crate::runtime::update::APP_VERSION),
            tooltip: "Update status has not been checked".to_string(),
            tone: BarTone::Muted,
            empty: true,
        },
        UpdateStatus::Checking => UpdateBarInfo {
            text: text.get(UiTextKey::SettingsCheckingForUpdates).to_string(),
            tooltip: text.get(UiTextKey::SettingsCheckingForUpdates).to_string(),
            tone: BarTone::Muted,
            empty: false,
        },
        UpdateStatus::UpToDate => UpdateBarInfo {
            text: text.get(UiTextKey::SettingsUpToDate).to_string(),
            tooltip: format!("v{}", crate::runtime::update::APP_VERSION),
            tone: BarTone::Success,
            empty: true,
        },
        UpdateStatus::Available(update) => UpdateBarInfo {
            text: format!("v{} available", update.version),
            tooltip: update.release_url.clone(),
            tone: BarTone::Accent,
            empty: false,
        },
        UpdateStatus::Failed(message) => UpdateBarInfo {
            text: text.get(UiTextKey::SettingsUpdateCheckFailed).to_string(),
            tooltip: message.clone(),
            tone: BarTone::Danger,
            empty: false,
        },
    }
}

fn git_changes_label(summary: &crate::runtime::git_status::GitStatusSummary) -> (String, bool) {
    if summary.is_clean() {
        return ("clean".to_string(), true);
    }
    let mut parts = Vec::with_capacity(4);
    if summary.added > 0 {
        parts.push(format!("+{}", summary.added));
    }
    if summary.modified > 0 {
        parts.push(format!("~{}", summary.modified));
    }
    if summary.deleted > 0 {
        parts.push(format!("-{}", summary.deleted));
    }
    if summary.untracked > 0 {
        parts.push(format!("?{}", summary.untracked));
    }
    (parts.join(" "), false)
}
