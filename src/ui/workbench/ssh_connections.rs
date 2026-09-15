use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
};

use base64::Engine as _;
use gpui_component::radio::RadioGroup;
use yttt_client_core::ClientEvent;
use yttt_core::model::{
    ids::{ConnectionId, CredentialId},
    project::RemotePathBuf,
};
use yttt_protocol::{
    Request, Response, ServerEvent,
    ssh::{CredentialAnswer, CredentialChallengeKind, HostKeyDecision},
};

use crate::config::ssh::{
    CredentialBinding, CredentialKind, CredentialRef, SshAuthPreference, SshConnectionConfig,
    save_ssh_connections,
};
use crate::config::ssh_command::{format_ssh_command, parse_ssh_command};

use super::*;

pub(super) async fn request_host(
    runtime: Option<Arc<crate::host_runtime::DesktopHostRuntime>>,
    request: Request,
) -> Result<Response, String> {
    let runtime = runtime.ok_or_else(|| "Host runtime is unavailable".to_string())?;
    runtime
        .request(request)
        .recv_async()
        .await
        .map_err(|_| "Host request channel closed".to_string())?
        .map_err(|error| error.to_string())
}

pub(super) async fn disconnect_host_ssh(
    runtime: Option<Arc<crate::host_runtime::DesktopHostRuntime>>,
    connection_id: ConnectionId,
) -> Result<(), String> {
    match request_host(
        runtime,
        Request::SshDisconnect {
            connection_id: connection_id.as_str().to_string(),
        },
    )
    .await?
    {
        Response::SshDisconnected => Ok(()),
        response => Err(format!(
            "Host returned an unexpected SSH disconnect response: {response:?}"
        )),
    }
}

async fn delete_host_credential(
    runtime: Option<Arc<crate::host_runtime::DesktopHostRuntime>>,
    credential_id: CredentialId,
) -> Result<(), String> {
    match request_host(
        runtime,
        Request::DeleteSshCredential {
            credential_id: credential_id.to_string(),
        },
    )
    .await?
    {
        Response::CredentialDeleted => Ok(()),
        response => Err(format!(
            "Host returned an unexpected credential response: {response:?}"
        )),
    }
}

fn is_ssh_host_event(event: &ServerEvent) -> bool {
    matches!(
        event,
        ServerEvent::SshStateChanged(_)
            | ServerEvent::CredentialChallenge(_)
            | ServerEvent::SshCredentialSaved { .. }
    )
}

