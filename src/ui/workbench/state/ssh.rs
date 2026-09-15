use std::collections::{HashMap, VecDeque};

use gpui::{
    App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IndexPath,
    input::InputState,
    list::{ListDelegate, ListItem, ListState},
};
use yttt_core::model::{
    ids::{ConnectionId, CredentialId},
    project::RemotePathBuf,
};
pub(in super::super) use yttt_protocol::ssh::SshConnectionState as ConnectionState;

use crate::{
    config::{
        paths::AppConfigPaths,
        ssh::{SshAuthPreference, SshConnectionConfig, SshConnectionsConfig, load_ssh_connections},
    },
    ui::theme::UiStyle,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) enum SshProjectConnectContinuation {
    Browse { initial_root: Option<RemotePathBuf> },
    OpenRecent { root: RemotePathBuf },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum SshProjectPickerView {
    #[default]
    Connections,
    QuickConnect,
    Password,
    Connecting,
    Opening,
    Browsing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct SshProjectDirectory {
    pub(in super::super) name: String,
    pub(in super::super) path: RemotePathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct ConnectionStatus {
    pub(in super::super) connection_id: ConnectionId,
    pub(in super::super) epoch: u64,
    pub(in super::super) state: ConnectionState,
    pub(in super::super) error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct HostKeyChallenge {
    pub(in super::super) challenge_id: u64,
    pub(in super::super) connection_id: ConnectionId,
    pub(in super::super) epoch: u64,
    pub(in super::super) host: String,
    pub(in super::super) port: u16,
    pub(in super::super) algorithm: String,
    pub(in super::super) fingerprint: String,
    pub(in super::super) previous_fingerprint: Option<String>,
}

#[derive(Default)]
pub(in super::super) struct SshProjectPickerState {
    pub(in super::super) open: bool,
    pub(in super::super) view: SshProjectPickerView,
    pub(in super::super) connection_id: Option<ConnectionId>,
    pub(in super::super) continuation: Option<SshProjectConnectContinuation>,
    pub(in super::super) current_path: Option<RemotePathBuf>,
    pub(in super::super) home_path: Option<RemotePathBuf>,
    pub(in super::super) selected_directory: usize,
    pub(in super::super) directory_scroll: gpui::ScrollHandle,
    pub(in super::super) directories: Vec<SshProjectDirectory>,
    pub(in super::super) directory_prefix: String,
    pub(in super::super) preserve_path_input: bool,
    pub(in super::super) loading: bool,
    pub(in super::super) generation: u64,
    pub(in super::super) connection_generation: u64,
    pub(in super::super) connection_epoch: Option<u64>,
    pub(in super::super) error: Option<String>,
    pub(in super::super) path_input: Option<Entity<InputState>>,
    pub(in super::super) path_input_subscription: Option<Subscription>,
    pub(in super::super) path_input_needs_sync: bool,
    pub(in super::super) password_input: Option<Entity<InputState>>,
    pub(in super::super) password_input_subscription: Option<Subscription>,
    pub(in super::super) password_input_needs_focus: bool,
    pub(in super::super) remember_password: bool,
    pub(in super::super) connection_list: Option<Entity<ListState<SshConnectionListDelegate>>>,
    pub(in super::super) connection_list_subscription: Option<Subscription>,
}

impl SshProjectPickerState {
    pub(in super::super) fn filtered_directories(
        &self,
    ) -> impl Iterator<Item = &SshProjectDirectory> {
        self.directories
            .iter()
            .filter(|directory| directory.name.starts_with(&self.directory_prefix))
    }

    pub(in super::super) fn shows_parent(&self) -> bool {
        self.directory_prefix.is_empty()
            && self
                .current_path
                .as_ref()
                .is_some_and(|path| path.as_str() != "/")
    }

    pub(in super::super) fn reset_directory_selection(&mut self) {
        self.selected_directory = usize::from(
            !self.directory_prefix.is_empty() && self.filtered_directories().next().is_some(),
        );
        self.directory_scroll
            .set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
    }

    pub(in super::super) fn reset(&mut self) {
        let generation = self.generation.wrapping_add(1);
        let connection_generation = self.connection_generation.wrapping_add(1);
        *self = Self {
            generation,
            connection_generation,
            ..Self::default()
        };
    }
}
pub(in super::super) struct SshControllerState {
    pub(in super::super) connections: SshConnectionsConfig,
    pub(in super::super) statuses: HashMap<ConnectionId, ConnectionStatus>,
    pub(in super::super) manager_open: bool,
    pub(in super::super) editor_open: bool,
    pub(in super::super) connecting: Option<ConnectionId>,
    pub(in super::super) credentials_only: bool,
    pub(in super::super) remote_access: Option<yttt_protocol::remote_access::RemoteAccessStatus>,
    pub(in super::super) remote_access_busy: bool,
    pub(in super::super) remote_access_loaded: bool,
    pub(in super::super) remote_access_error: Option<String>,
    pub(in super::super) remote_access_address: Option<Entity<InputState>>,
    pub(in super::super) form: Option<SshConnectionForm>,
    pub(in super::super) project_picker: SshProjectPickerState,
    pub(in super::super) pending_host_keys: VecDeque<HostKeyChallenge>,
    pub(in super::super) event_task: Option<Task<()>>,
    pub(in super::super) error: Option<String>,
}

impl SshControllerState {
    pub(in super::super) fn new(paths: &AppConfigPaths) -> (Self, Option<String>) {
        let (connections, config_error) = if crate::config::storage::is_remote() {
            (SshConnectionsConfig::default(), None)
        } else {
            match load_ssh_connections(paths) {
                Ok(config) => (config, None),
                Err(error) => (SshConnectionsConfig::default(), Some(error.to_string())),
            }
        };
        let load_error = config_error;
        (
            Self {
                connections,
                statuses: HashMap::new(),
                manager_open: false,
                editor_open: false,
                connecting: None,
                credentials_only: false,
                remote_access: None,
                remote_access_busy: false,
                remote_access_loaded: false,
                remote_access_error: None,
                remote_access_address: None,
                form: None,
                project_picker: SshProjectPickerState::default(),
                pending_host_keys: VecDeque::new(),
                event_task: None,
                error: load_error.clone(),
            },
            load_error,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum SshConnectionFormMode {
    Auto,
    Agent,
    Password,
    PrivateKey,
}
impl From<SshAuthPreference> for SshConnectionFormMode {
    fn from(value: SshAuthPreference) -> Self {
        match value {
            SshAuthPreference::Auto => Self::Auto,
            SshAuthPreference::Agent => Self::Agent,
            SshAuthPreference::Password => Self::Password,
            SshAuthPreference::PublicKey => Self::PrivateKey,
        }
    }
}

impl From<SshConnectionFormMode> for SshAuthPreference {
    fn from(value: SshConnectionFormMode) -> Self {
        match value {
            SshConnectionFormMode::Auto => Self::Auto,
            SshConnectionFormMode::Agent => Self::Agent,
            SshConnectionFormMode::Password => Self::Password,
            SshConnectionFormMode::PrivateKey => Self::PublicKey,
        }
    }
}

pub(in super::super) struct SshConnectionForm {
    pub(in super::super) connection_id: ConnectionId,
    pub(in super::super) credential_id: CredentialId,
    pub(in super::super) auth: SshConnectionFormMode,
    pub(in super::super) remember_password: bool,
    pub(in super::super) inputs: Option<SshConnectionFormInputs>,
    pub(in super::super) command_subscription: Option<Subscription>,
    pub(in super::super) initial: SshConnectionConfig,
}

impl SshConnectionForm {
    pub(in super::super) fn new(connection: SshConnectionConfig) -> Self {
        Self {
            connection_id: connection.id.clone(),
            credential_id: connection
                .credential
                .as_ref()
                .map(|credential| credential.id.clone())
                .unwrap_or_else(CredentialId::random),
            auth: connection.auth.into(),
            remember_password: true,
            inputs: None,
            command_subscription: None,
            initial: connection,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct SshConnectionFormInputs {
    pub(in super::super) command: Entity<InputState>,
    pub(in super::super) name: Entity<InputState>,
    pub(in super::super) host: Entity<InputState>,
    pub(in super::super) port: Entity<InputState>,
    pub(in super::super) user: Entity<InputState>,
    pub(in super::super) remote_root: Entity<InputState>,
    pub(in super::super) identity_file: Entity<InputState>,
    pub(in super::super) key_passphrase: Entity<InputState>,
    pub(in super::super) password: Entity<InputState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) enum SshConnectionListAction {
    New,
    Edit(ConnectionId),
    Open(ConnectionId),
    OpenRecent {
        connection_id: ConnectionId,
        root: RemotePathBuf,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum SshConnectionListTone {
    #[default]
    Neutral,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct SshConnectionListEntry {
    pub(in super::super) action: SshConnectionListAction,
    pub(in super::super) title: SharedString,
    pub(in super::super) subtitle: SharedString,
    pub(in super::super) status: SharedString,
    pub(in super::super) tone: SshConnectionListTone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct SshConnectionListSection {
    pub(in super::super) title: SharedString,
    pub(in super::super) entries: Vec<SshConnectionListEntry>,
}

pub(in super::super) struct SshConnectionListDelegate {
    sections: Option<Vec<SshConnectionListSection>>,
    all_sections: Vec<SshConnectionListSection>,
    query: String,
    selected_index: Option<IndexPath>,
    empty_message: SharedString,
    ui_style: UiStyle,
}

impl SshConnectionListDelegate {
    pub(in super::super) fn new(
        sections: Vec<SshConnectionListSection>,
        empty_message: impl Into<SharedString>,
        ui_style: UiStyle,
    ) -> Self {
        Self {
            all_sections: sections,
            query: String::new(),
            sections: None,
            selected_index: None,
            empty_message: empty_message.into(),
            ui_style,
        }
    }

    pub(in super::super) fn replace_sections(
        &mut self,
        sections: Vec<SshConnectionListSection>,
        ui_style: UiStyle,
    ) {
        if self.all_sections == sections && self.ui_style == ui_style {
            return;
        }
        let selected_action = self
            .selected_index
            .and_then(|index| self.action(index).cloned());
        self.all_sections = sections;
        self.filter_sections();
        self.ui_style = ui_style;
        self.selected_index = selected_action
            .as_ref()
            .and_then(|action| self.index_of(action));
    }

    pub(in super::super) fn action(&self, index: IndexPath) -> Option<&SshConnectionListAction> {
        self.entry(index).map(|entry| &entry.action)
    }

    pub(in super::super) fn index_of(&self, action: &SshConnectionListAction) -> Option<IndexPath> {
        self.visible_sections()
            .iter()
            .enumerate()
            .find_map(|(section, entries)| {
                entries
                    .entries
                    .iter()
                    .position(|entry| &entry.action == action)
                    .map(|row| IndexPath::new(row).section(section))
            })
    }

    fn visible_sections(&self) -> &[SshConnectionListSection] {
        self.sections.as_deref().unwrap_or(&self.all_sections)
    }

    fn filter_sections(&mut self) {
        if self.query.is_empty() {
            self.sections = None;
            return;
        }
        self.sections = Some(
            self.all_sections
                .iter()
                .filter_map(|section| {
                    let matches_section = section.title.to_lowercase().contains(&self.query);
                    let entries = section
                        .entries
                        .iter()
                        .filter(|entry| {
                            matches_section
                                || entry.title.to_lowercase().contains(&self.query)
                                || entry.subtitle.to_lowercase().contains(&self.query)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    (!entries.is_empty()).then(|| SshConnectionListSection {
                        title: section.title.clone(),
                        entries,
                    })
                })
                .collect(),
        );
    }

    fn entry(&self, index: IndexPath) -> Option<&SshConnectionListEntry> {
        self.visible_sections()
            .get(index.section)
            .and_then(|section| section.entries.get(index.row))
    }
}

impl ListDelegate for SshConnectionListDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.query = query.trim().to_lowercase();
        self.filter_sections();
        self.selected_index = None;
        Task::ready(())
    }

    fn sections_count(&self, _: &App) -> usize {
        self.visible_sections().len().max(1)
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.visible_sections()
            .get(section)
            .map(|section| section.entries.len())
            .unwrap_or(0)
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let entry = self.entry(index)?.clone();
        let status_color = match entry.tone {
            SshConnectionListTone::Neutral => cx.theme().muted_foreground,
        };
        let icon = match &entry.action {
            SshConnectionListAction::New => IconName::Plus,
            SshConnectionListAction::OpenRecent { .. } => IconName::FolderClosed,
            SshConnectionListAction::Open(_) => IconName::Plus,
            SshConnectionListAction::Edit(_) => IconName::Settings,
        };
        let ui_style = self.ui_style;

        Some(
            ListItem::new(index)
                .min_h(if entry.subtitle.is_empty() {
                    ui_style.rows.palette_compact_height
                } else {
                    ui_style.rows.palette_height
                })
                .mx(ui_style.palette.list_padding_x)
                .mb(ui_style.palette.list_gap)
                .px(ui_style.rows.palette_padding_x)
                .py(ui_style.spacing.xxs)
                .rounded(ui_style.rows.palette_radius)
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(ui_style.palette.item_content_gap)
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .items_center()
                                .gap(ui_style.palette.item_content_gap)
                                .child(
                                    div()
                                        .flex()
                                        .debug_selector(|| "ssh-connection-list-icon".to_string())
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .w(ui_style.palette.icon_column_width)
                                        .child(
                                            Icon::new(icon)
                                                .size(ui_style.palette.icon_size)
                                                .text_color(cx.theme().muted_foreground),
                                        ),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .flex()
                                        .flex_col()
                                        .gap_0p5()
                                        .child(div().truncate().text_sm().child(entry.title))
                                        .when(!entry.subtitle.is_empty(), |this| {
                                            this.child(
                                                div()
                                                    .truncate()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(entry.subtitle),
                                            )
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(status_color)
                                .child(entry.status),
                        ),
                ),
        )
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        let title = self.visible_sections().get(section)?.title.clone();
        if title.is_empty() {
            return None;
        }
        let ui_style = self.ui_style;
        Some(
            div()
                .w_full()
                .px(ui_style.rows.palette_padding_x)
                .border_t_1()
                .border_color(cx.theme().border)
                .py(ui_style.spacing.sm)
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(title),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .px_4()
            .text_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(self.empty_message.clone())
    }

    fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = index;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtering_keeps_server_context_and_refresh_preserves_selected_action() {
        let connection = ConnectionId::random();
        let open = SshConnectionListEntry {
            action: SshConnectionListAction::Open(connection.clone()),
            title: "Open Project".into(),
            subtitle: "".into(),
            status: "".into(),
            tone: SshConnectionListTone::Neutral,
        };
        let edit = SshConnectionListEntry {
            action: SshConnectionListAction::Edit(connection),
            title: "Edit connection".into(),
            ..open.clone()
        };
        let mut delegate = SshConnectionListDelegate::new(
            vec![SshConnectionListSection {
                title: "Production (host)".into(),
                entries: vec![open.clone(), edit.clone()],
            }],
            "No results",
            UiStyle::default(),
        );
        delegate.query = "production".into();
        delegate.filter_sections();
        assert_eq!(delegate.action(IndexPath::new(1)), Some(&edit.action));
        delegate.query = "edit".into();
        delegate.filter_sections();
        assert_eq!(delegate.action(IndexPath::new(0)), Some(&edit.action));
        assert_eq!(delegate.action(IndexPath::new(1)), None);
        delegate.query.clear();
        delegate.filter_sections();
        delegate.selected_index = Some(IndexPath::new(1));
        delegate.replace_sections(
            vec![SshConnectionListSection {
                title: "Production (host)".into(),
                entries: vec![edit.clone(), open],
            }],
            UiStyle::default(),
        );
        assert_eq!(delegate.selected_index, Some(IndexPath::new(0)));
        assert_eq!(
            delegate.action(delegate.selected_index.unwrap()),
            Some(&edit.action)
        );
    }
}
