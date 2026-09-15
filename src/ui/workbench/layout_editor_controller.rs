use super::layout_editor::BarEditorRegion;
use crate::config::bars::ShellBarsSettings;

use super::*;

impl WorkbenchView {
    pub fn layout_toml_editor_is_open(&self) -> bool {
        self.overlays.layout_toml_editor.is_some()
    }

    pub fn layout_toml_editor_path(&self) -> Option<&Path> {
        self.overlays
            .layout_toml_editor
            .as_ref()
            .map(|session| session.editor().path())
    }

    pub fn layout_toml_editor_value(&self) -> Option<&str> {
        self.overlays
            .layout_toml_editor
            .as_ref()
            .map(|session| session.editor().value())
    }

    pub fn visible_layout_toml_editor_error(&self) -> Option<&str> {
        self.overlays
            .layout_toml_editor
            .as_ref()
            .and_then(|session| session.editor().error())
    }

    pub fn visible_layout_toml_editor_diagnostics(&self) -> Vec<EditorDiagnostic> {
        self.overlays
            .layout_toml_editor
            .as_ref()
            .map(|session| session.editor().diagnostics().to_vec())
            .unwrap_or_default()
    }

    pub fn layout_editor_target_kind(&self) -> Option<&'static str> {
        self.overlays
            .layout_toml_editor
            .as_ref()
            .map(|session| session.target().kind())
    }

    pub fn visible_layout_toml_editor_language_id(&self) -> Option<EditorLanguageId> {
        self.overlays
            .layout_toml_editor
            .as_ref()
            .map(|session| session.editor().language_id())
    }

    pub fn visible_layout_toml_editor_config(&self) -> Option<&CodeEditorConfig> {
        self.overlays
            .layout_toml_editor
            .as_ref()
            .map(|session| session.editor().config())
    }

    pub fn visible_layout_toml_editor_appearance(&self) -> Option<&EditorAppearance> {
        self.overlays
            .layout_toml_editor
            .as_ref()
            .map(LayoutEditorSession::appearance)
    }

    pub fn open_layout_toml_editor(&mut self) -> Result<(), WorkbenchError> {
        self.open_default_layout_editor()
    }

    pub fn open_bars_toml_editor(&mut self) -> Result<(), WorkbenchError> {
        let path = crate::config::scope::device_bars_file()
            .or_else(|| {
                self.config_paths
                    .is_test_fixture()
                    .then(|| self.config_paths.bars_file())
            })
            .ok_or_else(|| {
                WorkbenchError::SettingsUnavailable("Device preferences are not bound.".to_string())
            })?;
        let value = toml::to_string_pretty(&self.app_settings.bars).map_err(|source| {
            WorkbenchError::LayoutTomlEditor(format!(
                "failed to serialize bars TOML for {}: {source}",
                path.display()
            ))
        })?;

        self.overlays.layout_toml_editor = Some(LayoutEditorSession::new(
            LayoutEditorTarget::Bars,
            CodeEditorState::new(
                path,
                CodeEditorConfig::new(
                    self.ui_text.get(UiTextKey::BarsEditorTitle),
                    CodeEditorLanguageMode::Explicit(EditorLanguageId::Toml),
                )
                .placeholder_text(self.ui_text.get(UiTextKey::BarsEditorPlaceholder))
                .with_rows(24)
                .with_editor_settings(&self.app_settings.editor),
                value,
            ),
            EditorAppearance::from(&self.app_settings.editor),
        ));
        self.finish_opening_layout_editor();
        Ok(())
    }

    pub fn open_default_layout_editor(&mut self) -> Result<(), WorkbenchError> {
        let path = self.default_layout_state.path().to_path_buf();
        let value = match optional_layout_editor_source(
            crate::config::storage::read_to_string(&path),
            &path,
            &self.ui_text,
        )? {
            Some(value) => value,
            None => serialize_layout_editor_source(self.default_layout_state.template(), &path)?,
        };

        self.overlays.layout_toml_editor = Some(LayoutEditorSession::new(
            LayoutEditorTarget::Default,
            CodeEditorState::new(
                path,
                CodeEditorConfig::new(
                    self.ui_text.get(UiTextKey::CommandLayoutDefaultEditTitle),
                    CodeEditorLanguageMode::Explicit(EditorLanguageId::Toml),
                )
                .placeholder_text(self.ui_text.get(UiTextKey::LayoutEditorPlaceholder))
                .with_rows(24)
                .with_editor_settings(&self.app_settings.editor),
                value,
            ),
            EditorAppearance::from(&self.app_settings.editor),
        ));
        self.finish_opening_layout_editor();
        Ok(())
    }

    pub fn open_project_layout_editor(&mut self) -> Result<(), WorkbenchError> {
        if !self.require_shared_mutation_control() {
            return Ok(());
        }
        let project_id = self
            .workspace
            .selected_project_id()
            .cloned()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let project = self
            .workspace
            .project(&project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
        let project_path = project
            .location
            .local_path()
            .cloned()
            .ok_or(WorkbenchError::UnsupportedRemoteProject)?;
        let effective_layout = project.layout.clone();
        let project_file = self.config_paths.project_layout_file(&project_path);
        let personal_file = self.config_paths.local_layout_file(&project_path);

        let (path, format, value, diagnostic) = if let Some(value) = optional_layout_editor_source(
            crate::config::storage::read_to_string(&personal_file),
            &personal_file,
            &self.ui_text,
        )? {
            match parse_personal_layout(&personal_file, &value) {
                Ok(PersonalLayout::Patch(_)) => (
                    personal_file,
                    ProjectLayoutEditorFormat::PersonalPatch,
                    value,
                    None,
                ),
                Ok(PersonalLayout::Replace(_)) => (
                    personal_file,
                    ProjectLayoutEditorFormat::PersonalReplace,
                    value,
                    None,
                ),
                Err(error) => {
                    let message = localized_layout_editor_error(
                        &self.ui_text,
                        UiTextKey::LayoutEditorPersonalInvalid,
                        error,
                    );
                    (
                        personal_file,
                        ProjectLayoutEditorFormat::InvalidPersonal,
                        value,
                        Some(message),
                    )
                }
            }
        } else if let Some(value) = optional_layout_editor_source(
            crate::config::storage::read_project_config(
                &project_path,
                yttt_protocol::workspace::WorkspaceProjectConfigFile::Layout,
                &project_file,
            ),
            &project_file,
            &self.ui_text,
        )? {
            (
                project_file,
                ProjectLayoutEditorFormat::ProjectConfig,
                value,
                None,
            )
        } else {
            let value = crate::config::layout_loader::serialize_personal_replace(&effective_layout)
                .map_err(|error| {
                    WorkbenchError::LayoutTomlEditor(format!(
                        "Failed to prepare layout TOML ({}): {error}",
                        personal_file.display(),
                    ))
                })?;
            (
                personal_file,
                ProjectLayoutEditorFormat::PersonalReplace,
                value,
                None,
            )
        };

        let mut editor = CodeEditorState::new(
            path.clone(),
            CodeEditorConfig::new(
                self.ui_text.get(UiTextKey::CommandLayoutProjectEditTitle),
                CodeEditorLanguageMode::Explicit(EditorLanguageId::Toml),
            )
            .placeholder_text(self.ui_text.get(UiTextKey::LayoutEditorPlaceholder))
            .with_rows(24)
            .with_editor_settings(&self.app_settings.editor),
            value,
        );
        if let Some(message) = diagnostic {
            editor.set_error(message.clone());
            editor.set_diagnostics(vec![EditorDiagnostic::new(
                EditorDiagnosticSeverity::Error,
                "personal-layout",
                message,
            )]);
        }
        self.overlays.layout_toml_editor = Some(LayoutEditorSession::new(
            LayoutEditorTarget::Project {
                project_id,
                path,
                format,
            },
            editor,
            EditorAppearance::from(&self.app_settings.editor),
        ));
        self.finish_opening_layout_editor();
        Ok(())
    }

    pub(super) fn finish_opening_layout_editor(&mut self) {
        self.reset_layout_toml_input();
        self.reset_bar_component_search_input();
        self.overlays.layout_toml_input_needs_focus = true;
        self.load_error = None;
        self.auxiliary_windows
            .request(AuxiliaryWindowKind::LayoutEditor);
        self.sync_input_owner_state();
    }

    pub fn set_layout_toml_editor_value(&mut self, value: impl Into<String>) {
        if let Some(session) = &mut self.overlays.layout_toml_editor {
            session.editor_mut().set_value(value);
            self.reset_layout_toml_input();
        }
    }

    pub fn save_layout_toml_editor(&mut self) -> Result<(), WorkbenchError> {
        let Some(session) = self.overlays.layout_toml_editor.clone() else {
            return Ok(());
        };
        if !matches!(session.target(), LayoutEditorTarget::Bars)
            && !self.require_shared_mutation_control()
        {
            return Ok(());
        }
        let editor = session.editor();

        match session.target() {
            LayoutEditorTarget::Bars => {
                let bars = match validate_bars_editor_source(editor.value(), &self.ui_text) {
                    Ok(bars) => bars,
                    Err((source, message)) => {
                        self.set_layout_toml_editor_error(source, message);
                        return Ok(());
                    }
                };
                self.app_settings.bars = bars;
                match self.persist_app_settings(true) {
                    Ok(true) => {}
                    Ok(false) => return Ok(()),
                    Err(error) => {
                        let message = localized_layout_editor_error(
                            &self.ui_text,
                            UiTextKey::BarsEditorSaveFailed,
                            error,
                        );
                        self.set_layout_toml_editor_error("bars", message.clone());
                        self.load_error = Some(message);
                        return Ok(());
                    }
                }
            }
            LayoutEditorTarget::Default => {
                let template = match toml::from_str::<DefaultLayoutTemplate>(editor.value()) {
                    Ok(template) => template,
                    Err(error) => {
                        let message = localized_layout_editor_error(
                            &self.ui_text,
                            UiTextKey::LayoutEditorParseFailed,
                            error,
                        );
                        self.set_layout_toml_editor_error("toml", message);
                        return Ok(());
                    }
                };
                if let Err(error) = template.validate() {
                    let message = localized_layout_editor_error(
                        &self.ui_text,
                        UiTextKey::LayoutEditorValidationFailed,
                        error,
                    );
                    self.set_layout_toml_editor_error("layout", message);
                    return Ok(());
                }
                if let Err(error) = self.default_layout_state.save(template) {
                    let message = localized_layout_editor_error(
                        &self.ui_text,
                        UiTextKey::LayoutEditorSaveFailed,
                        error,
                    );
                    self.set_layout_toml_editor_error("layout", message.clone());
                    self.load_error = Some(message);
                    return Ok(());
                }
            }
            LayoutEditorTarget::Project { path, format, .. } => {
                if let Err((source, message)) =
                    validate_project_editor_source(path, *format, editor.value(), &self.ui_text)
                {
                    self.set_layout_toml_editor_error(source, message);
                    return Ok(());
                }
                if let Err(error) = write_layout_file_atomic(path, editor.value()) {
                    let message = localized_layout_editor_error(
                        &self.ui_text,
                        UiTextKey::LayoutEditorSaveFailed,
                        error,
                    );
                    self.set_layout_toml_editor_error("filesystem", message.clone());
                    self.load_error = Some(message);
                    return Ok(());
                }
            }
        }

        self.overlays.layout_toml_editor = None;
        self.reset_layout_toml_input();
        self.reset_bar_component_search_input();
        self.load_error = None;
        self.restore_layout_editor_owner();
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn save_layout_toml_editor_with_runtime_refresh(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkbenchError> {
        let saving_bars = self
            .overlays
            .layout_toml_editor
            .as_ref()
            .is_some_and(|session| matches!(session.target(), LayoutEditorTarget::Bars));
        self.save_layout_toml_editor()?;
        if saving_bars && !self.layout_toml_editor_is_open() {
            self.sync_performance_monitoring(cx);
        }
        Ok(())
    }

    pub fn cancel_layout_toml_editor(&mut self) {
        self.overlays.layout_toml_editor = None;
        self.reset_layout_toml_input();
        self.reset_bar_component_search_input();
        self.restore_layout_editor_owner();
        self.sync_input_owner_state();
    }

    fn restore_layout_editor_owner(&mut self) {
        if self.auxiliary_windows.active == Some(AuxiliaryWindowKind::LayoutEditor) {
            self.auxiliary_windows.active = self
                .settings
                .settings_page
                .is_open
                .then_some(AuxiliaryWindowKind::Settings);
        }
    }

    pub(super) fn layout_toml_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        let editor = self.overlays.layout_toml_editor.as_ref()?.editor();

        let input = if let Some(input) = &self.overlays.layout_toml_input {
            input.clone()
        } else {
            let input = cx.new(|cx| code_editor_input_state(window, cx, editor));
            let subscription = cx.subscribe_in(&input, window, Self::on_layout_toml_input_event);
            self.overlays.layout_toml_input = Some(input.clone());
            self.overlays.layout_toml_input_subscription = Some(subscription);
            input
        };

        if self.overlays.layout_toml_input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.overlays.layout_toml_input_needs_focus = false;
        }

        Some(input)
    }

    pub(super) fn bar_component_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        let query = self
            .overlays
            .layout_toml_editor
            .as_ref()
            .filter(|session| matches!(session.target(), LayoutEditorTarget::Bars))
            .map(|session| session.bar_component_query().to_string())?;
        let input = if let Some(input) = &self.overlays.bar_component_search_input {
            input.clone()
        } else {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(self.ui_text.get(UiTextKey::BarsEditorSearchComponents))
                    .default_value(query)
            });
            let subscription =
                cx.subscribe_in(&input, window, Self::on_bar_component_search_input_event);
            self.overlays.bar_component_search_input = Some(input.clone());
            self.overlays.bar_component_search_input_subscription = Some(subscription);
            input
        };
        Some(input)
    }

    pub(super) fn set_bar_insert_region(&mut self, region: BarEditorRegion) {
        if let Some(session) = &mut self.overlays.layout_toml_editor
            && matches!(session.target(), LayoutEditorTarget::Bars)
        {
            session.set_bar_insert_region(region);
        }
    }

    pub(super) fn insert_bar_component(&mut self, component: &str) {
        let Some((source, region)) =
            self.overlays
                .layout_toml_editor
                .as_ref()
                .and_then(|session| {
                    matches!(session.target(), LayoutEditorTarget::Bars).then(|| {
                        (
                            session.editor().value().to_string(),
                            session.bar_insert_region(),
                        )
                    })
                })
        else {
            return;
        };
        let mut bars = match validate_bars_editor_source(&source, &self.ui_text) {
            Ok(bars) => bars,
            Err((source, message)) => {
                self.set_layout_toml_editor_error(source, message);
                return;
            }
        };
        let Ok(mut nodes) = crate::config::bars::parse_bar_template(&format!("[{component}]"))
        else {
            return;
        };
        let Some(module) = nodes.pop() else {
            return;
        };
        match region {
            BarEditorRegion::WindowLeft => bars.window.layout.left.push(module),
            BarEditorRegion::WindowCenter => bars.window.layout.center.push(module),
            BarEditorRegion::WindowRight => bars.window.layout.right.push(module),
            BarEditorRegion::StatusLeft => bars.status.layout.left.push(module),
            BarEditorRegion::StatusCenter => bars.status.layout.center.push(module),
            BarEditorRegion::StatusRight => bars.status.layout.right.push(module),
        }
        match toml::to_string_pretty(&bars) {
            Ok(source) => self.set_layout_toml_editor_value(source),
            Err(error) => self.set_layout_toml_editor_error(
                region.path(),
                localized_layout_editor_error(
                    &self.ui_text,
                    UiTextKey::BarsEditorSaveFailed,
                    error,
                ),
            ),
        }
    }

    fn reset_bar_component_search_input(&mut self) {
        self.overlays.bar_component_search_input = None;
        self.overlays.bar_component_search_input_subscription = None;
    }

    pub(super) fn on_layout_toml_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let value = input.read(cx).value().to_string();
                let is_bars_editor = self
                    .overlays
                    .layout_toml_editor
                    .as_ref()
                    .is_some_and(|session| matches!(session.target(), LayoutEditorTarget::Bars));
                if let Some(session) = &mut self.overlays.layout_toml_editor {
                    session.editor_mut().set_value(value.clone());
                }
                if is_bars_editor {
                    match validate_bars_editor_source(&value, &self.ui_text) {
                        Ok(_) => {
                            if let Some(session) = &mut self.overlays.layout_toml_editor {
                                session.editor_mut().clear_error();
                                session.editor_mut().clear_diagnostics();
                            }
                        }
                        Err((source, message)) => {
                            self.set_layout_toml_editor_error(source, message);
                        }
                    }
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn on_bar_component_search_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change)
            && let Some(session) = &mut self.overlays.layout_toml_editor
            && matches!(session.target(), LayoutEditorTarget::Bars)
        {
            session.set_bar_component_query(input.read(cx).value().to_string());
            cx.notify();
        }
    }
}

