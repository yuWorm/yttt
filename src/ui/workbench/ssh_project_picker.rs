use std::path::Path;

use crate::config::ssh::{SshAuthPreference, SshConnectionConfig};
use crate::ui::theme::icons::icon_for_visual;
use gpui_component::{
    alert::Alert,
    list::{List, ListEvent, ListState},
    radio::RadioGroup,
    switch::Switch,
};

use yttt_core::model::{
    ids::{ConnectionId, CredentialId},
    project::{ProjectLocation, RemotePathBuf, RemoteRelativePathBuf},
};
use yttt_ssh::{
    Authentication, ConnectRequest, ConnectionState, ConnectionStatus, HostKeyDecision,
    RemoteEntryKind, SshEndpoint, TransportError,
};
use zeroize::Zeroizing;

use super::{
    ssh_connections::{
        ssh_connection_state_text, ssh_connection_status, ssh_form_field,
        stored_credential_from_ref,
    },
    *,
};

struct SshPasswordAttempt {
    secret: Zeroizing<String>,
    save_as: Option<CredentialId>,
}
const SSH_PROJECT_DIRECTORY_SCROLL_ROW_LIMIT: usize = 8;

#[derive(Debug)]
enum SshProjectAuthenticationError {
    PasswordRequired,
    Other(String),
}

impl WorkbenchView {
    pub fn open_ssh_project_picker(&mut self) {
        self.close_palette();
        self.ssh.manager_open = false;
        self.ssh.form = None;
        self.ssh.project_picker.reset();
        self.ssh.project_picker.open = true;
        self.sync_input_owner_state();
    }

    pub fn close_ssh_project_picker(&mut self, cx: &mut Context<Self>) {
        self.cancel_pending_ssh_project_connection(cx);
        self.ssh.project_picker.reset();
        self.ssh.form = None;
        self.sync_input_owner_state();
    }

    fn cancel_pending_ssh_project_connection(&mut self, cx: &mut Context<Self>) {
        if self.ssh.project_picker.view != SshProjectPickerView::Connecting {
            return;
        }
        let (Some(connection_id), Some(epoch)) = (
            self.ssh.project_picker.connection_id.clone(),
            self.ssh.project_picker.connection_epoch,
        ) else {
            return;
        };
        let pending = std::mem::take(&mut self.ssh.pending_host_keys);
        self.ssh.pending_host_keys = pending
            .into_iter()
            .filter_map(|challenge| {
                if challenge.connection_id == connection_id && challenge.epoch == epoch {
                    let _ = challenge.respond(HostKeyDecision {
                        accept: false,
                        remember: false,
                    });
                    None
                } else {
                    Some(challenge)
                }
            })
            .collect();
        if let Some(transport) = self.ssh.transport.clone() {
            cx.background_spawn(async move {
                let _ = transport.disconnect_attempt(connection_id, epoch).await;
            })
            .detach();
        }
    }

    pub fn new_ssh_project_connection(&mut self) {
        self.new_ssh_connection_form();
        self.ssh.project_picker.view = SshProjectPickerView::QuickConnect;
        self.ssh.project_picker.connection_id = self
            .ssh
            .form
            .as_ref()
            .map(|form| form.connection_id.clone());
        self.ssh.project_picker.continuation =
            Some(SshProjectConnectContinuation::Browse { initial_root: None });
        self.ssh.project_picker.error = None;
        self.sync_input_owner_state();
    }

    pub(super) fn select_ssh_project_connection(
        &mut self,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        let initial_root = self
            .ssh
            .connections
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
            .and_then(|connection| connection.default_remote_root.clone());
        self.begin_ssh_project_connection(
            connection_id,
            SshProjectConnectContinuation::Browse { initial_root },
            None,
            None,
            cx,
        );
    }

    pub(super) fn open_recent_ssh_project(
        &mut self,
        connection_id: ConnectionId,
        root: RemotePathBuf,
        cx: &mut Context<Self>,
    ) {
        self.close_palette();
        self.ssh.project_picker.reset();
        self.ssh.project_picker.open = true;
        self.begin_ssh_project_connection(
            connection_id,
            SshProjectConnectContinuation::OpenRecent { root },
            None,
            None,
            cx,
        );
    }

    pub(super) fn connect_ssh_project_form(&mut self, cx: &mut Context<Self>) {
        let (password, key_passphrase) = self
            .ssh
            .form
            .as_ref()
            .and_then(|form| {
                let inputs = form.inputs.as_ref()?;
                let secret = inputs.password.read(cx).value().to_string();
                let password = (!secret.is_empty()).then(|| SshPasswordAttempt {
                    secret: Zeroizing::new(secret),
                    save_as: form.remember_password.then(|| form.credential_id.clone()),
                });
                Some((password, inputs.key_passphrase.read(cx).value().to_string()))
            })
            .unwrap_or_default();
        let Some(connection_id) = self.save_ssh_connection_from_form(true, false, cx) else {
            self.ssh.project_picker.error = self.ssh.error.take();
            cx.notify();
            return;
        };
        let continuation = self
            .ssh
            .project_picker
            .continuation
            .clone()
            .unwrap_or(SshProjectConnectContinuation::Browse { initial_root: None });
        self.begin_ssh_project_connection(
            connection_id,
            continuation,
            password,
            (!key_passphrase.is_empty()).then_some(key_passphrase),
            cx,
        );
    }

