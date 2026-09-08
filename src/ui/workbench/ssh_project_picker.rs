use std::path::Path;

use crate::config::ssh::SshAuthPreference;
use crate::ui::theme::icons::icon_for_visual;
use gpui_component::{
    Icon,
    list::{List, ListEvent, ListState},
    radio::RadioGroup,
};

use yttt_core::model::{
    ids::{ConnectionId, CredentialId},
    project::{ProjectLocation, RemotePathBuf},
};
use yttt_protocol::{
    Request, Response,
    ssh::{RemoteFileEntry, RemoteFileKind, RemoteFileRequest, RemoteFileResponse},
};
use zeroize::Zeroizing;

use super::{
    ssh_connections::{
        disconnect_host_ssh, request_host, ssh_connection_state_text, ssh_connection_status,
        ssh_form_field,
    },
    *,
};

struct SshPasswordAttempt {
    secret: Zeroizing<String>,
    save_as: Option<CredentialId>,
}
const SSH_PROJECT_DIRECTORY_SCROLL_ROW_LIMIT: usize = 8;

impl WorkbenchView {
    pub(super) fn open_remote_host_directory_picker(&mut self, cx: &mut Context<Self>) {
        let Some(environment) = self
            .terminal
            .host_runtime
            .as_ref()
            .and_then(|runtime| runtime.remote_environment())
        else {
            return;
        };
        let home = environment.home.clone();
        self.open_ssh_project_picker();
        self.load_remote_host_directory(home, cx);
    }

