use std::collections::{HashMap, VecDeque};

use gpui::{
    App, Context, Entity, FontWeight, IntoElement, ParentElement as _, SharedString, Styled as _,
    Subscription, Task, Window, div,
};
use gpui_component::{
    ActiveTheme as _, IndexPath,
    input::InputState,
    list::{ListDelegate, ListItem, ListState},
};
use yttt_core::model::{
    ids::{ConnectionId, CredentialId},
    project::RemotePathBuf,
};
use yttt_ssh::{
    ConnectionEpoch, ConnectionStatus, CredentialStore, HostKeyChallenge, TransportService,
};

use crate::config::{
    paths::AppConfigPaths,
    ssh::{SshAuthPreference, SshConnectionConfig, SshConnectionsConfig, load_ssh_connections},
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

#[derive(Default)]
pub(in super::super) struct SshProjectPickerState {
    pub(in super::super) open: bool,
    pub(in super::super) view: SshProjectPickerView,
    pub(in super::super) connection_id: Option<ConnectionId>,
    pub(in super::super) continuation: Option<SshProjectConnectContinuation>,
    pub(in super::super) current_path: Option<RemotePathBuf>,
    pub(in super::super) directories: Vec<SshProjectDirectory>,
    pub(in super::super) loading: bool,
    pub(in super::super) generation: u64,
    pub(in super::super) connection_generation: u64,
    pub(in super::super) connection_epoch: Option<ConnectionEpoch>,
    pub(in super::super) error: Option<String>,
    pub(in super::super) path_input: Option<Entity<InputState>>,
    pub(in super::super) path_input_subscription: Option<Subscription>,
    pub(in super::super) password_input: Option<Entity<InputState>>,
    pub(in super::super) password_input_subscription: Option<Subscription>,
    pub(in super::super) password_input_needs_focus: bool,
    pub(in super::super) remember_password: bool,
    pub(in super::super) connection_list: Option<Entity<ListState<SshConnectionListDelegate>>>,
    pub(in super::super) connection_list_subscription: Option<Subscription>,
}

impl SshProjectPickerState {
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
    pub(in super::super) transport: Option<TransportService>,
    pub(in super::super) credential_store: CredentialStore,
    pub(in super::super) statuses: HashMap<ConnectionId, ConnectionStatus>,
    pub(in super::super) manager_open: bool,
    pub(in super::super) form: Option<SshConnectionForm>,
    pub(in super::super) project_picker: SshProjectPickerState,
    pub(in super::super) pending_host_keys: VecDeque<HostKeyChallenge>,
    pub(in super::super) event_task: Option<Task<()>>,
    pub(in super::super) error: Option<String>,
    pub(in super::super) manager_connection_list:
        Option<Entity<ListState<SshConnectionListDelegate>>>,
    pub(in super::super) manager_connection_list_subscription: Option<Subscription>,
}

impl SshControllerState {
    pub(in super::super) fn new(paths: &AppConfigPaths) -> (Self, Option<String>) {
        let (connections, config_error) = match load_ssh_connections(paths) {
            Ok(config) => (config, None),
            Err(error) => (SshConnectionsConfig::default(), Some(error.to_string())),
        };
        let (transport, transport_error) = match TransportService::start(paths.ssh_host_keys_file())
        {
            Ok(service) => (Some(service), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let load_error = config_error.or(transport_error);
        (
            Self {
                connections,
                transport,
                credential_store: CredentialStore,
                statuses: HashMap::new(),
                manager_open: false,
                form: None,
                project_picker: SshProjectPickerState::default(),
                pending_host_keys: VecDeque::new(),
                event_task: None,
                error: load_error.clone(),
                manager_connection_list: None,
                manager_connection_list_subscription: None,
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
    Success,
    Warning,
    Danger,
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
    sections: Vec<SshConnectionListSection>,
    selected_index: Option<IndexPath>,
    empty_message: SharedString,
}

impl SshConnectionListDelegate {
    pub(in super::super) fn new(
        sections: Vec<SshConnectionListSection>,
        empty_message: impl Into<SharedString>,
    ) -> Self {
        Self {
            sections,
            selected_index: None,
            empty_message: empty_message.into(),
        }
    }

    pub(in super::super) fn replace_sections(&mut self, sections: Vec<SshConnectionListSection>) {
        self.sections = sections;
        if self
            .selected_index
            .is_some_and(|index| self.entry(index).is_none())
        {
            self.selected_index = None;
        }
    }

    pub(in super::super) fn action(&self, index: IndexPath) -> Option<&SshConnectionListAction> {
        self.entry(index).map(|entry| &entry.action)
    }

    pub(in super::super) fn index_of(&self, action: &SshConnectionListAction) -> Option<IndexPath> {
        self.sections
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

    fn entry(&self, index: IndexPath) -> Option<&SshConnectionListEntry> {
        self.sections
            .get(index.section)
            .and_then(|section| section.entries.get(index.row))
    }
}

impl ListDelegate for SshConnectionListDelegate {
    type Item = ListItem;

    fn sections_count(&self, _: &App) -> usize {
        self.sections.len().max(1)
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.sections
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
            SshConnectionListTone::Success => cx.theme().success,
            SshConnectionListTone::Warning => cx.theme().warning,
            SshConnectionListTone::Danger => cx.theme().danger,
        };
        Some(
            ListItem::new(index).child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_2()
                    .py_2()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(entry.title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(entry.subtitle),
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
        let title = self.sections.get(section)?.title.clone();
        Some(
            div()
                .w_full()
                .px_2()
                .py_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
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
