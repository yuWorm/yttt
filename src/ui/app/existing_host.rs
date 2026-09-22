mod credentials;

use credentials::ConnectionCredentialStore;

use crate::{
    config::profile::AppProfile,
    remote_launch::{
        ConnectionCode, MAX_CONNECTION_CODE_BYTES, RemoteAppearance, RemoteLaunch, RemoteTarget,
        validate_connection_address,
    },
    ui::{
        i18n::{UiText, UiTextKey},
        primitives::{
            button::{YtttButtonVariant, yttt_button},
            input::{YtttInputKind, yttt_input},
        },
        theme::{current_ui_style, current_workbench_theme},
    },
};
use gpui::{
    AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollHandle, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, px,
};
use gpui_component::{
    Disableable as _,
    input::{InputEvent, InputState},
    scroll::ScrollableElement as _,
};
use serde::{Deserialize, Serialize};
use std::io::{Read as _, Write as _};
use yttt_core::model::ids::CredentialId;
use yttt_protocol::remote_access::RemoteConnectionInfo;
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RememberedConnection {
    #[serde(default)]
    pub name: String,
    pub address: String,
    pub environment_id: String,
    pub credential_id: CredentialId,
}

impl RememberedConnection {
    fn normalize_name(&mut self) {
        self.name = display_name(&self.name, &self.address);
    }
}

fn display_name(name: &str, address: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        address.to_string()
    } else {
        name.to_string()
    }
}

fn read_connections(profile: &AppProfile) -> Result<Vec<RememberedConnection>, String> {
    let file = match std::fs::File::open(
        profile
            .paths()
            .state
            .join("client-connections/existing-host.json"),
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(8193)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 8192 {
        return Err("Saved connection metadata exceeds 8 KiB".into());
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Records {
        List(Vec<RememberedConnection>),
        Legacy(RememberedConnection),
    }
    let mut records = match serde_json::from_slice(&bytes).map_err(|error| error.to_string())? {
        Records::List(records) => records,
        Records::Legacy(record) => vec![record],
    };
    for record in &mut records {
        record.normalize_name();
    }
    Ok(records)
}

fn save_connections(profile: &AppProfile, records: &[RememberedConnection]) -> Result<(), String> {
    let bytes = serde_json::to_vec(records).map_err(|error| error.to_string())?;
    if bytes.len() > 8192 {
        return Err("Saved connection metadata exceeds 8 KiB".into());
    }
    let directory = profile.paths().state.join("client-connections");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(&directory).map_err(|error| error.to_string())?;
    temporary
        .write_all(&bytes)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(directory.join("existing-host.json"))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn update_record(
    records: &mut Vec<RememberedConnection>,
    record: RememberedConnection,
    editing: Option<&CredentialId>,
) -> Result<(), String> {
    if let Some(editing) = editing {
        let Some(index) = records
            .iter()
            .position(|existing| existing.credential_id == *editing)
        else {
            return Err("Saved connection was removed before it could be updated.".into());
        };
        if record.credential_id != *editing {
            return Err("Saved connection identity changed while editing.".into());
        }
        records[index] = record;
    } else {
        if records
            .iter()
            .any(|existing| existing.credential_id == record.credential_id)
        {
            return Err("A saved connection already uses this credential identity.".into());
        }
        records.push(record);
    }
    Ok(())
}

fn metadata_edit_record(
    existing: &RememberedConnection,
    name: &str,
    address: String,
) -> RememberedConnection {
    RememberedConnection {
        name: display_name(name, &address),
        address,
        environment_id: existing.environment_id.clone(),
        credential_id: existing.credential_id.clone(),
    }
}

fn connection_code_matches_record(
    record: &RememberedConnection,
    connection_info: &RemoteConnectionInfo,
) -> bool {
    record.environment_id == connection_info.environment_id
}

fn connection_code_for_editor(
    record: &RememberedConnection,
    secret: &str,
) -> Result<String, String> {
    if ConnectionCode::decode(secret).is_ok() {
        return Ok(secret.to_string());
    }
    let info = serde_json::from_str::<RemoteConnectionInfo>(secret)
        .map_err(|_| "Saved credentials are not a valid connection code.".to_string())?;
    ConnectionCode::encode(record.address.clone(), info).map_err(|error| error.to_string())
}

fn decode_saved_connection(
    record: &RememberedConnection,
    secret: &str,
) -> Result<(String, RemoteConnectionInfo), String> {
    validate_connection_address(&record.address).map_err(|error| error.to_string())?;
    let connection_info = match ConnectionCode::decode(secret) {
        Ok(code) => code.connection_info,
        Err(_) => serde_json::from_str::<RemoteConnectionInfo>(secret)
            .map_err(|_| "Saved credentials are not a valid connection code.".to_string())?,
    };
    Ok((record.address.clone(), connection_info))
}

pub(crate) fn create(
    profile: AppProfile,
    text: UiText,
    window: &mut Window,
    cx: &mut gpui::App,
) -> Entity<ExistingHostForm> {
    let view = cx.new(|cx| {
        let info = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(text.get(UiTextKey::ConnectionCodePlaceholder))
                .validate(|value, _| value.len() <= MAX_CONNECTION_CODE_BYTES)
        });
        let subscription = cx.subscribe_in(
            &info,
            window,
            |view: &mut ExistingHostForm, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.import_code(window, cx);
                }
            },
        );
        ExistingHostForm {
            profile,
            text,
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(text.get(UiTextKey::RemoteConnectionName))
                    .validate(|value, _| value.len() <= 1024)
            }),
            address: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("host:port")
                    .validate(|value, _| value.len() <= 1024)
            }),
            info,
            remember: false,
            remember_changed: false,
            busy: false,
            error: None,
            saved: Vec::new(),
            editor: Editor::Closed,
            operation_generation: 0,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    });
    view.update(cx, |view, cx| view.load_remembered(window, cx));
    view
}