    fn begin_ssh_project_connection(
        &mut self,
        connection_id: ConnectionId,
        continuation: SshProjectConnectContinuation,
        password: Option<SshPasswordAttempt>,
        key_passphrase: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self
            .ssh
            .connections
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
            .cloned()
        else {
            self.ssh.project_picker.open = true;
            self.ssh.project_picker.view = SshProjectPickerView::Connections;
            self.ssh.project_picker.error = Some(
                self.ui_text
                    .get(UiTextKey::SshProjectConnectionMissing)
                    .to_string(),
            );
            self.sync_input_owner_state();
            cx.notify();
            return;
        };

        if let Some(status) = self
            .ssh
            .statuses
            .get(&connection_id)
            .filter(|status| status.state == ConnectionState::Connected)
            .cloned()
        {
            self.ssh.project_picker.connection_id = Some(connection_id);
            self.ssh.project_picker.connection_epoch = Some(status.epoch);
            self.ssh.project_picker.continuation = Some(continuation);
            self.continue_ssh_project_after_connection(cx);
            return;
        }

        let password_was_attempted = password.is_some() || connection.credential.is_some();
        let password_retry_allowed = matches!(
            connection.auth,
            SshAuthPreference::Auto | SshAuthPreference::Password
        );
        let authentication = match ssh_project_authentication(&connection, password, key_passphrase)
        {
            Ok(authentication) => authentication,
            Err(SshProjectAuthenticationError::PasswordRequired) => {
                self.show_ssh_password_prompt(connection_id, continuation, None, cx);
                cx.notify();
                return;
            }
            Err(SshProjectAuthenticationError::Other(message)) => {
                self.ssh.form = Some(SshConnectionForm::new(connection));
                self.ssh.project_picker.open = true;
                self.ssh.project_picker.view = SshProjectPickerView::QuickConnect;
                self.ssh.project_picker.connection_id = Some(connection_id);
                self.ssh.project_picker.continuation = Some(continuation);
                self.ssh.project_picker.error = Some(message);
                self.sync_input_owner_state();
                cx.notify();
                return;
            }
        };
        let Some(transport) = self.ssh.transport.clone() else {
            self.ssh.project_picker.error = Some(
                self.ui_text
                    .get(UiTextKey::SshRuntimeUnavailable)
                    .to_string(),
            );
            cx.notify();
            return;
        };

        self.ssh.error = None;
        self.ssh.project_picker.connection_generation = self
            .ssh
            .project_picker
            .connection_generation
            .wrapping_add(1);
        let connection_generation = self.ssh.project_picker.connection_generation;
        let retry_continuation = continuation.clone();
        self.ssh.project_picker.open = true;
        self.ssh.project_picker.view = SshProjectPickerView::Connecting;
        self.ssh.project_picker.connection_id = Some(connection_id.clone());
        self.ssh.project_picker.connection_epoch = None;
        self.ssh.project_picker.continuation = Some(continuation);
        self.ssh.project_picker.error = None;
        self.sync_input_owner_state();

        let request_id = connection_id.clone();
        cx.spawn(async move |this, cx| {
            let attempt = match transport
                .start_connect(ConnectRequest {
                    connection_id: request_id.clone(),
                    endpoint: SshEndpoint {
                        host: connection.host,
                        port: connection.port,
                        user: connection.user,
                    },
                    authentication,
                    reconnect: false,
                })
                .await
            {
                Ok(attempt) => attempt,
                Err(error) => {
                    let _ = this.update(cx, |root, cx| {
                        if root.ssh.project_picker.open
                            && root.ssh.project_picker.connection_id.as_ref() == Some(&request_id)
                            && root.ssh.project_picker.connection_generation
                                == connection_generation
                        {
                            root.ssh.project_picker.view = SshProjectPickerView::Connecting;
                            root.ssh.project_picker.error = Some(error.to_string());
                        }
                        cx.notify();
                    });
                    return;
                }
            };
            let epoch = attempt.epoch();
            let active = this
                .update(cx, |root, cx| {
                    if !root.ssh.project_picker.open
                        || root.ssh.project_picker.connection_id.as_ref() != Some(&request_id)
                        || root.ssh.project_picker.connection_generation != connection_generation
                    {
                        return false;
                    }
                    root.ssh.project_picker.connection_epoch = Some(epoch);
                    root.reject_stale_ssh_host_key_challenges(&request_id, epoch);
                    if let Some(status) = root.ssh.statuses.get(&request_id).cloned() {
                        root.apply_ssh_project_connection_status(&status, cx);
                    }
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !active {
                let _ = transport
                    .disconnect_attempt(request_id.clone(), epoch)
                    .await;
                return;
            }

            let result = attempt.wait().await;
            let _ = this.update(cx, |root, cx| {
                if let Err(error) = result
                    && root.ssh.project_picker.open
                    && root.ssh.project_picker.connection_id.as_ref() == Some(&request_id)
                    && root.ssh.project_picker.connection_generation == connection_generation
                    && root.ssh.project_picker.connection_epoch == Some(epoch)
                {
                    let prompt_message = password_retry_allowed
                        .then(|| {
                            ssh_password_prompt_message(
                                &error,
                                password_was_attempted,
                                &root.ui_text,
                            )
                        })
                        .flatten();
                    if let Some(message) = prompt_message {
                        root.show_ssh_password_prompt(
                            request_id.clone(),
                            retry_continuation.clone(),
                            message,
                            cx,
                        );
                    } else {
                        root.ssh.project_picker.view = SshProjectPickerView::Connecting;
                        root.ssh.project_picker.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn show_ssh_password_prompt(
        &mut self,
        connection_id: ConnectionId,
        continuation: SshProjectConnectContinuation,
        error: Option<String>,
        _cx: &mut Context<Self>,
    ) {
        self.ssh.form = None;
        self.ssh.project_picker.open = true;
        self.ssh.project_picker.view = SshProjectPickerView::Password;
        self.ssh.project_picker.connection_id = Some(connection_id);
        self.ssh.project_picker.connection_epoch = None;
        self.ssh.project_picker.continuation = Some(continuation);
        self.ssh.project_picker.error = error;
        self.ssh.project_picker.password_input = None;
        self.ssh.project_picker.password_input_subscription = None;
        self.ssh.project_picker.password_input_needs_focus = true;
        self.ssh.project_picker.remember_password = true;
        self.sync_input_owner_state();
    }

    pub(super) fn apply_ssh_project_connection_status(
        &mut self,
        status: &ConnectionStatus,
        cx: &mut Context<Self>,
    ) {
        if !self.ssh.project_picker.open
            || self.ssh.project_picker.connection_id.as_ref() != Some(&status.connection_id)
            || self.ssh.project_picker.connection_epoch != Some(status.epoch)
        {
            return;
        }
        match status.state {
            ConnectionState::Connected => {
                self.ssh.project_picker.error = None;
                if !matches!(
                    self.ssh.project_picker.view,
                    SshProjectPickerView::Browsing | SshProjectPickerView::Opening
                ) {
                    self.continue_ssh_project_after_connection(cx);
                }
            }
            ConnectionState::Failed | ConnectionState::Disconnected => {
                if self.ssh.project_picker.view == SshProjectPickerView::Browsing {
                    self.ssh.project_picker.continuation =
                        Some(SshProjectConnectContinuation::Browse {
                            initial_root: self.ssh.project_picker.current_path.clone(),
                        });
                }
                self.ssh.project_picker.view = SshProjectPickerView::Connecting;
                self.ssh.project_picker.error = status.error.clone().or_else(|| {
                    Some(
                        self.ui_text
                            .get(UiTextKey::SshProjectConnectionFailed)
                            .to_string(),
                    )
                });
            }
            ConnectionState::Connecting
            | ConnectionState::VerifyingHostKey
            | ConnectionState::Authenticating
            | ConnectionState::Reconnecting => {
                if self.ssh.project_picker.view == SshProjectPickerView::Browsing {
                    self.ssh.project_picker.continuation =
                        Some(SshProjectConnectContinuation::Browse {
                            initial_root: self.ssh.project_picker.current_path.clone(),
                        });
                }
                self.ssh.project_picker.view = SshProjectPickerView::Connecting;
            }
        }
    }

    fn continue_ssh_project_after_connection(&mut self, cx: &mut Context<Self>) {
        let Some(connection_id) = self.ssh.project_picker.connection_id.clone() else {
            return;
        };
        let Some(continuation) = self.ssh.project_picker.continuation.clone() else {
            return;
        };
        match continuation {
            SshProjectConnectContinuation::Browse { initial_root } => {
                self.ssh.form = None;
                self.resolve_ssh_project_browser_root(connection_id, initial_root, cx);
            }
            SshProjectConnectContinuation::OpenRecent { root } => {
                self.validate_and_open_recent_ssh_project(connection_id, root, cx);
            }
        }
        cx.notify();
    }

    fn validate_and_open_recent_ssh_project(
        &mut self,
        connection_id: ConnectionId,
        root: RemotePathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(transport) = self.ssh.transport.clone() else {
            self.ssh.project_picker.error = Some(
                self.ui_text
                    .get(UiTextKey::SshRuntimeUnavailable)
                    .to_string(),
            );
            return;
        };
        let Some(epoch) = self.ssh.project_picker.connection_epoch else {
            return;
        };
        let sftp = transport.sftp_project(connection_id.clone(), root.clone());
        self.ssh.project_picker.view = SshProjectPickerView::Opening;
        self.ssh.project_picker.loading = true;
        self.ssh.project_picker.error = None;
        self.ssh.project_picker.generation = self.ssh.project_picker.generation.wrapping_add(1);
        let generation = self.ssh.project_picker.generation;
        let task =
            cx.background_spawn(
                async move { sftp.scan_directory(RemoteRelativePathBuf::root(), true) },
            );
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |workbench, cx| {
                if !workbench.ssh.project_picker.open
                    || workbench.ssh.project_picker.generation != generation
                    || workbench.ssh.project_picker.connection_id.as_ref() != Some(&connection_id)
                    || workbench.ssh.project_picker.connection_epoch != Some(epoch)
                {
                    return;
                }
                workbench.ssh.project_picker.loading = false;
                match result {
                    Ok(_) => match workbench.open_ssh_project_location(connection_id, root, true) {
                        Ok(()) => {
                            workbench.ssh.project_picker.reset();
                            workbench.ssh.form = None;
                            if let Some(project_id) =
                                workbench.workspace.selected_project_id().cloned()
                            {
                                workbench.refresh_project_git_status(project_id, cx);
                            }
                            workbench.sync_input_owner_state();
                        }
                        Err(error) => {
                            workbench.ssh.project_picker.error = Some(error.to_string());
                        }
                    },
                    Err(error) => {
                        workbench.ssh.project_picker.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn resolve_ssh_project_browser_root(
        &mut self,
        connection_id: ConnectionId,
        initial_root: Option<RemotePathBuf>,
        cx: &mut Context<Self>,
    ) {
        if let Some(root) = initial_root {
            self.load_ssh_project_directory(connection_id, root, cx);
            return;
        }
        let Some(transport) = self.ssh.transport.clone() else {
            self.ssh.project_picker.error = Some(
                self.ui_text
                    .get(UiTextKey::SshRuntimeUnavailable)
                    .to_string(),
            );
            return;
        };
        let root = RemotePathBuf::new("/").expect("remote filesystem root is valid");
        let sftp = transport.sftp_project(connection_id.clone(), root);
        self.ssh.project_picker.view = SshProjectPickerView::Browsing;
        self.ssh.project_picker.loading = true;
        self.ssh.project_picker.error = None;
        self.ssh.project_picker.generation = self.ssh.project_picker.generation.wrapping_add(1);
        let generation = self.ssh.project_picker.generation;
        let task = cx.background_spawn(async move { sftp.resolve_home() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                if !root.ssh.project_picker.open
                    || root.ssh.project_picker.generation != generation
                    || root.ssh.project_picker.connection_id.as_ref() != Some(&connection_id)
                {
                    return;
                }
                match result {
                    Ok(home) => root.load_ssh_project_directory(connection_id, home, cx),
                    Err(error) => {
                        root.ssh.project_picker.loading = false;
                        root.ssh.project_picker.error = Some(error.to_string());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn load_ssh_project_directory(
        &mut self,
        connection_id: ConnectionId,
        path: RemotePathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(transport) = self.ssh.transport.clone() else {
            self.ssh.project_picker.error = Some(
                self.ui_text
                    .get(UiTextKey::SshRuntimeUnavailable)
                    .to_string(),
            );
            return;
        };
        let sftp = transport.sftp_project(connection_id.clone(), path.clone());
        self.ssh.project_picker.open = true;
        self.ssh.project_picker.view = SshProjectPickerView::Browsing;
        self.ssh.project_picker.connection_id = Some(connection_id.clone());
        self.ssh.project_picker.current_path = Some(path.clone());
        self.ssh.project_picker.continuation = Some(SshProjectConnectContinuation::Browse {
            initial_root: Some(path.clone()),
        });
        self.ssh.project_picker.directories.clear();
        self.ssh.project_picker.loading = true;
        self.ssh.project_picker.error = None;
        self.ssh.project_picker.path_input = None;
        self.ssh.project_picker.path_input_subscription = None;
        self.ssh.project_picker.generation = self.ssh.project_picker.generation.wrapping_add(1);
        let generation = self.ssh.project_picker.generation;
        let task =
            cx.background_spawn(
                async move { sftp.scan_directory(RemoteRelativePathBuf::root(), true) },
            );
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                if !root.ssh.project_picker.open
                    || root.ssh.project_picker.generation != generation
                    || root.ssh.project_picker.connection_id.as_ref() != Some(&connection_id)
                    || root.ssh.project_picker.current_path.as_ref() != Some(&path)
                {
                    return;
                }
                root.ssh.project_picker.loading = false;
                match result {
                    Ok(snapshot) => {
                        root.ssh.project_picker.directories = snapshot
                            .entries
                            .into_iter()
                            .filter(|entry| entry.kind == RemoteEntryKind::Directory)
                            .filter_map(|entry| {
                                remote_child_path(&path, &entry.name).map(|child| {
                                    SshProjectDirectory {
                                        name: entry.name,
                                        path: child,
                                    }
                                })
                            })
                            .collect();
                        root.ssh.project_picker.error = None;
                    }
                    Err(error) => {
                        root.ssh.project_picker.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn navigate_ssh_project_directory(
        &mut self,
        path: RemotePathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self.ssh.project_picker.connection_id.clone() else {
            return;
        };
        self.load_ssh_project_directory(connection_id, path, cx);
    }

    pub(super) fn navigate_ssh_project_parent(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.ssh.project_picker.current_path.as_ref() else {
            return;
        };
        if let Some(parent) = remote_parent_path(path) {
            self.navigate_ssh_project_directory(parent, cx);
        }
    }

    pub(super) fn navigate_ssh_project_path_input(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.ssh.project_picker.path_input.as_ref() else {
            return;
        };
        let value = input.read(cx).value().trim().to_string();
        match RemotePathBuf::new(value) {
            Ok(path) => self.navigate_ssh_project_directory(path, cx),
            Err(error) => {
                self.ssh.project_picker.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    pub(super) fn retry_ssh_project_picker(&mut self, cx: &mut Context<Self>) {
        match self.ssh.project_picker.view {
            SshProjectPickerView::Connecting => {
                let Some(connection_id) = self.ssh.project_picker.connection_id.clone() else {
                    return;
                };
                let Some(continuation) = self.ssh.project_picker.continuation.clone() else {
                    return;
                };
                let (password, key_passphrase) = self
                    .ssh
                    .form
                    .as_ref()
                    .and_then(|form| {
                        let inputs = form.inputs.as_ref()?;
                        let secret = inputs.password.read(cx).value().to_string();
                        let password = (!secret.is_empty()).then(|| SshPasswordAttempt {
                            secret: Zeroizing::new(secret),
                            save_as: form.remember_password.then(|| form.credential_id.clone()),
                        });
                        Some((password, inputs.key_passphrase.read(cx).value().to_string()))
                    })
                    .unwrap_or_default();
                self.begin_ssh_project_connection(
                    connection_id,
                    continuation,
                    password,
                    (!key_passphrase.is_empty()).then_some(key_passphrase),
                    cx,
                );
            }
            SshProjectPickerView::Opening => {
                if let (
                    Some(connection_id),
                    Some(SshProjectConnectContinuation::OpenRecent { root }),
                ) = (
                    self.ssh.project_picker.connection_id.clone(),
                    self.ssh.project_picker.continuation.clone(),
                ) {
                    self.validate_and_open_recent_ssh_project(connection_id, root, cx);
                }
            }
            SshProjectPickerView::Browsing => {
                if let (Some(connection_id), Some(path)) = (
                    self.ssh.project_picker.connection_id.clone(),
                    self.ssh.project_picker.current_path.clone(),
                ) {
                    self.load_ssh_project_directory(connection_id, path, cx);
                } else if let (
                    Some(connection_id),
                    Some(SshProjectConnectContinuation::Browse { initial_root }),
                ) = (
                    self.ssh.project_picker.connection_id.clone(),
                    self.ssh.project_picker.continuation.clone(),
                ) {
                    self.resolve_ssh_project_browser_root(connection_id, initial_root, cx);
                }
            }
            SshProjectPickerView::Connections
            | SshProjectPickerView::QuickConnect
            | SshProjectPickerView::Password => {}
        }
    }

    pub(super) fn edit_ssh_project_credentials(&mut self) {
        let Some(connection_id) = self.ssh.project_picker.connection_id.as_ref() else {
            return;
        };
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
        self.ssh.project_picker.view = SshProjectPickerView::QuickConnect;
        self.ssh.project_picker.error = None;
        self.sync_input_owner_state();
    }

    pub(super) fn back_ssh_project_picker(&mut self, cx: &mut Context<Self>) {
        self.cancel_pending_ssh_project_connection(cx);
        self.ssh.form = None;
        self.ssh.project_picker.view = SshProjectPickerView::Connections;
        self.ssh.project_picker.connection_id = None;
        self.ssh.project_picker.connection_epoch = None;
        self.ssh.project_picker.continuation = None;
        self.ssh.project_picker.current_path = None;
        self.ssh.project_picker.directories.clear();
        self.ssh.project_picker.loading = false;
        self.ssh.project_picker.error = None;
        self.ssh.project_picker.path_input = None;
        self.ssh.project_picker.path_input_subscription = None;
        self.ssh.project_picker.password_input = None;
        self.ssh.project_picker.password_input_subscription = None;
        self.ssh.project_picker.password_input_needs_focus = false;
        self.ssh.project_picker.remember_password = false;
    }

    pub(super) fn open_current_ssh_project_directory(&mut self, cx: &mut Context<Self>) {
        if self.ssh.project_picker.loading || self.ssh.project_picker.error.is_some() {
            return;
        }
        let (Some(connection_id), Some(root)) = (
            self.ssh.project_picker.connection_id.clone(),
            self.ssh.project_picker.current_path.clone(),
        ) else {
            return;
        };
        match self.open_ssh_project_location(connection_id, root, true) {
            Ok(()) => {
                self.ssh.project_picker.reset();
                self.ssh.form = None;
                if let Some(project_id) = self.workspace.selected_project_id().cloned() {
                    self.refresh_project_git_status(project_id, cx);
                }
                self.sync_input_owner_state();
            }
            Err(error) => {
                self.ssh.project_picker.error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    pub(super) fn ssh_project_path_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        if self.ssh.project_picker.view != SshProjectPickerView::Browsing {
            return None;
        }
        if let Some(input) = &self.ssh.project_picker.path_input {
            return Some(input.clone());
        }
        let value = self
            .ssh
            .project_picker
            .current_path
            .as_ref()
            .map(RemotePathBuf::as_str)
            .unwrap_or("/")
            .to_string();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(self.ui_text.get(UiTextKey::SshProjectPath))
                .default_value(value)
        });
        let subscription = cx.subscribe_in(&input, window, Self::on_ssh_project_path_input_event);
        self.ssh.project_picker.path_input = Some(input.clone());
        self.ssh.project_picker.path_input_subscription = Some(subscription);
        Some(input)
    }

    fn on_ssh_project_path_input_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.navigate_ssh_project_path_input(cx);
        }
    }
    pub(super) fn ssh_project_password_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        if self.ssh.project_picker.view != SshProjectPickerView::Password {
            return None;
        }
        if let Some(input) = self.ssh.project_picker.password_input.clone() {
            if self.ssh.project_picker.password_input_needs_focus {
                input.update(cx, |input, cx| input.focus(window, cx));
                self.ssh.project_picker.password_input_needs_focus = false;
            }
            return Some(input);
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(self.ui_text.get(UiTextKey::SshPassword))
                .masked(true)
        });
        let subscription =
            cx.subscribe_in(&input, window, Self::on_ssh_project_password_input_event);
        input.update(cx, |input, cx| input.focus(window, cx));
        self.ssh.project_picker.password_input = Some(input.clone());
        self.ssh.project_picker.password_input_subscription = Some(subscription);
        self.ssh.project_picker.password_input_needs_focus = false;
        Some(input)
    }

    fn on_ssh_project_password_input_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.submit_ssh_project_password(cx);
        }
    }

    pub(super) fn submit_ssh_project_password(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.ssh.project_picker.password_input.as_ref() else {
            return;
        };
        let secret = input.read(cx).value().to_string();
        if secret.is_empty() {
            self.ssh.project_picker.error =
                Some(self.ui_text.get(UiTextKey::SshPasswordRequired).to_string());
            cx.notify();
            return;
        }
        let (Some(connection_id), Some(continuation)) = (
            self.ssh.project_picker.connection_id.clone(),
            self.ssh.project_picker.continuation.clone(),
        ) else {
            return;
        };
        let save_as = self.ssh.project_picker.remember_password.then(|| {
            self.ssh
                .connections
                .connections
                .iter()
                .find(|connection| connection.id == connection_id)
                .and_then(|connection| {
                    connection
                        .credential
                        .as_ref()
                        .map(|credential| credential.id.clone())
                })
                .unwrap_or_else(CredentialId::random)
        });
        self.ssh.project_picker.password_input = None;
        self.ssh.project_picker.password_input_subscription = None;
        self.ssh.project_picker.password_input_needs_focus = false;
        self.ssh.project_picker.error = None;
        self.begin_ssh_project_connection(
            connection_id,
            continuation,
            Some(SshPasswordAttempt {
                secret: Zeroizing::new(secret),
                save_as,
            }),
            None,
            cx,
        );
    }

    pub(super) fn ssh_project_connection_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ListState<SshConnectionListDelegate>> {
        let recent_entries = self
            .palette
            .recent_projects
            .iter()
            .filter_map(|project| match &project.location {
                ProjectLocation::Ssh {
                    connection_id,
                    root,
                } => {
                    let endpoint = self
                        .ssh
                        .connections
                        .connections
                        .iter()
                        .find(|connection| connection.id == *connection_id)
                        .map(|connection| {
                            format!("{}@{} · {}", connection.user, connection.host, root)
                        })
                        .unwrap_or_else(|| format!("{} · {}", connection_id, root));
                    let (status, tone) = ssh_connection_status(
                        self.ssh
                            .statuses
                            .get(connection_id)
                            .map(|status| status.state),
                        &self.ui_text,
                    );
                    Some(SshConnectionListEntry {
                        action: SshConnectionListAction::OpenRecent {
                            connection_id: connection_id.clone(),
                            root: root.clone(),
                        },
                        title: project.title.clone().into(),
                        subtitle: endpoint.into(),
                        status: status.into(),
                        tone,
                    })
                }
                ProjectLocation::Local { .. } => None,
            })
            .take(5)
            .collect::<Vec<_>>();
        let connection_entries = self
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
                    action: SshConnectionListAction::Open(connection.id.clone()),
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
            .collect::<Vec<_>>();
        let mut sections = Vec::with_capacity(2);
        if !recent_entries.is_empty() {
            sections.push(SshConnectionListSection {
                title: self.ui_text.get(UiTextKey::SshProjectRecent).into(),
                entries: recent_entries,
            });
        }
        sections.push(SshConnectionListSection {
            title: self.ui_text.get(UiTextKey::SshConnections).into(),
            entries: connection_entries,
        });

        if let Some(list) = self.ssh.project_picker.connection_list.clone() {
            list.update(cx, |list, cx| {
                list.delegate_mut().replace_sections(sections);
                cx.notify();
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
                match action {
                    Some(SshConnectionListAction::Open(connection_id)) => {
                        this.select_ssh_project_connection(connection_id, cx);
                    }
                    Some(SshConnectionListAction::OpenRecent {
                        connection_id,
                        root,
                    }) => {
                        this.open_recent_ssh_project(connection_id, root, cx);
                    }
                    Some(SshConnectionListAction::Edit(_)) | None => {}
                }
                cx.notify();
            },
        );
        self.ssh.project_picker.connection_list = Some(list.clone());
        self.ssh.project_picker.connection_list_subscription = Some(subscription);
        list
    }
}

fn ssh_project_authentication(
    connection: &SshConnectionConfig,
    password: Option<SshPasswordAttempt>,
    key_passphrase: Option<String>,
) -> Result<Authentication, SshProjectAuthenticationError> {
    if matches!(
        connection.auth,
        SshAuthPreference::Auto | SshAuthPreference::Password
    ) && let Some(password) = password
    {
        return Ok(Authentication::Password {
            secret: password.secret,
            save_as: password.save_as,
        });
    }
    match connection.auth {
        SshAuthPreference::Auto => Ok(Authentication::Auto {
            identity_file: connection.identity_file.clone(),
            passphrase: key_passphrase.map(Zeroizing::new),
            credential: connection
                .credential
                .as_ref()
                .map(stored_credential_from_ref),
        }),
        SshAuthPreference::Agent => Ok(Authentication::Agent),
        SshAuthPreference::Password => connection
            .credential
            .as_ref()
            .map(stored_credential_from_ref)
            .map(Authentication::StoredPassword)
            .ok_or(SshProjectAuthenticationError::PasswordRequired),
        SshAuthPreference::PublicKey => connection
            .identity_file
            .clone()
            .map(|path| Authentication::PrivateKey {
                path,
                passphrase: key_passphrase.map(Zeroizing::new),
            })
            .ok_or_else(|| {
                SshProjectAuthenticationError::Other(
                    "Private-key authentication requires an identity file.".to_string(),
                )
            }),
    }
}

fn ssh_password_prompt_message(
    error: &TransportError,
    password_was_attempted: bool,
    text: &UiText,
) -> Option<Option<String>> {
    match error {
        TransportError::AuthenticationRejected => Some(
            password_was_attempted.then(|| text.get(UiTextKey::SshPasswordRejected).to_string()),
        ),
        TransportError::CredentialMissing(_) | TransportError::CredentialBindingMismatch(_) => {
            Some(Some(
                text.get(UiTextKey::SshPasswordUnavailable).to_string(),
            ))
        }
        _ => None,
    }
}

fn remote_child_path(parent: &RemotePathBuf, name: &str) -> Option<RemotePathBuf> {
    let path = if parent.as_str() == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", parent.as_str())
    };
    RemotePathBuf::new(path).ok()
}

fn remote_parent_path(path: &RemotePathBuf) -> Option<RemotePathBuf> {
    if path.as_str() == "/" {
        return None;
    }
    let parent = path
        .as_str()
        .rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .unwrap_or("/");
    RemotePathBuf::new(parent).ok()
}

pub(super) fn ssh_project_picker_overlay(
    root: &mut WorkbenchView,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let ui_style = current_ui_style(cx);
    let dialog = yttt_dialog_style(theme, ui_style);
    let content = match root.ssh.project_picker.view {
        SshProjectPickerView::Connections => {
            ssh_project_connections(root, window, theme, ui_style, cx)
        }
        SshProjectPickerView::QuickConnect => {
            ssh_project_quick_connect(root, window, theme, ui_style, cx)
        }
        SshProjectPickerView::Password => {
            ssh_project_password_prompt(root, window, theme, ui_style, cx)
        }
        SshProjectPickerView::Connecting | SshProjectPickerView::Opening => {
            ssh_project_connecting(root, theme, ui_style, cx)
        }
        SshProjectPickerView::Browsing => ssh_project_browser(root, window, theme, ui_style, cx),
    };

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
                    .w(px(680.0))
                    .max_h(px(640.0))
                    .rounded(dialog.radius)
                    .border(dialog.border_width)
                    .border_color(dialog.border)
                    .bg(dialog.background)
                    .when(dialog.shadow, |panel| panel.shadow_lg())
                    .p(dialog.padding)
                    .text_color(dialog.text)
                    .child(content),
            ),
    )
}

fn ssh_project_connections(
    root: &mut WorkbenchView,
    window: &mut Window,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let connection_list = root.ssh_project_connection_list(window, cx);
    let mut body = div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.lg)
        .child(yttt_dialog_header(
            "close-ssh-project-picker",
            root.ui_text.get(UiTextKey::SshOpenRemoteProject),
            theme,
            ui_style,
            cx.listener(|this, _, _window, cx| {
                this.close_ssh_project_picker(cx);
                cx.notify();
            }),
        ))
        .child(
            div()
                .h(px(420.0))
                .min_h_0()
                .rounded(ui_style.radius.control)
                .border_1()
                .border_color(theme.border)
                .child(List::new(&connection_list).size_full()),
        );
    if let Some(error) = root.ssh.project_picker.error.clone() {
        body = body.child(
            Alert::error("ssh-project-connections-error", error)
                .title(root.ui_text.get(UiTextKey::SshProjectConnectionFailed)),
        );
    }
    body.child(
        div()
            .flex()
            .justify_between()
            .gap(ui_style.spacing.md)
            .child(yttt_dialog_button(
                cx,
                "ssh-project-new-connection",
                root.ui_text.get(UiTextKey::SshNewConnection),
                YtttButtonVariant::Secondary,
                theme,
                cx.listener(|this, _, _window, cx| {
                    this.new_ssh_project_connection();
                    cx.notify();
                }),
            ))
            .child(yttt_dialog_button(
                cx,
                "ssh-project-cancel",
                root.ui_text.get(UiTextKey::Cancel),
                YtttButtonVariant::Secondary,
                theme,
                cx.listener(|this, _, _window, cx| {
                    this.close_ssh_project_picker(cx);
                    cx.notify();
                }),
            )),
    )
}

fn ssh_project_quick_connect(
    root: &mut WorkbenchView,
    window: &mut Window,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
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
    let auth_index = match auth {
        SshConnectionFormMode::Auto => 0,
        SshConnectionFormMode::Agent => 1,
        SshConnectionFormMode::Password => 2,
        SshConnectionFormMode::PrivateKey => 3,
    };
    let mut fields = div()
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
                            "parse-ssh-project-command",
                            root.ui_text.get(UiTextKey::SshCommandParse),
                            YtttButtonVariant::Secondary,
                            theme,
                            cx.listener(|this, _, window, cx| {
                                this.apply_ssh_command_from_form(window, cx);
                            }),
                        )),
                ),
        )
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
                    RadioGroup::horizontal("ssh-project-authentication")
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
        fields = fields
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
        fields = fields
            .child(ssh_form_field(
                root.ui_text.get(UiTextKey::SshPassword),
                &inputs.password,
                theme,
                ui_style,
            ))
            .child(
                Switch::new("ssh-project-remember-password")
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
    let error = root
        .ssh
        .project_picker
        .error
        .clone()
        .or_else(|| root.ssh.error.clone());
    if let Some(error) = error {
        fields = fields.child(
            Alert::error("ssh-project-quick-connect-error", error)
                .title(root.ui_text.get(UiTextKey::SshProjectConnectionFailed)),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.lg)
        .child(ssh_project_back_header(root, theme, ui_style, cx))
        .child(fields)
        .child(
            div()
                .flex()
                .justify_end()
                .gap(ui_style.spacing.md)
                .child(yttt_dialog_button(
                    cx,
                    "ssh-project-connect-cancel",
                    root.ui_text.get(UiTextKey::Cancel),
                    YtttButtonVariant::Secondary,
                    theme,
                    cx.listener(|this, _, _window, cx| {
                        this.close_ssh_project_picker(cx);
                        cx.notify();
                    }),
                ))
                .child(yttt_dialog_button(
                    cx,
                    "ssh-project-connect",
                    root.ui_text.get(UiTextKey::SshConnect),
                    YtttButtonVariant::Primary,
                    theme,
                    cx.listener(|this, _, _window, cx| {
                        this.connect_ssh_project_form(cx);
                    }),
                )),
        )
}
fn ssh_project_password_prompt(
    root: &mut WorkbenchView,
    window: &mut Window,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let password_input = root.ssh_project_password_input(window, cx);
    let endpoint = root
        .ssh
        .project_picker
        .connection_id
        .as_ref()
        .and_then(|connection_id| {
            root.ssh
                .connections
                .connections
                .iter()
                .find(|connection| &connection.id == connection_id)
        })
        .map(|connection| {
            format!(
                "{}@{}:{}",
                connection.user, connection.host, connection.port
            )
        })
        .unwrap_or_default();
    let error = root.ssh.project_picker.error.clone();
    let mut body = div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.lg)
        .child(ssh_project_back_header(root, theme, ui_style, cx))
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(root.ui_text.get(UiTextKey::SshPasswordPromptTitle)),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme.text_muted)
                .child(root.ui_text.get(UiTextKey::SshPasswordPromptDescription)),
        )
        .child(div().font_family("monospace").text_sm().child(endpoint));
    if let Some(error) = error {
        body = body.child(Alert::error("ssh-password-prompt-error", error));
    }
    if let Some(input) = password_input {
        body = body.child(ssh_form_field(
            root.ui_text.get(UiTextKey::SshPassword),
            &input,
            theme,
            ui_style,
        ));
    }
    body.child(
        Switch::new("ssh-password-prompt-remember")
            .label(root.ui_text.get(UiTextKey::SshRememberPassword))
            .checked(root.ssh.project_picker.remember_password)
            .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                this.ssh.project_picker.remember_password = *checked;
                cx.notify();
            })),
    )
    .child(
        div()
            .flex()
            .justify_end()
            .gap(ui_style.spacing.md)
            .child(yttt_dialog_button(
                cx,
                "ssh-password-prompt-cancel",
                root.ui_text.get(UiTextKey::Cancel),
                YtttButtonVariant::Secondary,
                theme,
                cx.listener(|this, _, _window, cx| {
                    this.close_ssh_project_picker(cx);
                    cx.notify();
                }),
            ))
            .child(yttt_dialog_button(
                cx,
                "ssh-password-prompt-connect",
                root.ui_text.get(UiTextKey::SshConnect),
                YtttButtonVariant::Primary,
                theme,
                cx.listener(|this, _, _window, cx| {
                    this.submit_ssh_project_password(cx);
                }),
            )),
    )
}

fn ssh_project_connecting(
    root: &WorkbenchView,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let connection = root
        .ssh
        .project_picker
        .connection_id
        .as_ref()
        .and_then(|connection_id| {
            root.ssh
                .connections
                .connections
                .iter()
                .find(|connection| &connection.id == connection_id)
        });
    let title = connection
        .map(|connection| connection.name.clone())
        .unwrap_or_else(|| {
            root.ui_text
                .get(UiTextKey::SshOpenRemoteProject)
                .to_string()
        });
    let endpoint = connection
        .map(|connection| {
            format!(
                "{}@{}:{}",
                connection.user, connection.host, connection.port
            )
        })
        .unwrap_or_default();
    let status = root
        .ssh
        .project_picker
        .connection_id
        .as_ref()
        .and_then(|connection_id| root.ssh.statuses.get(connection_id))
        .map(|status| ssh_connection_state_text(status.state, &root.ui_text))
        .unwrap_or(root.ui_text.get(UiTextKey::SshConnecting));
    let error = root.ssh.project_picker.error.clone();

    let mut body = div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.lg)
        .child(ssh_project_back_header(root, theme, ui_style, cx))
        .child(div().text_lg().child(title))
        .child(div().text_sm().text_color(theme.text_muted).child(endpoint))
        .child(div().text_sm().child(status));
    if let Some(message) = error.clone() {
        let title = if message.to_ascii_lowercase().contains("host key")
            || message.contains("HOST IDENTIFICATION HAS CHANGED")
        {
            UiTextKey::SshHostKeyTitle
        } else {
            UiTextKey::SshProjectConnectionFailed
        };
        body = body.child(
            Alert::error("ssh-project-connection-error", message).title(root.ui_text.get(title)),
        );
    }
    body.child(
        div()
            .flex()
            .justify_end()
            .gap(ui_style.spacing.md)
            .when(error.is_some(), |footer| {
                footer
                    .child(yttt_dialog_button(
                        cx,
                        "ssh-project-edit-credentials",
                        root.ui_text.get(UiTextKey::SshEditConnection),
                        YtttButtonVariant::Secondary,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.edit_ssh_project_credentials();
                            cx.notify();
                        }),
                    ))
                    .child(yttt_dialog_button(
                        cx,
                        "ssh-project-connect-retry",
                        root.ui_text.get(UiTextKey::Retry),
                        YtttButtonVariant::Primary,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.retry_ssh_project_picker(cx);
                        }),
                    ))
            })
            .child(yttt_dialog_button(
                cx,
                "ssh-project-connecting-cancel",
                root.ui_text.get(UiTextKey::Cancel),
                YtttButtonVariant::Secondary,
                theme,
                cx.listener(|this, _, _window, cx| {
                    this.close_ssh_project_picker(cx);
                    cx.notify();
                }),
            )),
    )
}

fn ssh_project_browser(
    root: &mut WorkbenchView,
    window: &mut Window,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let path_input = root.ssh_project_path_input(window, cx);
    let current_path = root.ssh.project_picker.current_path.clone();
    let current_path_label = current_path
        .as_ref()
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let directories = root.ssh.project_picker.directories.clone();
    let loading = root.ssh.project_picker.loading;
    let error = root.ssh.project_picker.error.clone();
    let directory_empty = directories.is_empty();
    let can_open = current_path.is_some() && !loading && error.is_none();
    let has_parent = current_path
        .as_ref()
        .is_some_and(|path| path.as_str() != "/");
    let directory_row_count = directories.len() + usize::from(has_parent);
    let mut list_rows = div()
        .debug_selector(|| "ssh-project-directory-list".to_string())
        .flex()
        .flex_col()
        .gap(ui_style.spacing.xs);
    if has_parent {
        list_rows = list_rows.child(
            Button::new("ssh-project-parent-directory")
                .ghost()
                .w_full()
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(ui_style.spacing.sm)
                        .text_left()
                        .child(icon_for_visual(
                            root.icon_theme.resolve_directory(Path::new(".."), true),
                            theme.text_muted,
                        ))
                        .child(".."),
                )
                .on_click(cx.listener(|this, _, _window, cx| {
                    this.navigate_ssh_project_parent(cx);
                })),
        );
    }
    for directory in directories {
        let path = directory.path.clone();
        let debug_path = directory.path.to_string();
        let icon_debug_path = debug_path.clone();
        let directory_icon = icon_for_visual(
            root.icon_theme
                .resolve_directory(Path::new(directory.path.as_str()), false),
            theme.text_muted,
        );
        let chevron_icon =
            icon_for_visual(root.icon_theme.resolve_chevron(false), theme.text_muted);
        list_rows = list_rows.child(
            Button::new(SharedString::from(format!(
                "ssh-project-directory-{}",
                directory.path
            )))
            .ghost()
            .w_full()
            .child(
                div()
                    .debug_selector(move || format!("ssh-project-directory-content-{debug_path}"))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(ui_style.spacing.md)
                    .text_left()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(ui_style.spacing.sm)
                            .child(
                                div()
                                    .debug_selector(move || {
                                        format!("ssh-project-directory-icon-{icon_debug_path}")
                                    })
                                    .flex_none()
                                    .child(directory_icon),
                            )
                            .child(div().min_w_0().truncate().child(directory.name)),
                    )
                    .child(div().flex_none().child(chevron_icon)),
            )
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.navigate_ssh_project_directory(path.clone(), cx);
            })),
        );
    }
    if loading {
        list_rows = list_rows.child(
            div()
                .p(ui_style.spacing.lg)
                .text_sm()
                .text_color(theme.text_muted)
                .child(root.ui_text.get(UiTextKey::SshProjectLoadingDirectory)),
        );
    } else if directory_empty && error.is_none() {
        list_rows = list_rows.child(
            div()
                .p(ui_style.spacing.lg)
                .text_sm()
                .text_color(theme.text_muted)
                .child(root.ui_text.get(UiTextKey::SshProjectEmptyDirectory)),
        );
    }
    let list = if directory_row_count > SSH_PROJECT_DIRECTORY_SCROLL_ROW_LIMIT {
        list_rows
            .h(px(350.0))
            .overflow_y_scrollbar()
            .into_any_element()
    } else {
        list_rows.into_any_element()
    };

    let mut body = div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.lg)
        .child(ssh_project_back_header(root, theme, ui_style, cx));
    if let Some(input) = path_input {
        body = body.child(
            div()
                .flex()
                .items_end()
                .gap(ui_style.spacing.sm)
                .child(
                    ssh_form_field(
                        root.ui_text.get(UiTextKey::SshProjectPath),
                        &input,
                        theme,
                        ui_style,
                    )
                    .flex_1(),
                )
                .child(yttt_dialog_button(
                    cx,
                    "ssh-project-path-go",
                    root.ui_text.get(UiTextKey::SshProjectGo),
                    YtttButtonVariant::Secondary,
                    theme,
                    cx.listener(|this, _, _window, cx| {
                        this.navigate_ssh_project_path_input(cx);
                    }),
                )),
        );
    }
    body = body.child(list);
    if let Some(message) = error.clone() {
        body = body.child(div().text_xs().text_color(theme.danger).child(message));
    }
    body.child(
        div()
            .flex()
            .justify_between()
            .gap(ui_style.spacing.md)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(current_path_label),
            )
            .child(
                div()
                    .flex()
                    .gap(ui_style.spacing.md)
                    .when(error.is_some(), |footer| {
                        footer.child(yttt_dialog_button(
                            cx,
                            "ssh-project-directory-retry",
                            root.ui_text.get(UiTextKey::Retry),
                            YtttButtonVariant::Secondary,
                            theme,
                            cx.listener(|this, _, _window, cx| {
                                this.retry_ssh_project_picker(cx);
                            }),
                        ))
                    })
                    .child(yttt_dialog_button(
                        cx,
                        "ssh-project-browser-cancel",
                        root.ui_text.get(UiTextKey::Cancel),
                        YtttButtonVariant::Secondary,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.close_ssh_project_picker(cx);
                            cx.notify();
                        }),
                    ))
                    .child(
                        yttt_dialog_button(
                            cx,
                            "ssh-project-open-current",
                            root.ui_text.get(UiTextKey::SshProjectOpenCurrentFolder),
                            YtttButtonVariant::Primary,
                            theme,
                            cx.listener(|this, _, _window, cx| {
                                this.open_current_ssh_project_directory(cx);
                            }),
                        )
                        .disabled(!can_open)
                        .tab_stop(can_open),
                    ),
            ),
    )
}