fn optional_layout_editor_source(
    source: std::io::Result<String>,
    path: &Path,
    ui_text: &UiText,
) -> Result<Option<String>, WorkbenchError> {
    match source {
        Ok(source) => Ok(Some(source)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(WorkbenchError::LayoutTomlEditor(format!(
            "{} ({}): {source}",
            ui_text.get(UiTextKey::LayoutEditorReadFailed),
            path.display()
        ))),
    }
}

fn serialize_layout_editor_source(
    layout: &impl serde::Serialize,
    path: &Path,
) -> Result<String, WorkbenchError> {
    toml::to_string_pretty(layout).map_err(|source| {
        WorkbenchError::LayoutTomlEditor(format!(
            "failed to serialize layout TOML for {}: {source}",
            path.display()
        ))
    })
}

fn localized_layout_editor_error(
    ui_text: &UiText,
    key: UiTextKey,
    detail: impl std::fmt::Display,
) -> String {
    format!("{}: {detail}", ui_text.get(key))
}

pub(super) fn validate_bars_editor_source(
    source: &str,
    ui_text: &UiText,
) -> Result<ShellBarsSettings, (&'static str, String)> {
    let mut bars = toml::from_str::<ShellBarsSettings>(source).map_err(|error| {
        // Flattened Serde tables can attach a template error to the section header.
        // Recover the actual region from parsed TOML, including multiline/quoted values.
        if let Ok(document) = toml::from_str::<toml::Value>(source) {
            for (host, region, field) in [
                ("window", "left", "window.left"),
                ("window", "center", "window.center"),
                ("window", "right", "window.right"),
                ("status", "left", "status.left"),
                ("status", "center", "status.center"),
                ("status", "right", "status.right"),
            ] {
                if let Some(template) = document
                    .get(host)
                    .and_then(|table| table.get(region))
                    .and_then(toml::Value::as_str)
                    && let Err(message) = crate::config::bars::parse_bar_template(template)
                {
                    return (
                        field,
                        format!(
                            "{}: {field}: {message}",
                            ui_text.get(UiTextKey::BarsEditorValidationFailed)
                        ),
                    );
                }
            }
        }
        (
            "toml",
            localized_bars_editor_parse_error(source, &error, ui_text),
        )
    })?;
    if let Some(issue) = bars.validate().into_iter().next() {
        return Err((
            issue.field,
            format!(
                "{}: {} ({})",
                ui_text.get(UiTextKey::BarsEditorValidationFailed),
                issue.field,
                issue.value
            ),
        ));
    }
    Ok(bars)
}

fn localized_bars_editor_parse_error(
    source: &str,
    error: &toml::de::Error,
    ui_text: &UiText,
) -> String {
    let Some(span) = error.span() else {
        return localized_layout_editor_error(ui_text, UiTextKey::BarsEditorParseFailed, error);
    };
    let offset = span.start.min(source.len());
    let before = source.get(..offset).unwrap_or(source);
    let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map_or(before.chars().count() + 1, |(_, line)| {
            line.chars().count() + 1
        });
    let region = bars_editor_region_at_line(source, line)
        .map(|region| format!("{region}, "))
        .unwrap_or_default();
    format!(
        "{}: {region}line {line}, column {column}: {error}",
        ui_text.get(UiTextKey::BarsEditorParseFailed)
    )
}

fn bars_editor_region_at_line(source: &str, line: usize) -> Option<&'static str> {
    let field = source.lines().nth(line.checked_sub(1)?).and_then(|line| {
        ["left", "center", "right", "enabled"]
            .into_iter()
            .find(|field| line.trim_start().starts_with(*field))
    })?;
    let section = source
        .lines()
        .take(line)
        .filter_map(|line| match line.trim() {
            "[window]" => Some("window"),
            "[status]" => Some("status"),
            _ => None,
        })
        .last()?;
    match (section, field) {
        ("window", "left") => Some("window.left"),
        ("window", "center") => Some("window.center"),
        ("window", "right") => Some("window.right"),
        ("status", "left") => Some("status.left"),
        ("status", "center") => Some("status.center"),
        ("status", "right") => Some("status.right"),
        ("status", "enabled") => Some("status.enabled"),
        _ => None,
    }
}