pub(crate) struct ExistingHostForm {
    profile: AppProfile,
    text: UiText,
    name: Entity<InputState>,
    address: Entity<InputState>,
    info: Entity<InputState>,
    remember: bool,
    remember_changed: bool,
    busy: bool,
    error: Option<String>,
    saved: Vec<RememberedConnection>,
    editor: Editor,
    operation_generation: u64,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

#[derive(Clone)]
enum Editor {
    Closed,
    Full { editing: Option<CredentialId> },
    Credentials { target: RememberedConnection },
}

#[derive(Clone)]
enum LaunchSource {
    Saved,
    Full { editing: Option<CredentialId> },
    Credentials { credential_id: CredentialId },
}

enum SavedConnectionLoad {
    Ready {
        address: String,
        connection_info: RemoteConnectionInfo,
    },
    CredentialsRequired(Option<String>),
}

enum CredentialPersistence {
    Save(Zeroizing<String>),
    Delete,
    Unchanged,
}

impl ExistingHostForm {
    pub(crate) fn credentials_only(&self) -> bool {
        matches!(self.editor, Editor::Credentials { .. })
    }
    pub(crate) fn records(&self) -> &[RememberedConnection] {
        &self.saved
    }

    pub(crate) fn busy(&self) -> bool {
        self.busy
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn editor_open(&self) -> bool {
        !matches!(&self.editor, Editor::Closed)
    }

    pub(crate) fn new_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.editor_open() {
            return;
        }
        self.begin_full_editor(None, String::new(), String::new(), true, window, cx);
        self.info.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(crate) fn edit_connection(
        &mut self,
        id: CredentialId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.editor_open() {
            return;
        }
        let Some(record) = self
            .saved
            .iter()
            .find(|record| record.credential_id == id)
            .cloned()
        else {
            self.error = Some(self.text.get(UiTextKey::RemoteRecordMissing).into());
            cx.notify();
            return;
        };
        self.begin_full_editor(
            Some(record.credential_id.clone()),
            record.name.clone(),
            record.address.clone(),
            false,
            window,
            cx,
        );
        let generation = self.next_operation_generation();
        self.busy = true;
        let profile = self.profile.clone();
        let credential_id = record.credential_id.clone();
        let task_record = record.clone();
        let task = cx.background_spawn(async move {
            let store = ConnectionCredentialStore::new(format!(
                "{}.existing-host",
                profile.credential_namespace()
            ));
            store
                .load(&credential_id)
                .map_err(|error| error.to_string())?
                .map(|secret| connection_code_for_editor(&task_record, &secret))
                .transpose()
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, window, cx| {
                if !view.full_editor_is_current(generation, &Some(id.clone())) {
                    return;
                }
                view.busy = false;
                match result {
                    Ok(Some(code)) => {
                        view.remember = true;
                        view.info
                            .update(cx, |input, cx| input.set_value(code, window, cx));
                        view.address.update(cx, |input, cx| {
                            input.set_value(record.address.clone(), window, cx)
                        });
                    }
                    Ok(None) => {
                        view.remember = false;
                        view.error =
                            Some(view.text.get(UiTextKey::RemoteCredentialsRequired).into());
                        view.info.update(cx, |input, cx| input.focus(window, cx));
                    }
                    Err(error) => {
                        view.remember = false;
                        view.error = Some(error);
                        view.info.update(cx, |input, cx| input.focus(window, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn connect_saved(
        &mut self,
        id: CredentialId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.editor_open() {
            return;
        }
        let Some(record) = self
            .saved
            .iter()
            .find(|record| record.credential_id == id)
            .cloned()
        else {
            self.error = Some(self.text.get(UiTextKey::RemoteRecordMissing).into());
            cx.notify();
            return;
        };
        let generation = self.next_operation_generation();
        self.busy = true;
        self.error = None;
        let profile = self.profile.clone();
        let task_record = record.clone();
        let task = cx.background_spawn(async move {
            validate_connection_address(&task_record.address).map_err(|error| error.to_string())?;
            let store = ConnectionCredentialStore::new(format!(
                "{}.existing-host",
                profile.credential_namespace()
            ));
            match store.load(&task_record.credential_id) {
                Ok(Some(secret)) => match decode_saved_connection(&task_record, &secret) {
                    Ok((address, connection_info))
                        if connection_code_matches_record(&task_record, &connection_info) =>
                    {
                        Ok(SavedConnectionLoad::Ready {
                            address,
                            connection_info,
                        })
                    }
                    Ok(_) | Err(_) => Ok(SavedConnectionLoad::CredentialsRequired(None)),
                },
                Ok(None) => Ok(SavedConnectionLoad::CredentialsRequired(None)),
                Err(error) => Ok(SavedConnectionLoad::CredentialsRequired(Some(
                    error.to_string(),
                ))),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, window, cx| {
                if view.operation_generation != generation
                    || !matches!(&view.editor, Editor::Closed)
                {
                    return;
                }
                match result {
                    Ok(SavedConnectionLoad::Ready {
                        address,
                        connection_info,
                    }) => view.start_remote_client(
                        address,
                        connection_info,
                        generation,
                        LaunchSource::Saved,
                        window,
                        cx,
                    ),
                    Ok(SavedConnectionLoad::CredentialsRequired(error)) => {
                        view.busy = false;
                        view.error = error;
                        view.editor = Editor::Credentials { target: record };
                        view.remember = true;
                        view.remember_changed = false;
                        view.info.update(cx, |input, cx| {
                            input.set_value("", window, cx);
                            input.focus(window, cx);
                        });
                        cx.notify();
                    }
                    Err(error) => {
                        view.busy = false;
                        view.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn delete_connection(
        &mut self,
        id: CredentialId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.editor_open() {
            return;
        }
        if !self.saved.iter().any(|record| record.credential_id == id) {
            self.error = Some(self.text.get(UiTextKey::RemoteRecordMissing).into());
            cx.notify();
            return;
        }
        let generation = self.next_operation_generation();
        self.busy = true;
        self.error = None;
        let profile = self.profile.clone();
        let task_id = id.clone();
        let task = cx.background_spawn(async move {
            let mut records = read_connections(&profile)?;
            let Some(index) = records
                .iter()
                .position(|record| record.credential_id == task_id)
            else {
                return Err("Saved connection was removed before it could be deleted.".into());
            };
            let store = ConnectionCredentialStore::new(format!(
                "{}.existing-host",
                profile.credential_namespace()
            ));
            store.delete(&task_id).map_err(|error| error.to_string())?;
            records.remove(index);
            save_connections(&profile, &records)?;
            Ok::<_, String>(records)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, _, cx| {
                if view.operation_generation != generation || view.editor_open() {
                    return;
                }
                view.busy = false;
                match result {
                    Ok(records) => view.saved = records,
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn dismiss_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editor_open() {
            return;
        }
        self.next_operation_generation();
        self.busy = false;
        self.error = None;
        self.editor = Editor::Closed;
        self.remember = false;
        self.remember_changed = false;
        self.clear_editor_inputs(window, cx);
        cx.notify();
    }

    fn import_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(&self.editor, Editor::Full { .. }) {
            return;
        }
        let value = self.info.read(cx).value();
        if value.is_empty() {
            self.error = None;
            cx.notify();
            return;
        }
        match ConnectionCode::decode(&value) {
            Ok(code) => {
                self.address
                    .update(cx, |input, cx| input.set_value(code.address, window, cx));
                self.error = None;
            }
            Err(_) => self.error = Some(self.text.get(UiTextKey::ConnectionCodeInvalid).into()),
        }
        cx.notify();
    }

    fn load_remembered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.profile.clone();
        self.busy = true;
        let task = cx.background_spawn(async move { read_connections(&profile) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, _, cx| {
                if !view.busy || view.editor_open() {
                    return;
                }
                view.busy = false;
                match result {
                    Ok(saved) => view.saved = saved,
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn begin_full_editor(
        &mut self,
        editing: Option<CredentialId>,
        name: String,
        address: String,
        remember: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_operation_generation();
        self.editor = Editor::Full { editing };
        self.remember = remember;
        self.remember_changed = false;
        self.error = None;
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.info
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.address
            .update(cx, |input, cx| input.set_value(address, window, cx));
        cx.notify();
    }

    fn save_editor(&mut self, connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Editor::Full { editing } = self.editor.clone() else {
            return;
        };
        let address = self.address.read(cx).value().trim().to_string();
        if validate_connection_address(&address).is_err() {
            self.error = Some(self.text.get(UiTextKey::ConnectionAddressInvalid).into());
            cx.notify();
            return;
        }
        let name = self.name.read(cx).value().to_string();
        let payload = Zeroizing::new(self.info.read(cx).value().to_string());
        let blank_code = payload.trim().is_empty();
        let (record, connection_info, credential_persistence) = if blank_code && !connect {
            let Some(editing_id) = editing.as_ref() else {
                self.error = Some(self.text.get(UiTextKey::ConnectionCodeInvalid).into());
                cx.notify();
                return;
            };
            let Some(existing) = self
                .saved
                .iter()
                .find(|record| record.credential_id == *editing_id)
            else {
                self.error = Some(self.text.get(UiTextKey::RemoteRecordMissing).into());
                cx.notify();
                return;
            };
            if self.remember_changed && self.remember {
                self.error = Some(self.text.get(UiTextKey::RemoteCredentialsRequired).into());
                cx.notify();
                return;
            }
            (
                metadata_edit_record(existing, &name, address.clone()),
                None,
                if self.remember_changed {
                    CredentialPersistence::Delete
                } else {
                    CredentialPersistence::Unchanged
                },
            )
        } else {
            let connection_info = match ConnectionCode::decode(&payload) {
                Ok(code) => code.connection_info,
                Err(_) => {
                    self.error = Some(self.text.get(UiTextKey::ConnectionCodeInvalid).into());
                    cx.notify();
                    return;
                }
            };
            let record = RememberedConnection {
                name: display_name(&name, &address),
                address: address.clone(),
                environment_id: connection_info.environment_id.clone(),
                credential_id: editing.clone().unwrap_or_else(CredentialId::random),
            };
            let credential_persistence = if self.remember {
                CredentialPersistence::Save(Zeroizing::new(payload.trim().to_string()))
            } else if editing.is_some() {
                CredentialPersistence::Delete
            } else {
                CredentialPersistence::Unchanged
            };
            (record, Some(connection_info), credential_persistence)
        };
        let generation = self.next_operation_generation();
        self.busy = true;
        self.error = None;
        let profile = self.profile.clone();
        let editing_for_task = editing.clone();
        let text = self.text;
        let task = cx.background_spawn(async move {
            let mut records = read_connections(&profile)?;
            let store = ConnectionCredentialStore::new(format!(
                "{}.existing-host",
                profile.credential_namespace()
            ));
            match credential_persistence {
                CredentialPersistence::Save(payload) => {
                    store
                        .save(&record.credential_id, &payload)
                        .map_err(|error| {
                            format!(
                                "{}: {error}",
                                text.get(UiTextKey::RemoteCredentialStoreFailed)
                            )
                        })?;
                }
                CredentialPersistence::Delete => {
                    store
                        .delete(&record.credential_id)
                        .map_err(|error| error.to_string())?;
                }
                CredentialPersistence::Unchanged => {}
            }
            update_record(&mut records, record, editing_for_task.as_ref())?;
            save_connections(&profile, &records)?;
            Ok::<_, String>((records, connection_info))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, window, cx| {
                if !view.full_editor_is_current(generation, &editing) {
                    return;
                }
                match result {
                    Ok((records, connection_info)) => {
                        view.saved = records;
                        if connect {
                            view.start_remote_client(
                                address,
                                connection_info.expect("connecting requires a connection code"),
                                generation,
                                LaunchSource::Full { editing },
                                window,
                                cx,
                            );
                        } else {
                            view.finish_editor_success(window, cx);
                        }
                    }
                    Err(error) => {
                        view.busy = false;
                        view.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn submit_missing_credentials(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Editor::Credentials { target } = self.editor.clone() else {
            return;
        };
        let payload = Zeroizing::new(self.info.read(cx).value().to_string());
        let connection_info = match ConnectionCode::decode(&payload) {
            Ok(code) => code.connection_info,
            Err(_) => {
                self.error = Some(self.text.get(UiTextKey::ConnectionCodeInvalid).into());
                cx.notify();
                return;
            }
        };
        if !connection_code_matches_record(&target, &connection_info) {
            self.error = Some(self.text.get(UiTextKey::RemoteWrongHostCode).into());
            cx.notify();
            return;
        }
        if validate_connection_address(&target.address).is_err() {
            self.error = Some(self.text.get(UiTextKey::ConnectionAddressInvalid).into());
            cx.notify();
            return;
        }
        if !self
            .saved
            .iter()
            .any(|record| record.credential_id == target.credential_id)
        {
            self.error = Some(self.text.get(UiTextKey::RemoteRecordMissing).into());
            cx.notify();
            return;
        }
        let generation = self.next_operation_generation();
        self.busy = true;
        self.error = None;
        let profile = self.profile.clone();
        let remember = self.remember;
        let credential_id = target.credential_id.clone();
        let text = self.text;
        let task = cx.background_spawn(async move {
            if remember {
                let store = ConnectionCredentialStore::new(format!(
                    "{}.existing-host",
                    profile.credential_namespace()
                ));
                store
                    .save(&credential_id, payload.trim())
                    .map_err(|error| {
                        format!(
                            "{}: {error}",
                            text.get(UiTextKey::RemoteCredentialStoreFailed)
                        )
                    })?;
            }
            Ok::<_, String>(())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, window, cx| {
                let source = LaunchSource::Credentials {
                    credential_id: target.credential_id,
                };
                if !view.launch_is_current(generation, &source) {
                    return;
                }
                match result {
                    Ok(()) => view.start_remote_client(
                        target.address,
                        connection_info,
                        generation,
                        source,
                        window,
                        cx,
                    ),
                    Err(error) => {
                        view.busy = false;
                        view.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn start_remote_client(
        &mut self,
        address: String,
        connection_info: RemoteConnectionInfo,
        generation: u64,
        source: LaunchSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = self.profile.clone();
        let appearance = RemoteAppearance::capture(cx, self.text);
        let task = cx.background_spawn(async move {
            crate::remote_launch::spawn_remote_client(RemoteLaunch {
                local_profile: profile,
                appearance,
                target: RemoteTarget::ExistingHost {
                    address,
                    connection_info,
                },
            })
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, window, cx| {
                if !view.launch_is_current(generation, &source) {
                    return;
                }
                match result {
                    Ok(()) => match &source {
                        LaunchSource::Saved => {
                            view.busy = false;
                            view.error = None;
                            cx.notify();
                        }
                        LaunchSource::Full { .. } | LaunchSource::Credentials { .. } => {
                            view.finish_editor_success(window, cx);
                        }
                    },
                    Err(error) => {
                        view.busy = false;
                        view.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn full_editor_is_current(&self, generation: u64, editing: &Option<CredentialId>) -> bool {
        self.operation_generation == generation
            && matches!(&self.editor, Editor::Full { editing: current } if current == editing)
    }

    fn launch_is_current(&self, generation: u64, source: &LaunchSource) -> bool {
        if self.operation_generation != generation {
            return false;
        }
        match source {
            LaunchSource::Saved => matches!(&self.editor, Editor::Closed),
            LaunchSource::Full { editing } => self.full_editor_is_current(generation, editing),
            LaunchSource::Credentials { credential_id } => matches!(
                &self.editor,
                Editor::Credentials { target } if target.credential_id == *credential_id
            ),
        }
    }

    fn finish_editor_success(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = false;
        self.error = None;
        self.editor = Editor::Closed;
        self.remember = false;
        self.remember_changed = false;
        self.next_operation_generation();
        self.clear_editor_inputs(window, cx);
        cx.notify();
    }

    fn clear_editor_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.name
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.info
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.address
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn next_operation_generation(&mut self) -> u64 {
        self.operation_generation = self.operation_generation.wrapping_add(1);
        self.operation_generation
    }

    fn render_full_editor(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        let error = self.error.clone();
        div()
            .debug_selector(|| "existing-host-editor".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(theme.text)
            .child(
                div()
                    .id("existing-host-form-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .vertical_scrollbar(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(style.spacing.md)
                    .p(px(20.0))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text_muted)
                            .child(self.text.get(UiTextKey::ExistingHostDescription)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(self.text.get(UiTextKey::RemoteConnectionName)),
                    )
                    .child(
                        yttt_input(&self.name, YtttInputKind::Settings, theme, style)
                            .disabled(self.busy),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(self.text.get(UiTextKey::ConnectionCodeLabel)),
                    )
                    .child(
                        yttt_input(&self.info, YtttInputKind::Settings, theme, style)
                            .disabled(self.busy),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(self.text.get(UiTextKey::ConnectionCodeSecurity)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(self.text.get(UiTextKey::ConnectionAddressLabel)),
                    )
                    .child(
                        yttt_input(&self.address, YtttInputKind::Settings, theme, style)
                            .disabled(self.busy),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(self.text.get(UiTextKey::ConnectionAddressHint)),
                    )
                    .children(error.map(|error| div().text_sm().child(error))),
            )
            .child(
                div()
                    .id("existing-host-form-footer")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .p(px(12.0))
                    .gap(style.spacing.sm)
                    .border_t_1()
                    .border_color(theme.border_variant)
                    .child(
                        yttt_button(
                            "existing-host-remember",
                            self.text.get(if self.remember {
                                UiTextKey::ConnectionRemembered
                            } else {
                                UiTextKey::ConnectionRemember
                            }),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.remember = !view.remember;
                            view.remember_changed = true;
                            cx.notify();
                        })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(style.spacing.sm)
                            .child(
                                yttt_button(
                                    "existing-host-cancel",
                                    self.text.get(UiTextKey::Cancel),
                                    YtttButtonVariant::Secondary,
                                    theme,
                                    style,
                                    cx,
                                )
                                .disabled(self.busy)
                                .on_click(cx.listener(
                                    |view, _, window, cx| view.dismiss_editor(window, cx),
                                )),
                            )
                            .child(
                                yttt_button(
                                    "save-host-connection",
                                    self.text.get(UiTextKey::SettingsSave),
                                    YtttButtonVariant::Secondary,
                                    theme,
                                    style,
                                    cx,
                                )
                                .disabled(self.busy)
                                .on_click(cx.listener(
                                    |view, _, window, cx| view.save_editor(false, window, cx),
                                )),
                            )
                            .child(
                                yttt_button(
                                    "existing-host-save-connect",
                                    self.text.get(UiTextKey::RemoteSaveConnect),
                                    YtttButtonVariant::Primary,
                                    theme,
                                    style,
                                    cx,
                                )
                                .disabled(self.busy)
                                .on_click(cx.listener(
                                    |view, _, window, cx| view.save_editor(true, window, cx),
                                )),
                            ),
                    ),
            )
    }

    fn render_credentials_editor(
        &mut self,
        target: RememberedConnection,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        let error = self.error.clone();
        div()
            .debug_selector(|| "existing-host-credentials-editor".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(theme.text)
            .child(
                div()
                    .id("existing-host-credentials-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(style.spacing.md)
                    .p(px(20.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(target.name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(target.address),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text_muted)
                            .child(self.text.get(UiTextKey::RemoteCredentialsRequired)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(self.text.get(UiTextKey::ConnectionCodeLabel)),
                    )
                    .child(
                        yttt_input(&self.info, YtttInputKind::Settings, theme, style)
                            .disabled(self.busy),
                    )
                    .child(
                        yttt_button(
                            "existing-host-credentials-remember",
                            self.text.get(if self.remember {
                                UiTextKey::ConnectionRemembered
                            } else {
                                UiTextKey::ConnectionRemember
                            }),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.remember = !view.remember;
                            cx.notify();
                        })),
                    )
                    .children(error.map(|error| div().text_sm().child(error))),
            )
            .child(
                div()
                    .id("existing-host-credentials-footer")
                    .flex()
                    .flex_none()
                    .justify_end()
                    .gap(style.spacing.sm)
                    .p(px(12.0))
                    .border_t_1()
                    .border_color(theme.border_variant)
                    .child(
                        yttt_button(
                            "existing-host-credentials-cancel",
                            self.text.get(UiTextKey::Cancel),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(|view, _, window, cx| view.dismiss_editor(window, cx)),
                        ),
                    )
                    .child(
                        yttt_button(
                            "existing-host-credentials-connect",
                            self.text.get(UiTextKey::SshConnect),
                            YtttButtonVariant::Primary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.submit_missing_credentials(window, cx)
                        })),
                    ),
            )
    }
}

impl Render for ExistingHostForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.editor.clone() {
            Editor::Closed => div().debug_selector(|| "existing-host-controller".into()),
            Editor::Full { .. } => self.render_full_editor(cx),
            Editor::Credentials { target } => self.render_credentials_editor(target, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection_info() -> RemoteConnectionInfo {
        RemoteConnectionInfo {
            environment_id: "paste-environment".into(),
            profile_id: yttt_core::model::ids::ProfileId::new("paste-profile"),
            server_name: "yttt-host.local".into(),
            certificate_der: vec![1, 2, 3],
            certificate_sha256: "ab".repeat(32),
            credential_generation: 1,
            work_secret: [37; 32],
        }
    }

    #[gpui::test]
    fn pasting_connection_code_fills_address_and_rejects_bad_credentials(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::config::profile::{
            EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        };
        use std::{cell::RefCell, rc::Rc};
        cx.update(gpui_component::init);
        let temporary = tempfile::tempdir().unwrap();
        let profile = AppProfile::scoped(
            yttt_core::model::ids::ProfileId::new("paste-test"),
            EnvironmentKind::Test,
            ProfilePersistence::Ephemeral,
            temporary.path(),
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        );
        let slot = Rc::new(RefCell::new(None));
        let window_slot = slot.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let form = create(profile, crate::ui::i18n::UiText::english(), window, cx);
            *window_slot.borrow_mut() = Some(form.clone());
            gpui_component::Root::new(form, window, cx)
        });
        let form = slot.borrow_mut().take().unwrap();
        cx.run_until_parked();
        form.update_in(cx, |form, window, cx| form.new_connection(window, cx));
        form.read_with(cx, |form, _| {
            assert!(
                form.remember,
                "new connections must remember credentials by default"
            );
        });
        let code =
            ConnectionCode::encode("remote.example:43123".into(), connection_info()).unwrap();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(code));
        form.update_in(cx, |form, window, cx| {
            form.info.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-v"
        } else {
            "ctrl-v"
        });
        cx.run_until_parked();
        form.read_with(cx, |form, cx| {
            assert_eq!(
                form.address.read(cx).value().as_str(),
                "remote.example:43123"
            );
            assert!(form.error.is_none());
            assert_eq!(
                ConnectionCode::decode(&form.info.read(cx).value())
                    .unwrap()
                    .connection_info
                    .work_secret,
                [37; 32]
            );
        });
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "not-a-connection-code".into(),
        ));
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a cmd-v"
        } else {
            "ctrl-a ctrl-v"
        });
        cx.run_until_parked();
        form.update_in(cx, |form, window, cx| {
            assert!(form.error.is_some());
            form.save_editor(true, window, cx);
            assert!(!form.busy, "invalid credentials must not launch a client");
        });
    }

    #[test]
    fn saved_connection_keeps_metadata_route_and_accepts_legacy_credentials() {
        let record = RememberedConnection {
            name: "Override route".into(),
            address: "override.example:43123".into(),
            environment_id: "paste-environment".into(),
            credential_id: CredentialId::new("legacy-key"),
        };
        let code = ConnectionCode::encode("code.example:4001".into(), connection_info()).unwrap();
        let (address, info) = decode_saved_connection(&record, &code).unwrap();
        assert_eq!(address, record.address);
        assert_eq!(info.work_secret, [37; 32]);

        let legacy = serde_json::to_string(&connection_info()).unwrap();
        let (address, info) = decode_saved_connection(&record, &legacy).unwrap();
        assert_eq!(address, record.address);
        assert_eq!(info.work_secret, [37; 32]);
    }

    #[test]
    fn metadata_only_edit_preserves_host_identity_without_credentials() {
        let existing = RememberedConnection {
            name: "Old name".into(),
            address: "old.example:43123".into(),
            environment_id: "existing-environment".into(),
            credential_id: CredentialId::new("existing-key"),
        };
        let updated = metadata_edit_record(&existing, "New name", "new.example:43123".into());
        assert_eq!(updated.name, "New name");
        assert_eq!(updated.address, "new.example:43123");
        assert_eq!(updated.environment_id, existing.environment_id);
        assert_eq!(updated.credential_id, existing.credential_id);
    }

    #[test]
    fn credential_prompt_rejects_code_for_a_different_host() {
        let record = RememberedConnection {
            name: "Saved Host".into(),
            address: "saved.example:43123".into(),
            environment_id: "paste-environment".into(),
            credential_id: CredentialId::new("saved-key"),
        };
        assert!(connection_code_matches_record(&record, &connection_info()));
        let mut other = connection_info();
        other.environment_id = "different-environment".into();
        assert!(!connection_code_matches_record(&record, &other));
    }

    #[test]
    fn editing_replaces_only_the_selected_saved_connection() {
        let first_id = CredentialId::new("first-key");
        let second_id = CredentialId::new("second-key");
        let mut records = vec![
            RememberedConnection {
                name: "First".into(),
                address: "127.0.0.1:4001".into(),
                environment_id: "first".into(),
                credential_id: first_id.clone(),
            },
            RememberedConnection {
                name: "Second".into(),
                address: "127.0.0.1:4002".into(),
                environment_id: "second".into(),
                credential_id: second_id.clone(),
            },
        ];
        update_record(
            &mut records,
            RememberedConnection {
                name: "Renamed first".into(),
                address: "localhost:4003".into(),
                environment_id: "updated".into(),
                credential_id: first_id.clone(),
            },
            Some(&first_id),
        )
        .unwrap();
        assert_eq!(records[0].credential_id, first_id);
        assert_eq!(records[0].name, "Renamed first");
        assert_eq!(records[1].credential_id, second_id);
        assert_eq!(records[1].name, "Second");
        assert_eq!(records[1].address, "127.0.0.1:4002");
    }

    #[test]
    fn legacy_connection_survives_adding_another_host() {
        use crate::config::profile::{
            EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        };
        use yttt_core::model::ids::ProfileId;
        let temporary = tempfile::tempdir().unwrap();
        let profile = AppProfile::scoped(
            ProfileId::new("connections-test"),
            EnvironmentKind::Test,
            ProfilePersistence::Ephemeral,
            temporary.path(),
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        );
        let directory = profile.paths().state.join("client-connections");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("existing-host.json"),
            serde_json::json!({
                "address": "127.0.0.1:4001",
                "environment_id": "first",
                "credential_id": CredentialId::new("first-key"),
            })
            .to_string(),
        )
        .unwrap();
        let mut records = read_connections(&profile).unwrap();
        assert_eq!(records[0].name, "127.0.0.1:4001");
        records.push(RememberedConnection {
            name: "Second".into(),
            address: "127.0.0.1:4002".into(),
            environment_id: "second".into(),
            credential_id: CredentialId::new("second-key"),
        });
        save_connections(&profile, &records).unwrap();
        let restored = read_connections(&profile).unwrap();
        assert_eq!(
            restored
                .iter()
                .map(|record| record.address.as_str())
                .collect::<Vec<_>>(),
            vec!["127.0.0.1:4001", "127.0.0.1:4002"]
        );
    }
}