impl WorkbenchView {
    pub fn start_ssh_event_listener(&mut self, cx: &mut Context<Self>) {
        if self.ssh.event_task.is_some() {
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.as_ref() else {
            return;
        };
        let events = runtime.events();
        self.ssh.event_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv_async().await {
                let ClientEvent::Server(event) = event else {
                    continue;
                };
                if !is_ssh_host_event(&event.body) {
                    continue;
                }
                if this
                    .update(cx, |root, cx| {
                        root.apply_ssh_host_event(event.body, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn apply_ssh_host_event(&mut self, event: ServerEvent, cx: &mut Context<Self>) {
        match event {
            ServerEvent::SshStateChanged(status) => {
                let status = ConnectionStatus {
                    connection_id: ConnectionId::new(status.connection_id),
                    epoch: status.epoch,
                    state: status.state,
                    error: status.error,
                };
                if self
                    .ssh
                    .statuses
                    .get(&status.connection_id)
                    .is_some_and(|current| current.epoch > status.epoch)
                {
                    return;
                }
                let connected_id = (status.state == ConnectionState::Connected)
                    .then(|| status.connection_id.clone());
                if status.state == ConnectionState::Failed {
                    self.ssh.error = status.error.clone();
                }
                self.ssh
                    .statuses
                    .insert(status.connection_id.clone(), status.clone());
                self.reject_stale_ssh_host_key_challenges(&status.connection_id, status.epoch);
                self.apply_ssh_project_connection_status(&status, cx);
                if let Some(connection_id) = connected_id {
                    let project_ids = self
                        .workspace
                        .opened_projects()
                        .iter()
                        .filter_map(|project| match &project.location {
                            ProjectLocation::Ssh {
                                connection_id: project_connection_id,
                                ..
                            } if project_connection_id == &connection_id => {
                                Some(project.id.clone())
                            }
                            ProjectLocation::Local { .. } | ProjectLocation::Ssh { .. } => None,
                        })
                        .collect::<Vec<_>>();
                    for project_id in project_ids {
                        self.queue_project_tree_refresh(project_id.clone());
                        self.refresh_project_git_status(project_id, cx);
                    }
                }
            }
            ServerEvent::CredentialChallenge(challenge) => {
                let connection_id = ConnectionId::new(challenge.connection_id);
                let epoch = self
                    .ssh
                    .statuses
                    .get(&connection_id)
                    .map_or(0, |status| status.epoch);
                let CredentialChallengeKind::HostKey {
                    host,
                    port,
                    algorithm,
                    fingerprint,
                    previous_fingerprint,
                } = challenge.kind
                else {
                    return;
                };
                let challenge = HostKeyChallenge {
                    challenge_id: challenge.challenge_id,
                    connection_id,
                    epoch,
                    host,
                    port,
                    algorithm,
                    fingerprint,
                    previous_fingerprint,
                };
                let is_current = self
                    .ssh
                    .statuses
                    .get(&challenge.connection_id)
                    .is_some_and(|status| status.epoch == challenge.epoch)
                    && self
                        .ssh
                        .connections
                        .connections
                        .iter()
                        .any(|connection| connection.id == challenge.connection_id);
                if is_current {
                    self.ssh.pending_host_keys.push_back(challenge);
                } else {
                    self.send_ssh_host_key_answer(challenge, false, false);
                }
            }
            ServerEvent::SshCredentialSaved {
                connection_id,
                epoch,
                credential,
            } => {
                let connection_id = ConnectionId::new(connection_id);
                let credential_id = CredentialId::new(credential.id);
                let current_epoch_matches = self
                    .ssh
                    .statuses
                    .get(&connection_id)
                    .is_some_and(|status| status.epoch == epoch);
                if !current_epoch_matches {
                    let runtime = self.terminal.host_runtime.clone();
                    cx.background_spawn(async move {
                        let _ = delete_host_credential(runtime, credential_id).await;
                    })
                    .detach();
                    return;
                }
                let mut updated = self.ssh.connections.clone();
                if let Some(connection) = updated
                    .connections
                    .iter_mut()
                    .find(|connection| connection.id == connection_id)
                {
                    let previous = connection
                        .credential
                        .as_ref()
                        .map(|item| item.binding.clone());
                    connection.credential = Some(CredentialRef {
                        id: credential_id.clone(),
                        kind: CredentialKind::LoginPassword,
                        binding: CredentialBinding {
                            connection_id: connection_id.clone(),
                            effective_user: credential.effective_user.clone(),
                            resolved_host: previous
                                .as_ref()
                                .map(|binding| binding.resolved_host.clone())
                                .unwrap_or_else(|| connection.host.clone()),
                            port: previous
                                .as_ref()
                                .map(|binding| binding.port)
                                .unwrap_or(connection.port),
                            host_key_sha256: previous
                                .map(|binding| binding.host_key_sha256)
                                .unwrap_or_default(),
                            private_key_identity: credential.private_key_identity.clone(),
                        },
                    });
                }
                match save_ssh_connections(&self.config_paths, &updated) {
                    Ok(()) => {
                        self.ssh.connections = updated;
                        if let Some(connection) = self
                            .ssh
                            .connections
                            .connections
                            .iter()
                            .find(|connection| connection.id == connection_id)
                            .cloned()
                            && let Some(form) = self.ssh.form.as_mut()
                            && form.connection_id == connection_id
                        {
                            form.initial = connection;
                        }
                    }
                    Err(error) => {
                        self.ssh.error = Some(error.to_string());
                        let runtime = self.terminal.host_runtime.clone();
                        cx.background_spawn(async move {
                            let _ = delete_host_credential(runtime, credential_id).await;
                        })
                        .detach();
                    }
                }
            }
            _ => {}
        }
    }

    pub fn open_ssh_connection_manager(&mut self) {
        if crate::config::storage::is_remote() {
            self.load_error = Some(self.ui_text.get(UiTextKey::RemoteManageLocally).into());
            return;
        }
        self.ssh.manager_open = true;
        self.ssh.editor_open = false;
        self.ssh.credentials_only = false;
        self.ssh.form = None;
        self.auxiliary_windows.remote_page =
            super::auxiliary_windows::RemoteServicesPage::Connections;
        self.auxiliary_windows
            .request(AuxiliaryWindowKind::RemoteServices);
        self.ssh.error = None;
        self.sync_input_owner_state();
    }

    pub fn close_ssh_connection_manager(&mut self) {
        self.ssh.manager_open = false;
        self.close_ssh_connection_editor();
        self.ssh.remote_access_address = None;
        self.auxiliary_windows.existing_host = None;
        self.auxiliary_windows.existing_host_subscription = None;
        self.auxiliary_windows.pending_new_host_editor = false;
        if self.auxiliary_windows.active == Some(AuxiliaryWindowKind::RemoteServices) {
            self.auxiliary_windows.active = None;
        }
        self.sync_input_owner_state();
    }

    pub(super) fn open_ssh_connection_editor(
        &mut self,
        connection_id: Option<ConnectionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if crate::config::storage::is_remote() {
            self.load_error = Some(self.ui_text.get(UiTextKey::RemoteManageLocally).into());
            return;
        }
        let connection = match connection_id {
            Some(connection_id) => self
                .ssh
                .connections
                .connections
                .iter()
                .find(|connection| connection.id == connection_id)
                .cloned(),
            None => {
                let user = std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_default();
                let mut connection = SshConnectionConfig::new("", "", 22, user);
                connection.default_remote_root =
                    Some(RemotePathBuf::new("/").expect("root is a valid remote path"));
                Some(connection)
            }
        };
        let Some(connection) = connection else {
            self.ssh.error = Some(self.ui_text.get(UiTextKey::RemoteRecordMissing).into());
            cx.notify();
            return;
        };
        self.ssh.manager_open = true;
        self.ssh.editor_open = true;
        self.ssh.credentials_only = false;
        self.ssh.form = Some(SshConnectionForm::new(connection));
        self.ssh.error = None;
        if let Some(inputs) = self.ssh_connection_form_inputs(window, cx) {
            inputs
                .command
                .update(cx, |input, cx| input.focus(window, cx));
        }
        self.sync_input_owner_state();
        cx.notify();
    }

    pub(super) fn close_ssh_connection_editor(&mut self) {
        self.ssh.editor_open = false;
        self.ssh.credentials_only = false;
        self.ssh.connecting = None;
        self.ssh.form = None;
        self.ssh.error = None;
    }

    pub fn new_ssh_connection_form(&mut self) {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        let mut connection = SshConnectionConfig::new("", "", 22, user);
        connection.default_remote_root =
            Some(RemotePathBuf::new("/").expect("root is a valid remote path"));
        self.ssh.form = Some(SshConnectionForm::new(connection));
        self.ssh.error = None;
    }

    pub fn edit_ssh_connection(&mut self, connection_id: &ConnectionId) {
        let Some(connection) = self
            .ssh
            .connections
            .connections
            .iter()
            .find(|connection| &connection.id == connection_id)
            .cloned()
        else {
            return;
        };
        self.ssh.form = Some(SshConnectionForm::new(connection));
        self.ssh.error = None;
    }

    pub(super) fn ssh_connection_form_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<SshConnectionFormInputs> {
        let form = self.ssh.form.as_mut()?;
        if let Some(inputs) = form.inputs.as_ref() {
            return Some(inputs.clone());
        }
        let initial = &form.initial;
        let command = ssh_input(
            window,
            cx,
            self.ui_text.get(UiTextKey::SshCommandPlaceholder),
            format_ssh_command(
                &initial.host,
                initial.port,
                &initial.user,
                initial.identity_file.as_deref(),
            ),
            false,
        );
        let command_subscription =
            cx.subscribe_in(&command, window, Self::on_ssh_command_input_event);
        let inputs = SshConnectionFormInputs {
            command,
            name: ssh_input(
                window,
                cx,
                self.ui_text.get(UiTextKey::SshConnectionName),
                initial.name.clone(),
                false,
            ),
            host: ssh_input(
                window,
                cx,
                self.ui_text.get(UiTextKey::SshHost),
                initial.host.clone(),
                false,
            ),
            port: ssh_input(
                window,
                cx,
                self.ui_text.get(UiTextKey::SshPort),
                initial.port.to_string(),
                false,
            ),
            user: ssh_input(
                window,
                cx,
                self.ui_text.get(UiTextKey::SshUser),
                initial.user.clone(),
                false,
            ),
            remote_root: ssh_input(
                window,
                cx,
                self.ui_text.get(UiTextKey::SshRemoteRoot),
                initial
                    .default_remote_root
                    .as_ref()
                    .map(RemotePathBuf::as_str)
                    .unwrap_or("/")
                    .to_string(),
                false,
            ),
            identity_file: ssh_input(
                window,
                cx,
                self.ui_text.get(UiTextKey::SshIdentityFile),
                initial
                    .identity_file
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                false,
            ),
            key_passphrase: ssh_input(
                window,
                cx,
                self.ui_text.get(UiTextKey::SshKeyPassphrase),
                String::new(),
                true,
            ),
            password: ssh_input(
                window,
                cx,
                self.ui_text.get(UiTextKey::SshPassword),
                String::new(),
                true,
            ),
        };
        form.inputs = Some(inputs.clone());
        form.command_subscription = Some(command_subscription);
        Some(inputs)
    }

    fn on_ssh_command_input_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.apply_ssh_command_from_form(window, cx);
        }
    }
    pub(super) fn apply_ssh_command_from_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(inputs) = self
            .ssh
            .form
            .as_ref()
            .and_then(|form| form.inputs.as_ref())
            .cloned()
        else {
            return;
        };
        let command = input_value(&inputs.command, cx);
        match parse_ssh_command(&command) {
            Ok(parsed) => {
                let name_is_empty = input_value(&inputs.name, cx).is_empty();
                let identity_file = parsed.identity_file.map(expand_ssh_identity_path);
                set_ssh_input_value(&inputs.host, parsed.host.clone(), window, cx);
                set_ssh_input_value(&inputs.port, parsed.port.to_string(), window, cx);
                if let Some(user) = parsed.user {
                    set_ssh_input_value(&inputs.user, user, window, cx);
                }
                set_ssh_input_value(
                    &inputs.identity_file,
                    identity_file
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    window,
                    cx,
                );
                if name_is_empty {
                    set_ssh_input_value(&inputs.name, parsed.host, window, cx);
                }
                self.ssh.error = None;
            }
            Err(error) => self.ssh.error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(super) fn save_ssh_connection_from_form(
        &mut self,
        default_name_from_host: bool,
        disconnect_changed_connection: bool,
        cx: &mut Context<Self>,
    ) -> Option<ConnectionId> {
        let form = self.ssh.form.as_ref()?;
        let inputs = form.inputs.as_ref()?;
        let host = input_value(&inputs.host, cx);
        let name = match input_value(&inputs.name, cx) {
            name if name.is_empty() && default_name_from_host => host.clone(),
            name => name,
        };
        let user = input_value(&inputs.user, cx);
        if name.is_empty() || host.is_empty() || user.is_empty() {
            self.ssh.error = Some("Name, host, and user are required.".to_string());
            return None;
        }
        let port = match input_value(&inputs.port, cx).parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => {
                self.ssh.error = Some(self.ui_text.get(UiTextKey::RemoteInvalidPort).into());
                return None;
            }
        };
        let remote_root = match RemotePathBuf::new(input_value(&inputs.remote_root, cx)) {
            Ok(path) if path.as_str().starts_with('/') => path,
            Ok(_) => {
                self.ssh.error = Some(self.ui_text.get(UiTextKey::RemoteInvalidRoot).into());
                return None;
            }
            Err(error) => {
                self.ssh.error = Some(error.to_string());
                return None;
            }
        };
        let identity_file = input_value(&inputs.identity_file, cx);
        let identity_file = (!identity_file.is_empty()).then(|| PathBuf::from(identity_file));
        let private_key_identity = identity_file
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        let keep_credential = match form.auth {
            SshConnectionFormMode::Auto => true,
            SshConnectionFormMode::Password => form.remember_password,
            SshConnectionFormMode::Agent | SshConnectionFormMode::PrivateKey => false,
        };
        let credential = keep_credential
            .then(|| form.initial.credential.clone())
            .flatten()
            .filter(|credential| {
                credential.binding.connection_id == form.connection_id
                    && credential.binding.effective_user == user
                    && credential.binding.resolved_host == host
                    && credential.binding.port == port
                    && credential.binding.private_key_identity == private_key_identity
            });
        let stale_credential_id = form
            .initial
            .credential
            .as_ref()
            .filter(|existing| {
                credential
                    .as_ref()
                    .is_none_or(|retained| retained.id != existing.id)
            })
            .map(|credential| credential.id.clone());
        let connection = SshConnectionConfig {
            id: form.connection_id.clone(),
            name,
            host,
            port,
            user,
            auth: form.auth.into(),
            identity_file,
            credential,
            default_remote_root: Some(remote_root),
        };
        let requires_reconnect = form.initial.host != connection.host
            || form.initial.port != connection.port
            || form.initial.user != connection.user
            || form.initial.auth != connection.auth
            || form.initial.identity_file != connection.identity_file;
        let mut updated = self.ssh.connections.clone();
        if let Some(existing) = updated
            .connections
            .iter_mut()
            .find(|existing| existing.id == connection.id)
        {
            *existing = connection.clone();
        } else {
            updated.connections.push(connection.clone());
        }
        if let Err(error) = save_ssh_connections(&self.config_paths, &updated) {
            self.ssh.error = Some(error.to_string());
            return None;
        }
        self.ssh.connections = updated;
        if let Some(form) = self.ssh.form.as_mut() {
            form.initial = connection.clone();
            if stale_credential_id.is_some() {
                form.credential_id = CredentialId::random();
            }
        }
        self.ssh.error = None;
        let connection_was_active = self.ssh.statuses.get(&connection.id).is_some_and(|status| {
            !matches!(
                status.state,
                ConnectionState::Disconnected | ConnectionState::Failed
            )
        });
        if disconnect_changed_connection && requires_reconnect && connection_was_active {
            self.ssh.statuses.remove(&connection.id);
            let runtime = self.terminal.host_runtime.clone();
            let connection_id = connection.id.clone();
            cx.spawn(async move |this, cx| {
                if let Err(error) = disconnect_host_ssh(runtime, connection_id).await {
                    let _ = this.update(cx, |root, cx| {
                        root.ssh.error = Some(error);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        if let Some(credential_id) = stale_credential_id {
            let runtime = self.terminal.host_runtime.clone();
            let delete_task =
                cx.background_spawn(
                    async move { delete_host_credential(runtime, credential_id).await },
                );
            cx.spawn(async move |this, cx| {
                let result = delete_task.await;
                if let Err(error) = result {
                    let _ = this.update(cx, |root, cx| {
                        root.ssh.error = Some(error.to_string());
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        Some(connection.id)
    }

    fn save_ssh_connection_editor(&mut self, cx: &mut Context<Self>) {
        if self
            .save_ssh_connection_from_form(false, true, cx)
            .is_some()
        {
            self.queue_status_notification(self.ui_text.get(UiTextKey::SshConnectionSaved), "");
            self.close_ssh_connection_editor();
        }
        cx.notify();
    }

    fn save_and_connect_ssh_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((password, key_passphrase)) = self.ssh.form.as_ref().and_then(|form| {
            form.inputs.as_ref().map(|inputs| {
                (
                    secret_input_value(&inputs.password, cx),
                    secret_input_value(&inputs.key_passphrase, cx),
                )
            })
        }) else {
            return;
        };
        let Some(connection_id) = self.save_ssh_connection_from_form(false, true, cx) else {
            cx.notify();
            return;
        };
        let save_password_as = self.ssh.form.as_ref().and_then(|form| {
            (form.auth == SshConnectionFormMode::Password && form.remember_password)
                .then(|| form.credential_id.clone())
        });
        self.launch_saved_ssh_connection(
            connection_id,
            (!password.is_empty()).then_some(password),
            (!key_passphrase.is_empty()).then_some(key_passphrase),
            save_password_as,
            window,
            cx,
        );
    }

    fn submit_ssh_connection_credentials(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((connection_id, password, secret, save_password_as)) = self
            .ssh
            .form
            .as_ref()
            .filter(|_| self.ssh.credentials_only)
            .and_then(|form| {
                form.inputs.as_ref().map(|inputs| {
                    let password = form.auth == SshConnectionFormMode::Password;
                    (
                        form.connection_id.clone(),
                        password,
                        if password {
                            secret_input_value(&inputs.password, cx)
                        } else {
                            secret_input_value(&inputs.key_passphrase, cx)
                        },
                        (password && form.remember_password).then(|| form.credential_id.clone()),
                    )
                })
            })
        else {
            return;
        };
        if secret.is_empty() {
            self.ssh.error = Some(if password {
                self.ui_text.get(UiTextKey::SshPasswordRequired).to_string()
            } else {
                format!(
                    "{} is required.",
                    self.ui_text.get(UiTextKey::SshKeyPassphrase)
                )
            });
            cx.notify();
            return;
        }
        let (password, passphrase) = if password {
            (Some(secret), None)
        } else {
            (None, Some(secret))
        };
        self.launch_saved_ssh_connection(
            connection_id,
            password,
            passphrase,
            save_password_as,
            window,
            cx,
        );
    }

    pub fn connect_ssh_connection(
        &mut self,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.launch_saved_ssh_connection(connection_id, None, None, None, window, cx);
    }

    fn open_ssh_credentials_prompt(
        &mut self,
        connection_id: ConnectionId,
        error: Option<String>,
        remember_password: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_ssh_connection_editor(Some(connection_id), window, cx);
        self.ssh.credentials_only = true;
        if let Some(remember_password) = remember_password
            && let Some(form) = self.ssh.form.as_mut()
        {
            form.remember_password = remember_password;
        }
        self.ssh.error = error;
        if let Some(inputs) = self.ssh_connection_form_inputs(window, cx) {
            let input = self
                .ssh
                .form
                .as_ref()
                .is_some_and(|form| form.auth == SshConnectionFormMode::Password)
                .then_some(&inputs.password)
                .unwrap_or(&inputs.key_passphrase);
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn check_stored_ssh_password(
        &mut self,
        connection_id: ConnectionId,
        credential_id: CredentialId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(local_profile) = self.config_paths.profile().cloned() else {
            self.ssh.error = Some(self.ui_text.get(UiTextKey::RemoteManageLocally).into());
            cx.notify();
            return;
        };
        self.ssh.connecting = Some(connection_id.clone());
        self.ssh.error = None;
        cx.notify();
        let credential_namespace = local_profile.credential_namespace().to_string();
        let task = cx.background_spawn(async move {
            yttt_ssh::CredentialStore::new(credential_namespace)
                .load(&credential_id)
                .map(|secret| secret.is_some())
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let available = task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                if root.ssh.connecting.as_ref() != Some(&connection_id) {
                    return;
                }
                root.ssh.connecting = None;
                match available {
                    Ok(true) => root.launch_saved_ssh_connection_after_credential_check(
                        connection_id,
                        None,
                        None,
                        None,
                        window,
                        cx,
                    ),
                    Ok(false) => {
                        root.open_ssh_credentials_prompt(connection_id, None, None, window, cx)
                    }
                    Err(error) => root.open_ssh_credentials_prompt(
                        connection_id,
                        Some(error),
                        Some(false),
                        window,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    fn launch_saved_ssh_connection(
        &mut self,
        connection_id: ConnectionId,
        password: Option<String>,
        passphrase: Option<String>,
        save_password_as: Option<CredentialId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.launch_saved_ssh_connection_impl(
            connection_id,
            password,
            passphrase,
            save_password_as,
            false,
            window,
            cx,
        );
    }

    fn launch_saved_ssh_connection_after_credential_check(
        &mut self,
        connection_id: ConnectionId,
        password: Option<String>,
        passphrase: Option<String>,
        save_password_as: Option<CredentialId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.launch_saved_ssh_connection_impl(
            connection_id,
            password,
            passphrase,
            save_password_as,
            true,
            window,
            cx,
        );
    }

    fn launch_saved_ssh_connection_impl(
        &mut self,
        connection_id: ConnectionId,
        password: Option<String>,
        passphrase: Option<String>,
        save_password_as: Option<CredentialId>,
        credential_checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ssh.connecting.is_some() {
            return;
        }
        let Some(connection) = self
            .ssh
            .connections
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
            .cloned()
        else {
            self.ssh.error = Some(self.ui_text.get(UiTextKey::RemoteRecordMissing).into());
            cx.notify();
            return;
        };
        if connection.auth == SshAuthPreference::Password && password.is_none() {
            if let Some(credential) = connection.credential.as_ref() {
                if !credential_checked {
                    self.check_stored_ssh_password(
                        connection_id,
                        credential.id.clone(),
                        window,
                        cx,
                    );
                    return;
                }
            } else {
                self.open_ssh_credentials_prompt(connection_id, None, None, window, cx);
                return;
            }
        }
        if matches!(
            connection.auth,
            SshAuthPreference::Auto | SshAuthPreference::PublicKey
        ) && passphrase.is_none()
            && connection
                .identity_file
                .as_deref()
                .is_some_and(ssh_identity_requires_passphrase)
        {
            self.open_ssh_credentials_prompt(connection_id, None, None, window, cx);
            return;
        }
        let Some(local_profile) = self.config_paths.profile().cloned() else {
            self.ssh.error = Some(self.ui_text.get(UiTextKey::RemoteManageLocally).into());
            cx.notify();
            return;
        };
        self.ssh.connecting = Some(connection_id.clone());
        self.ssh.error = None;
        cx.notify();
        let appearance = crate::remote_launch::RemoteAppearance::capture(cx, self.ui_text);
        let launch = crate::remote_launch::RemoteLaunch {
            local_profile,
            appearance,
            target: crate::remote_launch::RemoteTarget::SshServer {
                connection,
                password,
                passphrase,
                save_password_as,
            },
        };
        let task =
            cx.background_spawn(async move { crate::remote_launch::spawn_remote_client(launch) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, _window, cx| {
                if root.ssh.connecting.as_ref() != Some(&connection_id) {
                    return;
                }
                root.ssh.connecting = None;
                match result {
                    Ok(()) => root.close_ssh_connection_manager(),
                    Err(error) => {
                        root.ssh.error = Some(format!(
                            "{}: {error}",
                            root.ui_text.get(UiTextKey::RemoteLaunchFailed)
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn disconnect_ssh_connection(
        &mut self,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let runtime = self.terminal.host_runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = disconnect_host_ssh(runtime, connection_id).await;
            let _ = this.update_in(cx, |root, _window, cx| {
                if let Err(error) = result {
                    root.ssh.error = Some(error);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn delete_ssh_connection(
        &mut self,
        connection_id: ConnectionId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.opened_projects().iter().any(|project| {
            matches!(
                &project.location,
                ProjectLocation::Ssh {
                    connection_id: project_connection_id,
                    ..
                } if project_connection_id == &connection_id
            )
        }) {
            self.ssh.error = Some(
                self.ui_text
                    .get(UiTextKey::SshConnectionDeleteInUse)
                    .to_string(),
            );
            cx.notify();
            return;
        }
        let Some(connection) = self
            .ssh
            .connections
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
            .cloned()
        else {
            return;
        };
        let credential_id = connection
            .credential
            .as_ref()
            .map(|credential| credential.id.clone());
        let mut updated = self.ssh.connections.clone();
        updated
            .connections
            .retain(|connection| connection.id != connection_id);
        if let Err(error) = remove_recent_projects_for_ssh_connection(
            &self.config_paths,
            &mut self.recent_projects_config,
            &connection_id,
        ) {
            self.ssh.error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.palette.recent_projects = recent_projects_for_palette(&self.recent_projects_config);
        if let Err(error) = save_ssh_connections(&self.config_paths, &updated) {
            self.ssh.error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.ssh.connections = updated;
        if self
            .ssh
            .form
            .as_ref()
            .is_some_and(|form| form.connection_id == connection_id)
        {
            self.close_ssh_connection_editor();
        }
        self.ssh.error = None;
        self.ssh.statuses.remove(&connection_id);
        cx.notify();
        let disconnect_id = connection_id.clone();
        let runtime = self.terminal.host_runtime.clone();
        cx.spawn(async move |this, cx| {
            let disconnect_error = disconnect_host_ssh(runtime.clone(), disconnect_id)
                .await
                .err();
            let delete_error = match credential_id {
                Some(credential_id) => delete_host_credential(runtime, credential_id).await.err(),
                None => None,
            };
            if let Some(error) = disconnect_error.or(delete_error) {
                let _ = this.update(cx, |root, cx| {
                    root.ssh.error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn set_ssh_auth_mode(&mut self, mode: SshConnectionFormMode) {
        if let Some(form) = self.ssh.form.as_mut() {
            form.auth = mode;
        }
    }

    pub(super) fn open_ssh_project_location(
        &mut self,
        connection_id: ConnectionId,
        root: RemotePathBuf,
        require_connected: bool,
    ) -> Result<(), WorkbenchError> {
        self.open_ssh_project_location_with_mode(
            connection_id,
            root,
            require_connected,
            ProjectOpenMode::Fresh,
        )
    }

    pub(super) fn open_ssh_project_location_with_mode(
        &mut self,
        connection_id: ConnectionId,
        root: RemotePathBuf,
        require_connected: bool,
        mode: ProjectOpenMode,
    ) -> Result<(), WorkbenchError> {
        if mode == ProjectOpenMode::Fresh && !self.shared_mutation_allowed() {
            return Err(WorkbenchError::RemoteProject(
                "Shared editing control is required.".to_string(),
            ));
        }

        let connection = self
            .ssh
            .connections
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
            .cloned()
            .ok_or_else(|| {
                WorkbenchError::RemoteProject(format!(
                    "SSH connection {} is not configured",
                    connection_id.as_str()
                ))
            })?;
        if require_connected
            && !self
                .ssh
                .statuses
                .get(&connection_id)
                .is_some_and(|status| status.state == ConnectionState::Connected)
        {
            return Err(WorkbenchError::RemoteProject(
                "Connect the SSH endpoint before opening its remote project.".to_string(),
            ));
        }
        let runtime = self.terminal.host_runtime.clone().ok_or_else(|| {
            WorkbenchError::RemoteProject("SSH runtime is unavailable.".to_string())
        })?;
        let title = root
            .file_name()
            .filter(|name| !name.is_empty())
            .unwrap_or(connection.name.as_str())
            .to_string();
        let opened = open_ssh_project_config(
            &self.config_paths,
            connection_id.clone(),
            root.clone(),
            &title,
            &mut self.default_layout_state,
        )?;
        let source_message = layout_source_message(&opened.layout_source);
        let warning_message = layout_load_warning_message(&opened.warnings);
        let services = ProjectServices::ssh(
            runtime,
            opened.descriptor.id.clone(),
            connection_id,
            root.clone(),
        )
        .map_err(WorkbenchError::RemoteProject)?;
        let already_open = self.workspace.project(&opened.descriptor.id).is_some();
        let project_id = match self
            .workspace
            .open_project(opened.descriptor, opened.layout)
        {
            Ok(project_id) => project_id,
            Err(error) => {
                let _ = services.close_host_registration();
                return Err(error.into());
            }
        };
        if !already_open {
            match mode {
                ProjectOpenMode::Fresh => self
                    .agent_manager
                    .reset_project_sessions(project_id.as_str()),
                ProjectOpenMode::RestoreLastSession => {
                    self.restore_project_agent_snapshots(&project_id)
                }
            }
        }
        let selected_terminal_id = self.workspace.project(&project_id).and_then(|project| {
            project
                .layout
                .tab(&project.selected_tab_id)
                .map(|_| project.selected_tab_id.clone())
        });
        self.project.services.insert(project_id.clone(), services);
        self.project.project_editor_runtime.open_project(
            project_id.clone(),
            PathBuf::from(root.as_str()),
            selected_terminal_id,
            self.app_settings.project_panel.default_open,
            self.app_settings.project_panel.width,
        );
        self.queue_selected_terminal_focus();
        self.project
            .layout_source_messages
            .insert(project_id, source_message);
        self.recent_projects_config = opened.recent_projects;
        let persistence_error = self.persist_opened_project_paths();
        self.palette.recent_projects = recent_projects_for_palette(&self.recent_projects_config);
        self.load_error = combine_load_messages(warning_message, persistence_error);
        Ok(())
    }

    pub(super) fn reject_stale_ssh_host_key_challenges(
        &mut self,
        connection_id: &ConnectionId,
        epoch: u64,
    ) {
        let mut retained = VecDeque::with_capacity(self.ssh.pending_host_keys.len());
        while let Some(challenge) = self.ssh.pending_host_keys.pop_front() {
            if challenge.connection_id == *connection_id && challenge.epoch != epoch {
                self.send_ssh_host_key_answer(challenge, false, false);
            } else {
                retained.push_back(challenge);
            }
        }
        self.ssh.pending_host_keys = retained;
    }

    pub(super) fn send_ssh_host_key_answer(
        &mut self,
        challenge: HostKeyChallenge,
        accept: bool,
        remember: bool,
    ) {
        let answer = if !accept {
            HostKeyDecision::Reject
        } else if remember {
            HostKeyDecision::AcceptAndStore
        } else {
            HostKeyDecision::AcceptOnce
        };
        let Some(runtime) = self.terminal.host_runtime.as_ref() else {
            self.ssh.error = Some(self.ui_text.get(UiTextKey::RemoteRuntimeUnavailable).into());
            return;
        };
        if let Err(error) = runtime.request_detached(Request::CredentialAnswer {
            challenge_id: challenge.challenge_id,
            answer: CredentialAnswer::HostKey(answer),
        }) {
            self.ssh.error = Some(error.to_string());
        }
    }

    pub fn answer_ssh_host_key(&mut self, accept: bool, remember: bool) {
        if let Some(challenge) = self.ssh.pending_host_keys.pop_front() {
            self.send_ssh_host_key_answer(challenge, accept, remember);
        }
    }
}

fn ssh_input(
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
    placeholder: &'static str,
    value: String,
    masked: bool,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
            .masked(masked)
    })
}

fn input_value(input: &Entity<InputState>, cx: &Context<WorkbenchView>) -> String {
    input.read(cx).value().trim().to_string()
}

fn secret_input_value(input: &Entity<InputState>, cx: &Context<WorkbenchView>) -> String {
    input.read(cx).value().to_string()
}

fn ssh_identity_requires_passphrase(path: &Path) -> bool {
    let Ok(pem) = std::fs::read_to_string(path) else {
        return false;
    };
    let pem = zeroize::Zeroizing::new(pem);
    if pem.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----")
        || pem.contains("Proc-Type: 4,ENCRYPTED")
    {
        return true;
    }
    let encoded = zeroize::Zeroizing::new(
        pem.lines()
            .filter(|line| !line.starts_with("-----"))
            .collect::<String>(),
    );
    let Ok(payload) = base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) else {
        return false;
    };
    let payload = zeroize::Zeroizing::new(payload);
    let Some(payload) = payload.strip_prefix(b"openssh-key-v1\0") else {
        return false;
    };
    let Some((cipher, _)) = ssh_wire_string(payload) else {
        return false;
    };
    cipher != b"none"
}

fn ssh_wire_string(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let length = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
    Some((bytes.get(4..4 + length)?, bytes.get(4 + length..)?))
}

pub(super) fn ssh_connection_state_text(state: ConnectionState, text: &UiText) -> &'static str {
    match state {
        ConnectionState::Disconnected => text.get(UiTextKey::SshDisconnected),
        ConnectionState::Connecting => text.get(UiTextKey::SshConnecting),
        ConnectionState::VerifyingHostKey => text.get(UiTextKey::SshVerifyingHostKey),
        ConnectionState::Authenticating => text.get(UiTextKey::SshAuthenticating),
        ConnectionState::Connected => text.get(UiTextKey::SshConnected),
        ConnectionState::Reconnecting => text.get(UiTextKey::SshReconnecting),
        ConnectionState::Failed => text.get(UiTextKey::SshFailed),
    }
}

pub(super) fn ssh_connection_editor(
    root: &mut WorkbenchView,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let ui_style = current_ui_style(cx);
    let dialog = yttt_dialog_style(theme, ui_style);
    let Some(inputs) = root.ssh_connection_form_inputs(window, cx) else {
        return div();
    };
    let auth = root
        .ssh
        .form
        .as_ref()
        .map(|form| form.auth)
        .unwrap_or(SshConnectionFormMode::Auto);
    let remember_password = root
        .ssh
        .form
        .as_ref()
        .is_some_and(|form| form.remember_password);
    let connecting = root.ssh.connecting.is_some();

    if root.ssh.credentials_only {
        let password_prompt = auth == SshConnectionFormMode::Password;
        let mut fields = div()
            .flex()
            .flex_col()
            .gap(ui_style.spacing.md)
            .child(
                div().text_sm().text_color(theme.text_muted).child(
                    root.ssh
                        .form
                        .as_ref()
                        .map(|form| {
                            format!(
                                "{}@{}:{}",
                                form.initial.user, form.initial.host, form.initial.port
                            )
                        })
                        .unwrap_or_default(),
                ),
            )
            .child(if password_prompt {
                ssh_form_field(
                    root.ui_text.get(UiTextKey::SshPassword),
                    &inputs.password,
                    theme,
                    ui_style,
                )
            } else {
                ssh_form_field(
                    root.ui_text.get(UiTextKey::SshKeyPassphrase),
                    &inputs.key_passphrase,
                    theme,
                    ui_style,
                )
            });
        if password_prompt {
            fields = fields.child(yttt_labeled_switch(
                "ssh-remember-password",
                root.ui_text.get(UiTextKey::SshRememberPassword),
                remember_password,
                theme,
                ui_style,
                cx.listener(|this, checked: &bool, _window, cx| {
                    if let Some(form) = this.ssh.form.as_mut() {
                        form.remember_password = *checked;
                    }
                    cx.notify();
                }),
            ));
        }
        if let Some(error) = root.ssh.error.clone() {
            fields = fields.child(
                yttt_alert(
                    "ssh-connection-error",
                    error,
                    YtttNotificationTone::Error,
                    theme,
                    ui_style,
                )
                .title(root.ui_text.get(UiTextKey::SshFailed)),
            );
        }
        return div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                div()
                    .id("remote-service-form-scroll")
                    .debug_selector(|| "ssh-form-viewport".into())
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .vertical_scrollbar(&root.auxiliary_windows.ssh_form_scroll)
                    .p(gpui::rems(1.5))
                    .child(fields.w_full().max_w(gpui::rems(44.0))),
            )
            .child(
                div()
                    .debug_selector(|| "ssh-form-footer".into())
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .min_h(gpui::rems(3.0))
                    .px(gpui::rems(1.0))
                    .py(gpui::rems(0.5))
                    .border_t_1()
                    .border_color(dialog.border)
                    .gap(ui_style.spacing.sm)
                    .child(
                        yttt_dialog_button(
                            cx,
                            "cancel-ssh-connection-credentials",
                            root.ui_text.get(UiTextKey::Cancel),
                            YtttButtonVariant::Ghost,
                            theme,
                            cx.listener(|this, _, _window, cx| {
                                this.close_ssh_connection_editor();
                                cx.notify();
                            }),
                        )
                        .disabled(connecting),
                    )
                    .child(
                        yttt_dialog_button(
                            cx,
                            "connect-ssh-connection-credentials",
                            root.ui_text.get(if connecting {
                                UiTextKey::RemoteConnecting
                            } else {
                                UiTextKey::SshConnect
                            }),
                            YtttButtonVariant::Primary,
                            theme,
                            cx.listener(|this, _, window, cx| {
                                this.submit_ssh_connection_credentials(window, cx);
                            }),
                        )
                        .disabled(connecting),
                    ),
            );
    }

    let auth_index = match auth {
        SshConnectionFormMode::Auto => 0,
        SshConnectionFormMode::Agent => 1,
        SshConnectionFormMode::Password => 2,
        SshConnectionFormMode::PrivateKey => 3,
    };
    let mut form_fields = div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.md)
        .child(
            div()
                .debug_selector(|| "ssh-command-field".into())
                .flex()
                .flex_col()
                .gap(ui_style.spacing.xs)
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(root.ui_text.get(UiTextKey::SshCommand)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(ui_style.spacing.sm)
                        .child(div().min_w_0().flex_1().child(yttt_dialog_input(
                            &inputs.command,
                            theme,
                            ui_style,
                        )))
                        .child(yttt_dialog_button(
                            cx,
                            "parse-ssh-command",
                            root.ui_text.get(UiTextKey::SshCommandParse),
                            YtttButtonVariant::Secondary,
                            theme,
                            cx.listener(|this, _, window, cx| {
                                this.apply_ssh_command_from_form(window, cx);
                            }),
                        )),
                ),
        )
        .child(ssh_form_field(
            root.ui_text.get(UiTextKey::SshConnectionName),
            &inputs.name,
            theme,
            ui_style,
        ))
        .child(
            div()
                .flex()
                .gap(ui_style.spacing.md)
                .child(
                    ssh_form_field(
                        root.ui_text.get(UiTextKey::SshHost),
                        &inputs.host,
                        theme,
                        ui_style,
                    )
                    .flex_1(),
                )
                .child(
                    ssh_form_field(
                        root.ui_text.get(UiTextKey::SshPort),
                        &inputs.port,
                        theme,
                        ui_style,
                    )
                    .w(px(110.0)),
                ),
        )
        .child(ssh_form_field(
            root.ui_text.get(UiTextKey::SshUser),
            &inputs.user,
            theme,
            ui_style,
        ))
        .child(ssh_form_field(
            root.ui_text.get(UiTextKey::SshRemoteRoot),
            &inputs.remote_root,
            theme,
            ui_style,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.xs)
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(root.ui_text.get(UiTextKey::SshAuthentication)),
                )
                .child(
                    RadioGroup::horizontal("ssh-authentication")
                        .children([
                            root.ui_text.get(UiTextKey::SshAuthAuto),
                            root.ui_text.get(UiTextKey::SshAuthAgent),
                            root.ui_text.get(UiTextKey::SshAuthPassword),
                            root.ui_text.get(UiTextKey::SshAuthPrivateKey),
                        ])
                        .selected_index(Some(auth_index))
                        .on_click(cx.listener(|this, index: &usize, _window, cx| {
                            let mode = match *index {
                                1 => SshConnectionFormMode::Agent,
                                2 => SshConnectionFormMode::Password,
                                3 => SshConnectionFormMode::PrivateKey,
                                _ => SshConnectionFormMode::Auto,
                            };
                            this.set_ssh_auth_mode(mode);
                            cx.notify();
                        })),
                ),
        );
    if matches!(
        auth,
        SshConnectionFormMode::Auto | SshConnectionFormMode::PrivateKey
    ) {
        form_fields = form_fields
            .child(ssh_form_field(
                root.ui_text.get(UiTextKey::SshIdentityFile),
                &inputs.identity_file,
                theme,
                ui_style,
            ))
            .child(ssh_form_field(
                root.ui_text.get(UiTextKey::SshKeyPassphrase),
                &inputs.key_passphrase,
                theme,
                ui_style,
            ));
    }
    if auth == SshConnectionFormMode::Password {
        form_fields = form_fields
            .child(ssh_form_field(
                root.ui_text.get(UiTextKey::SshPassword),
                &inputs.password,
                theme,
                ui_style,
            ))
            .child(yttt_labeled_switch(
                "ssh-remember-password",
                root.ui_text.get(UiTextKey::SshRememberPassword),
                remember_password,
                theme,
                ui_style,
                cx.listener(|this, checked: &bool, _window, cx| {
                    if let Some(form) = this.ssh.form.as_mut() {
                        form.remember_password = *checked;
                    }
                    cx.notify();
                }),
            ));
    }
    if let Some(error) = root.ssh.error.clone() {
        let title = if error.to_ascii_lowercase().contains("host key")
            || error.contains("HOST IDENTIFICATION HAS CHANGED")
        {
            UiTextKey::SshHostKeyTitle
        } else {
            UiTextKey::SshFailed
        };
        form_fields = form_fields.child(
            yttt_alert(
                "ssh-connection-error",
                error,
                YtttNotificationTone::Error,
                theme,
                ui_style,
            )
            .title(root.ui_text.get(title)),
        );
    }

    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .child(
            div()
                .id("remote-service-form-scroll")
                .debug_selector(|| "ssh-form-viewport".into())
                .min_h_0()
                .flex_1()
                .overflow_y_scroll()
                .vertical_scrollbar(&root.auxiliary_windows.ssh_form_scroll)
                .p(gpui::rems(1.5))
                .child(form_fields.w_full().max_w(gpui::rems(44.0))),
        )
        .child(
            div()
                .debug_selector(|| "ssh-form-footer".into())
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .min_h(gpui::rems(3.0))
                .px(gpui::rems(1.0))
                .py(gpui::rems(0.5))
                .border_t_1()
                .border_color(dialog.border)
                .gap(ui_style.spacing.sm)
                .child(
                    yttt_dialog_button(
                        cx,
                        "cancel-ssh-connection",
                        root.ui_text.get(UiTextKey::Cancel),
                        YtttButtonVariant::Ghost,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.close_ssh_connection_editor();
                            cx.notify();
                        }),
                    )
                    .disabled(connecting),
                )
                .child(
                    yttt_dialog_button(
                        cx,
                        "save-ssh-connection",
                        root.ui_text.get(UiTextKey::SettingsSave),
                        YtttButtonVariant::Secondary,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.save_ssh_connection_editor(cx);
                        }),
                    )
                    .disabled(connecting),
                )
                .child(
                    yttt_dialog_button(
                        cx,
                        "save-connect-ssh-connection",
                        root.ui_text.get(if connecting {
                            UiTextKey::RemoteConnecting
                        } else {
                            UiTextKey::RemoteSaveConnect
                        }),
                        YtttButtonVariant::Primary,
                        theme,
                        cx.listener(|this, _, window, cx| {
                            this.save_and_connect_ssh_connection(window, cx);
                        }),
                    )
                    .disabled(connecting),
                ),
        )
}

pub(super) fn ssh_host_key_overlay(root: &WorkbenchView, cx: &mut Context<WorkbenchView>) -> Div {
    let Some(challenge) = root.ssh.pending_host_keys.front() else {
        return div();
    };
    let theme = root.theme_runtime().ui;
    let ui_style = current_ui_style(cx);
    let dialog = yttt_dialog_style(theme, ui_style);
    let host_key_changed = challenge.previous_fingerprint.is_some();
    let title = root.ui_text.get(if host_key_changed {
        UiTextKey::SshHostKeyChangedTitle
    } else {
        UiTextKey::SshHostKeyTitle
    });
    let description = root.ui_text.get(if host_key_changed {
        UiTextKey::SshHostKeyChangedDescription
    } else {
        UiTextKey::SshHostKeyDescription
    });
    let save_label = root.ui_text.get(if host_key_changed {
        UiTextKey::SshHostKeyReplace
    } else {
        UiTextKey::SshHostKeyTrustAndSave
    });
    yttt_dialog_overlay(
        yttt_dialog_surface(theme, ui_style)
            .gap(ui_style.spacing.lg)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(if host_key_changed {
                yttt_alert(
                    "ssh-host-key-changed-warning",
                    description,
                    YtttNotificationTone::Warning,
                    theme,
                    ui_style,
                )
                .into_any_element()
            } else {
                div()
                    .text_sm()
                    .text_color(dialog.hint)
                    .child(description)
                    .into_any_element()
            })
            .child(
                div()
                    .text_sm()
                    .child(format!("{}:{}", challenge.host, challenge.port)),
            )
            .child(
                div()
                    .font_family("monospace")
                    .text_sm()
                    .child(challenge.algorithm.clone()),
            )
            .when_some(
                challenge.previous_fingerprint.clone(),
                |panel, fingerprint| {
                    panel.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(ui_style.spacing.xs)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(dialog.hint)
                                    .child(root.ui_text.get(UiTextKey::SshHostKeySavedFingerprint)),
                            )
                            .child(div().font_family("monospace").text_sm().child(fingerprint)),
                    )
                },
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(ui_style.spacing.xs)
                    .child(
                        div()
                            .text_xs()
                            .text_color(dialog.hint)
                            .child(root.ui_text.get(UiTextKey::SshHostKeyReceivedFingerprint)),
                    )
                    .child(
                        div()
                            .font_family("monospace")
                            .text_sm()
                            .child(challenge.fingerprint.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(ui_style.spacing.md)
                    .child(yttt_dialog_button(
                        cx,
                        "reject-ssh-host-key",
                        root.ui_text.get(UiTextKey::SshHostKeyReject),
                        YtttButtonVariant::Danger,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.answer_ssh_host_key(false, false);
                            cx.notify();
                        }),
                    ))
                    .child(yttt_dialog_button(
                        cx,
                        "trust-ssh-host-key-once",
                        root.ui_text.get(UiTextKey::SshHostKeyTrustOnce),
                        YtttButtonVariant::Secondary,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.answer_ssh_host_key(true, false);
                            cx.notify();
                        }),
                    ))
                    .child(yttt_dialog_button(
                        cx,
                        "trust-and-save-ssh-host-key",
                        save_label,
                        YtttButtonVariant::Primary,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.answer_ssh_host_key(true, true);
                            cx.notify();
                        }),
                    )),
            ),
        YtttDialogPlacement::Top,
        theme,
        ui_style,
    )
}

pub(super) fn ssh_form_field(
    label: &'static str,
    input: &Entity<InputState>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.xs)
        .child(div().text_xs().text_color(theme.text_muted).child(label))
        .child(yttt_dialog_input(input, theme, ui_style))
}

fn expand_ssh_identity_path(path: PathBuf) -> PathBuf {
    let Ok(relative) = path.strip_prefix("~") else {
        return path;
    };
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    home.map(PathBuf::from)
        .map(|home| home.join(relative))
        .unwrap_or(path)
}

fn set_ssh_input_value(
    input: &Entity<InputState>,
    value: String,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) {
    input.update(cx, |input, cx| input.set_value(value, window, cx));
}

#[cfg(test)]
mod tests {
    use yttt_protocol::ssh::{SshConnectionState, SshConnectionStatus};

    use super::*;

    #[test]
    fn ssh_listener_rejects_terminal_events_before_updating_workbench() {
        assert!(is_ssh_host_event(&ServerEvent::SshStateChanged(
            SshConnectionStatus {
                connection_id: "ssh".to_string(),
                epoch: 1,
                state: SshConnectionState::Connected,
                error: None,
            }
        )));
        assert!(!is_ssh_host_event(&ServerEvent::TerminalExit {
            session_id: TerminalSessionId::new("terminal"),
            session_epoch: 1,
            code: Some(0),
            final_sequence: 1,
        }));
    }
}