fn ssh_project_back_header(
    root: &WorkbenchView,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(ui_style.spacing.md)
        .child(
            Button::new("ssh-project-back")
                .ghost()
                .label(format!(
                    "← {}",
                    root.ui_text.get(UiTextKey::SshOpenRemoteProject)
                ))
                .on_click(cx.listener(|this, _, _window, cx| {
                    this.back_ssh_project_picker(cx);
                    cx.notify();
                })),
        )
        .child(
            Button::new("ssh-project-close")
                .ghost()
                .label("×")
                .text_color(theme.text_muted)
                .on_click(cx.listener(|this, _, _window, cx| {
                    this.close_ssh_project_picker(cx);
                    cx.notify();
                })),
        )
}

#[cfg(test)]
mod tests {
    use super::{
        SshPasswordAttempt, remote_child_path, remote_parent_path, ssh_password_prompt_message,
        ssh_project_authentication,
    };
    use crate::{
        config::ssh::{SshAuthPreference, SshConnectionConfig},
        ui::i18n::UiText,
    };
    use yttt_core::model::{ids::CredentialId, project::RemotePathBuf};
    use yttt_ssh::{Authentication, TransportError};
    use zeroize::Zeroizing;

    #[test]
    fn remote_picker_path_navigation_stays_absolute_and_normalized() {
        let root = RemotePathBuf::new("/").unwrap();
        let project = remote_child_path(&root, "project").unwrap();
        assert_eq!(project.as_str(), "/project");
        let nested = remote_child_path(&project, "src").unwrap();
        assert_eq!(nested.as_str(), "/project/src");
        assert_eq!(remote_parent_path(&nested).unwrap().as_str(), "/project");
        assert_eq!(remote_parent_path(&project).unwrap().as_str(), "/");
        assert!(remote_parent_path(&root).is_none());
    }
    #[test]
    fn password_errors_only_reprompt_for_password_recoverable_failures() {
        let text = UiText::english();
        assert_eq!(
            ssh_password_prompt_message(&TransportError::AuthenticationRejected, false, &text,),
            Some(None)
        );
        assert_eq!(
            ssh_password_prompt_message(&TransportError::AuthenticationRejected, true, &text,),
            Some(Some(
                "The server rejected the password. Check it and try again.".to_string()
            ))
        );
        assert_eq!(
            ssh_password_prompt_message(
                &TransportError::CredentialMissing(CredentialId::new("missing")),
                false,
                &text,
            ),
            Some(Some(
                "The saved password is unavailable. Enter it again.".to_string()
            ))
        );
        assert_eq!(
            ssh_password_prompt_message(
                &TransportError::Connection("offline".to_string()),
                true,
                &text,
            ),
            None
        );
    }

    #[test]
    fn auto_authentication_prefers_an_entered_password_and_preserves_save_intent() {
        let mut connection =
            SshConnectionConfig::new("Password Host", "host.example.com", 22, "alice");
        connection.auth = SshAuthPreference::Auto;
        let credential_id = CredentialId::new("password-save");

        let authentication = ssh_project_authentication(
            &connection,
            Some(SshPasswordAttempt {
                secret: Zeroizing::new("secret".to_string()),
                save_as: Some(credential_id.clone()),
            }),
            None,
        )
        .unwrap();

        let Authentication::Password { secret, save_as } = authentication else {
            panic!("entered password must override automatic authentication");
        };
        assert_eq!(secret.as_str(), "secret");
        assert_eq!(save_as, Some(credential_id));
    }
}
