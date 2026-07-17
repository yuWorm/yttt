use std::{collections::VecDeque, path::PathBuf};

use gpui_component::{
    alert::Alert,
    list::{List, ListEvent, ListState},
    radio::RadioGroup,
    switch::Switch,
};
use yttt_core::model::{
    ids::{ConnectionId, CredentialId},
    project::RemotePathBuf,
};
use yttt_ssh::{
    Authentication, ConnectRequest, ConnectionEpoch, ConnectionState, HostKeyDecision, SshEndpoint,
    StoredCredential, TransportError, TransportEvent,
};
use zeroize::Zeroizing;

use crate::config::ssh::{
    CredentialBinding, CredentialKind, CredentialRef, SshAuthPreference, SshConnectionConfig,
    save_ssh_connections,
};
use crate::config::ssh_command::{format_ssh_command, parse_ssh_command};

use super::*;

impl WorkbenchView {
    pub fn start_ssh_event_listener(&mut self, cx: &mut Context<Self>) {
        if self.ssh.event_task.is_some() {
            return;
        }
        let Some(transport) = self.ssh.transport.as_ref() else {
            return;
        };
        let events = transport.events();
        self.ssh.event_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |root, cx| {
                        root.apply_ssh_transport_event(event, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn apply_ssh_transport_event(&mut self, event: TransportEvent, cx: &mut Context<Self>) {
        match event {
            TransportEvent::StateChanged(status) => {
                if self
                    .ssh
                    .statuses
                    .get(&status.connection_id)
                    .is_some_and(|current| current.epoch.get() > status.epoch.get())
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
            TransportEvent::HostKeyChallenge(challenge) => {
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
                    let _ = challenge.respond(HostKeyDecision {
                        accept: false,
                        remember: false,
                    });
                }
            }
            TransportEvent::CredentialSaved {
                connection_id,
                epoch,
                credential,
            } => {
                let current_epoch_matches = self
                    .ssh
                    .statuses
                    .get(&connection_id)
                    .is_some_and(|status| status.epoch == epoch);
                if !current_epoch_matches {
                    let store = self.ssh.credential_store.clone();
                    cx.background_spawn(async move {
                        let _ = store.delete(&credential.id);
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
                    connection.credential = Some(CredentialRef {
                        id: credential.id.clone(),
                        kind: CredentialKind::LoginPassword,
                        binding: CredentialBinding {
                            connection_id: connection_id.clone(),
                            effective_user: credential.effective_user.clone(),
                            resolved_host: credential.resolved_host.clone(),
                            port: credential.port,
                            host_key_sha256: credential.host_key_sha256.clone(),
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
                        let store = self.ssh.credential_store.clone();
                        let credential_id = credential.id;
                        cx.background_spawn(async move {
                            let _ = store.delete(&credential_id);
                        })
                        .detach();
                    }
                }
            }
        }
    }

    pub fn open_ssh_connection_manager(&mut self) {
        self.ssh.manager_open = true;
        self.ssh.error = None;
        if self.ssh.form.is_none() {
            if let Some(connection) = self.ssh.connections.connections.first().cloned() {
                self.ssh.form = Some(SshConnectionForm::new(connection));
            } else {
                self.new_ssh_connection_form();
            }
        }
        self.sync_input_owner_state();
    }

    pub fn close_ssh_connection_manager(&mut self) {
        self.ssh.manager_open = false;
        self.ssh.form = None;
        self.sync_input_owner_state();
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
                self.ssh.error = Some("SSH port must be between 1 and 65535.".to_string());
                return None;
            }
        };
        let remote_root = match RemotePathBuf::new(input_value(&inputs.remote_root, cx)) {
            Ok(path) if path.as_str().starts_with('/') => path,
            Ok(_) => {
                self.ssh.error = Some("Remote root must be an absolute POSIX path.".to_string());
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
            if let Some(transport) = self.ssh.transport.clone() {
                let connection_id = connection.id.clone();
                cx.spawn(async move |this, cx| {
                    if let Err(error) = transport.disconnect(connection_id).await {
                        let _ = this.update(cx, |root, cx| {
                            root.ssh.error = Some(error.to_string());
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
        }
        if let Some(credential_id) = stale_credential_id {
            let store = self.ssh.credential_store.clone();
            let delete_task = cx.background_spawn(async move { store.delete(&credential_id) });
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

    pub fn save_ssh_connection(&mut self, cx: &mut Context<Self>) {
        if self
            .save_ssh_connection_from_form(false, true, cx)
            .is_some()
        {
            self.queue_status_notification(self.ui_text.get(UiTextKey::SshConnectionSaved), "");
        }
    }

    pub fn connect_ssh_connection(
        &mut self,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(saved_id) = self.save_ssh_connection_from_form(false, false, cx) else {
            return;
        };
        let connection_id = if saved_id == connection_id {
            connection_id
        } else {
            saved_id
        };
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
        let (password, key_passphrase) = self
            .ssh
            .form
            .as_ref()
            .filter(|form| form.connection_id == connection_id)
            .and_then(|form| form.inputs.as_ref())
            .map(|inputs| {
                (
                    secret_input_value(&inputs.password, cx),
                    secret_input_value(&inputs.key_passphrase, cx),
                )
            })
            .unwrap_or_default();
        let key_passphrase = (!key_passphrase.is_empty()).then(|| Zeroizing::new(key_passphrase));
        let authentication = match connection.auth {
            SshAuthPreference::Auto => Authentication::Auto {
                identity_file: connection.identity_file.clone(),
                passphrase: key_passphrase,
                credential: connection
                    .credential
                    .as_ref()
                    .map(stored_credential_from_ref),
            },
            SshAuthPreference::Agent => Authentication::Agent,
            SshAuthPreference::Password if password.is_empty() => {
                let Some(credential) = connection.credential.as_ref() else {
                    self.ssh.error = Some("Enter a password before connecting.".to_string());
                    return;
                };
                Authentication::StoredPassword(stored_credential_from_ref(credential))
            }
            SshAuthPreference::Password => Authentication::Password {
                secret: Zeroizing::new(password),
                save_as: self
                    .ssh
                    .form
                    .as_ref()
                    .is_some_and(|form| form.remember_password)
                    .then(|| {
                        self.ssh
                            .form
                            .as_ref()
                            .expect("form checked above")
                            .credential_id
                            .clone()
                    }),
            },
            SshAuthPreference::PublicKey => {
                let Some(path) = connection.identity_file.clone() else {
                    self.ssh.error =
                        Some("Private-key authentication requires an identity file.".to_string());
                    return;
                };
                Authentication::PrivateKey {
                    path,
                    passphrase: key_passphrase,
                }
            }
        };
        let Some(transport) = self.ssh.transport.clone() else {
            self.ssh.error = Some("SSH runtime is unavailable.".to_string());
            return;
        };
        self.ssh.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = transport
                .connect(ConnectRequest {
                    connection_id,
                    endpoint: SshEndpoint {
                        host: connection.host,
                        port: connection.port,
                        user: connection.user,
                    },
                    authentication,
                    reconnect: false,
                })
                .await;
            let _ = this.update_in(cx, |root, _window, cx| {
                if let Err(error) = result
                    && !matches!(error, TransportError::Superseded)
                {
                    root.ssh.error = Some(error.to_string());
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
        let Some(transport) = self.ssh.transport.clone() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = transport.disconnect(connection_id).await;
            let _ = this.update_in(cx, |root, _window, cx| {
                if let Err(error) = result {
                    root.ssh.error = Some(error.to_string());
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
        self.ssh.form = None;
        if let Some(connection) = self.ssh.connections.connections.first().cloned() {
            self.ssh.form = Some(SshConnectionForm::new(connection));
        } else {
            self.new_ssh_connection_form();
        }
        self.ssh.error = None;
        self.ssh.statuses.remove(&connection_id);
        let transport = self.ssh.transport.clone();
        let disconnect_id = connection_id.clone();
        let store = self.ssh.credential_store.clone();
        let delete_task = cx.background_spawn(async move {
            match credential_id {
                Some(credential_id) => store.delete(&credential_id),
                None => Ok(()),
            }
        });
        cx.spawn(async move |this, cx| {
            let disconnect_error = match transport {
                Some(transport) => transport
                    .disconnect(disconnect_id)
                    .await
                    .err()
                    .map(|error| error.to_string()),
                None => None,
            };
            let delete_error = delete_task.await.err().map(|error| error.to_string());
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
        let transport = self.ssh.transport.clone().ok_or_else(|| {
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
        let project_id = self
            .workspace
            .open_project(opened.descriptor, opened.layout)?;
        let selected_terminal_id = self.workspace.project(&project_id).and_then(|project| {
            project
                .layout
                .tab(&project.selected_tab_id)
                .map(|_| project.selected_tab_id.clone())
        });
        self.project.services.insert(
            project_id.clone(),
            ProjectServices::ssh(transport.sftp_project(connection_id, root.clone())),
        );
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
        epoch: ConnectionEpoch,
    ) {
        let mut retained = VecDeque::with_capacity(self.ssh.pending_host_keys.len());
        while let Some(challenge) = self.ssh.pending_host_keys.pop_front() {
            if challenge.connection_id == *connection_id && challenge.epoch != epoch {
                let _ = challenge.respond(HostKeyDecision {
                    accept: false,
                    remember: false,
                });
            } else {
                retained.push_back(challenge);
            }
        }
        self.ssh.pending_host_keys = retained;
    }

    pub(super) fn ssh_manager_connection_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ListState<SshConnectionListDelegate>> {
        let entries = self
            .ssh
            .connections
            .connections
            .iter()
            .map(|connection| {
                let (status, tone) = ssh_connection_status(
                    self.ssh
                        .statuses
                        .get(&connection.id)
                        .map(|status| status.state),
                    &self.ui_text,
                );
                SshConnectionListEntry {
                    action: SshConnectionListAction::Edit(connection.id.clone()),
                    title: connection.name.clone().into(),
                    subtitle: format!(
                        "{}@{}:{}",
                        connection.user, connection.host, connection.port
                    )
                    .into(),
                    status: status.into(),
                    tone,
                }
            })
            .collect();
        let sections = vec![SshConnectionListSection {
            title: self.ui_text.get(UiTextKey::SshConnections).into(),
            entries,
        }];
        let selected_action = self
            .ssh
            .form
            .as_ref()
            .map(|form| SshConnectionListAction::Edit(form.connection_id.clone()));

        if let Some(list) = self.ssh.manager_connection_list.clone() {
            list.update(cx, |list, cx| {
                list.delegate_mut().replace_sections(sections);
                let selected_index = selected_action
                    .as_ref()
                    .and_then(|action| list.delegate().index_of(action));
                if list.selected_index() != selected_index {
                    list.set_selected_index(selected_index, window, cx);
                }
            });
            return list;
        }

        let empty_message = self.ui_text.get(UiTextKey::SshNoConnections);
        let list = cx.new(|cx| {
            ListState::new(
                SshConnectionListDelegate::new(sections, empty_message),
                window,
                cx,
            )
        });
        let subscription = cx.subscribe(
            &list,
            |this, list: Entity<ListState<SshConnectionListDelegate>>, event, cx| {
                let ListEvent::Confirm(index) = event else {
                    return;
                };
                let action = list.read(cx).delegate().action(*index).cloned();
                if let Some(SshConnectionListAction::Edit(connection_id)) = action {
                    this.edit_ssh_connection(&connection_id);
                    cx.notify();
                }
            },
        );
        if let Some(selected_action) = selected_action {
            list.update(cx, |list, cx| {
                let selected_index = list.delegate().index_of(&selected_action);
                list.set_selected_index(selected_index, window, cx);
            });
        }
        self.ssh.manager_connection_list = Some(list.clone());
        self.ssh.manager_connection_list_subscription = Some(subscription);
        list
    }

    pub fn answer_ssh_host_key(&mut self, accept: bool, remember: bool) {
        if let Some(challenge) = self.ssh.pending_host_keys.pop_front() {
            let _ = challenge.respond(HostKeyDecision { accept, remember });
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

pub(super) fn ssh_connection_status(
    state: Option<ConnectionState>,
    text: &UiText,
) -> (&'static str, SshConnectionListTone) {
    match state.unwrap_or(ConnectionState::Disconnected) {
        ConnectionState::Connected => (
            text.get(UiTextKey::SshConnected),
            SshConnectionListTone::Success,
        ),
        ConnectionState::Failed => (
            text.get(UiTextKey::SshFailed),
            SshConnectionListTone::Danger,
        ),
        ConnectionState::Connecting
        | ConnectionState::VerifyingHostKey
        | ConnectionState::Authenticating
        | ConnectionState::Reconnecting => (
            ssh_connection_state_text(state.unwrap_or(ConnectionState::Disconnected), text),
            SshConnectionListTone::Warning,
        ),
        ConnectionState::Disconnected => (
            text.get(UiTextKey::SshDisconnected),
            SshConnectionListTone::Neutral,
        ),
    }
}

pub(super) fn ssh_connections_overlay(
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
    let connection_list = root.ssh_manager_connection_list(window, cx);
    let selected_id = root
        .ssh
        .form
        .as_ref()
        .map(|form| form.connection_id.clone());
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
    let selected_id_for_connect = selected_id.clone();
    let selected_id_for_disconnect = selected_id.clone();
    let selected_id_for_delete = selected_id;

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
            .child(
                Switch::new("ssh-remember-password")
                    .label(root.ui_text.get(UiTextKey::SshRememberPassword))
                    .checked(remember_password)
                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                        if let Some(form) = this.ssh.form.as_mut() {
                            form.remember_password = *checked;
                        }
                        cx.notify();
                    })),
            );
    }
    if let Some(error) = root.ssh.error.clone() {
        let title = if error.to_ascii_lowercase().contains("host key")
            || error.contains("HOST IDENTIFICATION HAS CHANGED")
        {
            UiTextKey::SshHostKeyTitle
        } else {
            UiTextKey::SshFailed
        };
        form_fields = form_fields
            .child(Alert::error("ssh-connection-error", error).title(root.ui_text.get(title)));
    }

    capture_overlay_input(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(ui_style.spacing.overlay_top)
            .bg(dialog.overlay)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(920.0))
                    .h(px(680.0))
                    .max_h(px(680.0))
                    .rounded(dialog.radius)
                    .border(dialog.border_width)
                    .border_color(dialog.border)
                    .bg(dialog.background)
                    .when(dialog.shadow, |panel| panel.shadow_lg())
                    .p(dialog.padding)
                    .text_color(dialog.text)
                    .child(yttt_dialog_header(
                        "close-ssh-connections",
                        root.ui_text.get(UiTextKey::SshConnections),
                        theme,
                        ui_style,
                        cx.listener(|this, _, _window, cx| {
                            this.close_ssh_connection_manager();
                            cx.notify();
                        }),
                    ))
                    .child(
                        div()
                            .mt(ui_style.spacing.xs)
                            .text_xs()
                            .text_color(dialog.hint)
                            .child(root.ui_text.get(UiTextKey::SshConnectionsDescription)),
                    )
                    .child(
                        div()
                            .mt(ui_style.spacing.lg)
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .gap(ui_style.spacing.lg)
                            .child(
                                div()
                                    .w(px(285.0))
                                    .min_h_0()
                                    .flex()
                                    .flex_col()
                                    .gap(ui_style.spacing.md)
                                    .pr(ui_style.spacing.md)
                                    .border_r_1()
                                    .border_color(dialog.border)
                                    .child(
                                        div()
                                            .min_h_0()
                                            .flex_1()
                                            .child(List::new(&connection_list).size_full()),
                                    )
                                    .child(yttt_dialog_button(
                                        cx,
                                        "new-ssh-connection",
                                        root.ui_text.get(UiTextKey::SshNewConnection),
                                        YtttButtonVariant::Secondary,
                                        theme,
                                        cx.listener(|this, _, _window, cx| {
                                            this.new_ssh_connection_form();
                                            cx.notify();
                                        }),
                                    )),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .min_h_0()
                                    .flex_1()
                                    .overflow_y_scrollbar()
                                    .pr(ui_style.spacing.sm)
                                    .child(form_fields),
                            ),
                    )
                    .child(
                        div()
                            .mt(ui_style.spacing.lg)
                            .pt(ui_style.spacing.md)
                            .border_t_1()
                            .border_color(dialog.border)
                            .flex()
                            .justify_between()
                            .gap(ui_style.spacing.md)
                            .child(yttt_dialog_button(
                                cx,
                                "delete-ssh-connection",
                                root.ui_text.get(UiTextKey::SshDeleteConnection),
                                YtttButtonVariant::Danger,
                                theme,
                                cx.listener(move |this, _, window, cx| {
                                    if let Some(connection_id) = selected_id_for_delete.clone() {
                                        this.delete_ssh_connection(connection_id, window, cx);
                                    }
                                }),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .gap(ui_style.spacing.md)
                                    .child(yttt_dialog_button(
                                        cx,
                                        "save-ssh-connection",
                                        root.ui_text.get(UiTextKey::SettingsSave),
                                        YtttButtonVariant::Secondary,
                                        theme,
                                        cx.listener(|this, _, _window, cx| {
                                            this.save_ssh_connection(cx);
                                            cx.notify();
                                        }),
                                    ))
                                    .child(yttt_dialog_button(
                                        cx,
                                        "disconnect-ssh-connection",
                                        root.ui_text.get(UiTextKey::SshDisconnect),
                                        YtttButtonVariant::Secondary,
                                        theme,
                                        cx.listener(move |this, _, window, cx| {
                                            if let Some(connection_id) =
                                                selected_id_for_disconnect.clone()
                                            {
                                                this.disconnect_ssh_connection(
                                                    connection_id,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                    ))
                                    .child(yttt_dialog_button(
                                        cx,
                                        "connect-ssh-connection",
                                        root.ui_text.get(UiTextKey::SshConnect),
                                        YtttButtonVariant::Primary,
                                        theme,
                                        cx.listener(move |this, _, window, cx| {
                                            if let Some(connection_id) =
                                                selected_id_for_connect.clone()
                                            {
                                                this.connect_ssh_connection(
                                                    connection_id,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                    )),
                            ),
                    ),
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
    capture_overlay_input(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(ui_style.spacing.overlay_top)
            .bg(dialog.overlay)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(ui_style.spacing.lg)
                    .w(dialog.max_width)
                    .rounded(dialog.radius)
                    .border(dialog.border_width)
                    .border_color(dialog.border)
                    .bg(dialog.background)
                    .p(dialog.padding)
                    .text_color(dialog.text)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(if host_key_changed {
                        Alert::warning("ssh-host-key-changed-warning", description)
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
                                    .child(div().text_xs().text_color(dialog.hint).child(
                                        root.ui_text.get(UiTextKey::SshHostKeySavedFingerprint),
                                    ))
                                    .child(
                                        div().font_family("monospace").text_sm().child(fingerprint),
                                    ),
                            )
                        },
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(ui_style.spacing.xs)
                            .child(
                                div().text_xs().text_color(dialog.hint).child(
                                    root.ui_text.get(UiTextKey::SshHostKeyReceivedFingerprint),
                                ),
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
            ),
    )
}

pub(super) fn stored_credential_from_ref(credential: &CredentialRef) -> StoredCredential {
    StoredCredential {
        id: credential.id.clone(),
        effective_user: credential.binding.effective_user.clone(),
        resolved_host: credential.binding.resolved_host.clone(),
        port: credential.binding.port,
        host_key_sha256: credential.binding.host_key_sha256.clone(),
        private_key_identity: credential.binding.private_key_identity.clone(),
    }
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