    fn create_remote_host_directory(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.ssh.project_picker.path_input.as_ref() else {
            return;
        };
        let path = PathBuf::from(input.read(cx).value().trim());
        if let Err(error) = yttt_protocol::HostPath::from_path(&path) {
            self.ssh.project_picker.error = Some(error.to_string());
            return;
        }
        self.ssh.project_picker.loading = true;
        let task = cx.background_spawn(async move {
            crate::config::storage::create_dir_all(&path)
                .map(|_| path)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                root.ssh.project_picker.loading = false;
                match result {
                    Ok(path) => match yttt_protocol::HostPath::from_path(&path) {
                        Ok(path) => root.load_remote_host_directory(path, cx),
                        Err(error) => root.ssh.project_picker.error = Some(error.to_string()),
                    },
                    Err(error) => root.ssh.project_picker.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_remote_host_directory(
        &mut self,
        path: yttt_protocol::HostPath,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        self.ssh.project_picker.view = SshProjectPickerView::Browsing;
        self.ssh.project_picker.loading = true;
        self.ssh.project_picker.error = None;
        self.ssh.project_picker.path_input = None;
        self.ssh.project_picker.path_input_subscription = None;
        self.ssh.project_picker.generation = self.ssh.project_picker.generation.wrapping_add(1);
        let generation = self.ssh.project_picker.generation;
        let task = cx.background_spawn(async move {
            runtime.workspace_request(yttt_protocol::workspace::WorkspaceRequest::Browse {
                path,
                include_hidden: false,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                if !root.ssh.project_picker.open || root.ssh.project_picker.generation != generation {
                    return;
                }
                root.ssh.project_picker.loading = false;
                match result {
                    Ok(yttt_protocol::workspace::WorkspaceResponse::Directory(directory)) => {
                        let result = directory.path.to_path().map_err(|error| error.to_string())
                            .and_then(|path| RemotePathBuf::new(path.to_string_lossy().into_owned()).map_err(|error| error.to_string()));
                        match result {
                            Ok(path) => root.ssh.project_picker.current_path = Some(path),
                            Err(error) => root.ssh.project_picker.error = Some(error),
                        }
                        root.ssh.project_picker.directories = directory.entries.into_iter()
                            .filter(|entry| entry.kind == yttt_protocol::workspace::WorkspaceDirectoryEntryKind::Directory)
                            .filter_map(|entry| Some(SshProjectDirectory {
                                name: entry.name.to_os_string().to_string_lossy().into_owned(),
                                path: RemotePathBuf::new(entry.path.to_path().ok()?.to_string_lossy().into_owned()).ok()?,
                            })).collect();
                    }
                    Ok(_) => root.ssh.project_picker.error = Some("Unexpected Host directory response".into()),
                    Err(error) => root.ssh.project_picker.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

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
        for challenge in pending {
            if challenge.connection_id == connection_id && challenge.epoch == epoch {
                self.send_ssh_host_key_answer(challenge, false, false);
            } else {
                self.ssh.pending_host_keys.push_back(challenge);
            }
        }
        let runtime = self.terminal.host_runtime.clone();
        cx.background_spawn(async move {
            let _ = disconnect_host_ssh(runtime, connection_id).await;
        })
        .detach();
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
        let Some(mut connection) = self
            .ssh
            .connections
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
            .cloned()
        else {
            self.ssh.project_picker.error = Some(
                self.ui_text
                    .get(UiTextKey::SshProjectConnectionMissing)
                    .to_string(),
            );
            return;
        };
        if connection.auth == SshAuthPreference::Password
            && password.is_none()
            && connection.credential.is_none()
        {
            self.show_ssh_password_prompt(connection_id, continuation, None, cx);
            return;
        }
        let Some(local_profile) = self.config_paths.profile().cloned() else {
            self.ssh.project_picker.error =
                Some("Manage remote servers from the local yttt window.".into());
            return;
        };
        match continuation {
            SshProjectConnectContinuation::Browse { initial_root } => {
                if initial_root.is_some() {
                    connection.default_remote_root = initial_root;
                }
            }
            SshProjectConnectContinuation::OpenRecent { root } => {
                connection.default_remote_root = Some(root)
            }
        }
        let launch = crate::remote_launch::RemoteLaunch {
            local_profile,
            target: crate::remote_launch::RemoteTarget::SshServer {
                connection,
                save_password_as: password
                    .as_ref()
                    .and_then(|password| password.save_as.clone()),
                password: password.map(|password| password.secret.to_string()),
                passphrase: key_passphrase,
            },
        };
        let task =
            cx.background_spawn(async move { crate::remote_launch::spawn_remote_client(launch) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |root, cx| {
                match result {
                    Ok(()) => root.close_ssh_project_picker(cx),
                    Err(error) => {
                        root.ssh.project_picker.error =
                            Some(format!("Failed to launch remote workspace: {error}"))
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
        {
            return;
        }
        if let Some(epoch) = self.ssh.project_picker.connection_epoch {
            if epoch != status.epoch {
                return;
            }
        } else {
            self.ssh.project_picker.connection_epoch = Some(status.epoch);
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
        let Some(runtime) = self.terminal.host_runtime.clone() else {
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
        self.ssh.project_picker.view = SshProjectPickerView::Opening;
        self.ssh.project_picker.loading = true;
        self.ssh.project_picker.error = None;
        self.ssh.project_picker.generation = self.ssh.project_picker.generation.wrapping_add(1);
        let generation = self.ssh.project_picker.generation;
        let task = cx.background_spawn(scan_ssh_directory(
            runtime,
            connection_id.clone(),
            root.clone(),
        ));
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
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            self.ssh.project_picker.error = Some(
                self.ui_text
                    .get(UiTextKey::SshRuntimeUnavailable)
                    .to_string(),
            );
            return;
        };
        self.ssh.project_picker.view = SshProjectPickerView::Browsing;
        self.ssh.project_picker.loading = true;
        self.ssh.project_picker.error = None;
        self.ssh.project_picker.generation = self.ssh.project_picker.generation.wrapping_add(1);
        let generation = self.ssh.project_picker.generation;
        let task = cx.background_spawn(resolve_ssh_home(runtime, connection_id.clone()));
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
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            self.ssh.project_picker.error = Some(
                self.ui_text
                    .get(UiTextKey::SshRuntimeUnavailable)
                    .to_string(),
            );
            return;
        };
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
        let task = cx.background_spawn(scan_ssh_directory(
            runtime,
            connection_id.clone(),
            path.clone(),
        ));
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
                            .into_iter()
                            .filter(|entry| entry.kind == RemoteFileKind::Directory)
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
        if self
            .terminal
            .host_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.is_remote())
        {
            match yttt_protocol::HostPath::from_path(Path::new(path.as_str())) {
                Ok(path) => self.load_remote_host_directory(path, cx),
                Err(error) => self.ssh.project_picker.error = Some(error.to_string()),
            }
            return;
        }
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
        if self
            .terminal
            .host_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.is_remote())
        {
            if let Some(path) = self.ssh.project_picker.current_path.clone() {
                self.navigate_ssh_project_directory(path, cx);
            } else {
                self.open_remote_host_directory_picker(cx);
            }
            return;
        }
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
        if self
            .terminal
            .host_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.is_remote())
        {
            self.close_ssh_project_picker(cx);
            return;
        }
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
        if self
            .terminal
            .host_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.is_remote())
        {
            let Some(path) = self.ssh.project_picker.current_path.clone() else {
                return;
            };
            match self.open_project_path(Path::new(path.as_str())) {
                Ok(()) => self.close_ssh_project_picker(cx),
                Err(error) => self.ssh.project_picker.error = Some(error.to_string()),
            }
            cx.notify();
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
        let ui_style = current_ui_style(cx);
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
                list.delegate_mut().replace_sections(sections, ui_style);
                cx.notify();
            });
            return list;
        }

        let empty_message = self.ui_text.get(UiTextKey::SshNoConnections);
        let list = cx.new(|cx| {
            ListState::new(
                SshConnectionListDelegate::new(sections, empty_message, ui_style),
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

async fn resolve_ssh_home(
    runtime: Arc<crate::host_runtime::DesktopHostRuntime>,
    connection_id: ConnectionId,
) -> Result<RemotePathBuf, String> {
    match request_host(
        Some(runtime),
        Request::RemoteFile(RemoteFileRequest::ResolveHome {
            connection_id: connection_id.as_str().to_string(),
        }),
    )
    .await?
    {
        Response::RemoteFile(RemoteFileResponse::Home(home)) => {
            RemotePathBuf::new(home).map_err(|error| error.to_string())
        }
        response => Err(format!(
            "Host returned an unexpected SSH home response: {response:?}"
        )),
    }
}

async fn scan_ssh_directory(
    runtime: Arc<crate::host_runtime::DesktopHostRuntime>,
    connection_id: ConnectionId,
    root: RemotePathBuf,
) -> Result<Vec<RemoteFileEntry>, String> {
    match request_host(
        Some(runtime),
        Request::RemoteFile(RemoteFileRequest::BrowseDirectory {
            connection_id: connection_id.as_str().to_string(),
            root: root.as_str().to_string(),
            relative_directory: String::new(),
            show_hidden: true,
        }),
    )
    .await?
    {
        Response::RemoteFile(RemoteFileResponse::Directory(snapshot)) => Ok(snapshot.entries),
        response => Err(format!(
            "Host returned an unexpected SSH directory response: {response:?}"
        )),
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

    yttt_dialog_overlay(
        yttt_dialog_surface(theme, ui_style)
            .w(ui_style.palette.remote_panel_width)
            .max_w(ui_style.palette.remote_panel_width)
            .max_h(ui_style.palette.remote_panel_max_height)
            .child(content),
        YtttDialogPlacement::Top,
        theme,
        ui_style,
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
    let new_connection = yttt_row(
        YtttRowKind::PaletteCompact,
        SelectableState::Inactive,
        true,
        theme,
        ui_style,
    )
    .id("ssh-project-new-connection")
    .debug_selector(|| "ssh-project-new-connection".to_string())
    .mx(ui_style.palette.list_padding_x)
    .flex()
    .items_center()
    .gap(ui_style.palette.item_content_gap)
    .on_click(cx.listener(|this, _, _window, cx| {
        this.new_ssh_project_connection();
        cx.notify();
    }))
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(ui_style.palette.icon_column_width)
            .child(
                Icon::new(IconName::Plus)
                    .size(ui_style.palette.icon_size)
                    .text_color(theme.text_muted),
            ),
    )
    .child(
        div()
            .text_sm()
            .text_color(theme.text)
            .child(root.ui_text.get(UiTextKey::SshNewConnection)),
    );

    let mut body = div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.md)
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
        .child(new_connection)
        .child(
            div()
                .debug_selector(|| "ssh-project-connection-list".to_string())
                .h(ui_style.palette.remote_list_height)
                .min_h_0()
                .overflow_hidden()
                .when(ui_style.palette.item_cards, |this| {
                    this.rounded(ui_style.radius.control)
                        .border(ui_style.border.hairline)
                        .border_color(theme.border)
                })
                .child(List::new(&connection_list).size_full()),
        );
    if let Some(error) = root.ssh.project_picker.error.clone() {
        body = body.child(
            yttt_alert(
                "ssh-project-connections-error",
                error,
                YtttNotificationTone::Error,
                theme,
                ui_style,
            )
            .title(root.ui_text.get(UiTextKey::SshProjectConnectionFailed)),
        );
    }
    body.child(
        div()
            .flex()
            .justify_end()
            .border_t(ui_style.border.hairline)
            .border_color(theme.border)
            .pt(ui_style.spacing.md)
            .child(yttt_dialog_button(
                cx,
                "ssh-project-cancel",
                root.ui_text.get(UiTextKey::Cancel),
                YtttButtonVariant::Ghost,
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
            .child(yttt_labeled_switch(
                "ssh-project-remember-password",
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
    let error = root
        .ssh
        .project_picker
        .error
        .clone()
        .or_else(|| root.ssh.error.clone());
    if let Some(error) = error {
        fields = fields.child(
            yttt_alert(
                "ssh-project-quick-connect-error",
                error,
                YtttNotificationTone::Error,
                theme,
                ui_style,
            )
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
        body = body.child(yttt_alert(
            "ssh-password-prompt-error",
            error,
            YtttNotificationTone::Error,
            theme,
            ui_style,
        ));
    }
    if let Some(input) = password_input {
        body = body.child(ssh_form_field(
            root.ui_text.get(UiTextKey::SshPassword),
            &input,
            theme,
            ui_style,
        ));
    }
    body.child(yttt_labeled_switch(
        "ssh-password-prompt-remember",
        root.ui_text.get(UiTextKey::SshRememberPassword),
        root.ssh.project_picker.remember_password,
        theme,
        ui_style,
        cx.listener(|this, checked: &bool, _window, cx| {
            this.ssh.project_picker.remember_password = *checked;
            cx.notify();
        }),
    ))
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
            yttt_alert(
                "ssh-project-connection-error",
                message,
                YtttNotificationTone::Error,
                theme,
                ui_style,
            )
            .title(root.ui_text.get(title)),
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
            yttt_button_base(
                "ssh-project-parent-directory",
                YtttButtonVariant::Ghost,
                theme,
                ui_style,
                cx,
            )
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
            yttt_button_base(
                SharedString::from(format!("ssh-project-directory-{}", directory.path)),
                YtttButtonVariant::Ghost,
                theme,
                ui_style,
                cx,
            )
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
                ))
                .when(crate::config::storage::is_remote(), |row| {
                    row.child(
                        yttt_dialog_button(
                            cx,
                            "remote-project-create-directory",
                            "Create folder",
                            YtttButtonVariant::Secondary,
                            theme,
                            cx.listener(|this, _, _, cx| this.create_remote_host_directory(cx)),
                        )
                        .disabled(loading),
                    )
                }),
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
            yttt_button(
                "ssh-project-back",
                format!("← {}", root.ui_text.get(UiTextKey::SshOpenRemoteProject)),
                YtttButtonVariant::Ghost,
                theme,
                ui_style,
                cx,
            )
            .on_click(cx.listener(|this, _, _window, cx| {
                this.back_ssh_project_picker(cx);
                cx.notify();
            })),
        )
        .child(
            yttt_button(
                "ssh-project-close",
                "×",
                YtttButtonVariant::Ghost,
                theme,
                ui_style,
                cx,
            )
            .on_click(cx.listener(|this, _, _window, cx| {
                this.close_ssh_project_picker(cx);
                cx.notify();
            })),
        )
}

#[cfg(test)]
mod tests {
    use super::{remote_child_path, remote_parent_path};
    use yttt_core::model::project::RemotePathBuf;

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
}