fn validate_project_editor_source(
    path: &Path,
    format: ProjectLayoutEditorFormat,
    source: &str,
    ui_text: &UiText,
) -> Result<(), (&'static str, String)> {
    match format {
        ProjectLayoutEditorFormat::ProjectConfig => {
            let layout = toml::from_str::<ProjectLayout>(source).map_err(|error| {
                (
                    "toml",
                    localized_layout_editor_error(
                        ui_text,
                        UiTextKey::LayoutEditorParseFailed,
                        error,
                    ),
                )
            })?;
            layout.validate().map_err(|error| {
                (
                    "layout",
                    localized_layout_editor_error(
                        ui_text,
                        UiTextKey::LayoutEditorValidationFailed,
                        error,
                    ),
                )
            })
        }
        ProjectLayoutEditorFormat::PersonalPatch
        | ProjectLayoutEditorFormat::PersonalReplace
        | ProjectLayoutEditorFormat::InvalidPersonal => {
            let personal = parse_personal_layout(path, source).map_err(|error| {
                (
                    "personal-layout",
                    localized_layout_editor_error(
                        ui_text,
                        UiTextKey::LayoutEditorPersonalInvalid,
                        error,
                    ),
                )
            })?;
            match (format, personal) {
                (ProjectLayoutEditorFormat::PersonalPatch, PersonalLayout::Patch(_))
                | (ProjectLayoutEditorFormat::PersonalReplace, PersonalLayout::Replace(_))
                | (ProjectLayoutEditorFormat::InvalidPersonal, _) => Ok(()),
                (ProjectLayoutEditorFormat::PersonalPatch, PersonalLayout::Replace(_)) => Err((
                    "personal-layout",
                    ui_text
                        .get(UiTextKey::LayoutEditorRequiresPatchMode)
                        .to_string(),
                )),
                (ProjectLayoutEditorFormat::PersonalReplace, PersonalLayout::Patch(_)) => Err((
                    "personal-layout",
                    ui_text
                        .get(UiTextKey::LayoutEditorRequiresReplaceMode)
                        .to_string(),
                )),
                (ProjectLayoutEditorFormat::ProjectConfig, _) => unreachable!(),
            }
        }
    }
}
