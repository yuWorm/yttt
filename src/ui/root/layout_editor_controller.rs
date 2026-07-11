use super::*;

impl RootView {
    pub fn layout_toml_editor_is_open(&self) -> bool {
        self.layout_toml_editor.is_some()
    }

    pub fn layout_toml_editor_path(&self) -> Option<&Path> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.editor().path())
    }

    pub fn layout_toml_editor_value(&self) -> Option<&str> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.editor().value())
    }

    pub fn visible_layout_toml_editor_error(&self) -> Option<&str> {
        self.layout_toml_editor
            .as_ref()
            .and_then(|session| session.editor().error())
    }

    pub fn visible_layout_toml_editor_diagnostics(&self) -> Vec<EditorDiagnostic> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.editor().diagnostics().to_vec())
            .unwrap_or_default()
    }

    pub fn layout_editor_target_kind(&self) -> Option<&'static str> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.target().kind())
    }

    pub fn visible_layout_toml_editor_language_id(&self) -> Option<EditorLanguageId> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.editor().language_id())
    }

    pub fn open_layout_toml_editor(&mut self) -> Result<(), RootViewError> {
        self.open_default_layout_editor()
    }

    pub fn open_default_layout_editor(&mut self) -> Result<(), RootViewError> {
        let path = self.default_layout_state.path().to_path_buf();
        let value = fs::read_to_string(&path).map_err(|source| {
            RootViewError::LayoutTomlEditor(format!(
                "failed to read layout TOML at {}: {source}",
                path.display()
            ))
        })?;

        self.layout_toml_editor = Some(LayoutEditorSession::new(
            LayoutEditorTarget::Default,
            CodeEditorState::new(
                path,
                CodeEditorConfig::new(
                    "Edit default layout TOML",
                    CodeEditorLanguageMode::Explicit(EditorLanguageId::Toml),
                )
                .placeholder_text("Edit layout TOML...")
                .with_rows(24)
                .with_soft_wrap(false),
                value,
            ),
        ));
        self.finish_opening_layout_editor();
        Ok(())
    }

    pub fn open_project_layout_editor(&mut self) -> Result<(), RootViewError> {
        let project_id = self
            .workspace
            .selected_project_id()
            .cloned()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let project = self
            .workspace
            .project(&project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
        let project_path = project.path.clone();
        let effective_layout = project.layout.clone();
        let project_file = self.config_paths.project_layout_file(&project_path);
        let personal_file = self.config_paths.local_layout_file(&project_path);

        let (path, format, value, diagnostic) = if personal_file.exists() {
            let value = read_layout_editor_source(&personal_file)?;
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
                    let message = error.to_string();
                    (
                        personal_file,
                        ProjectLayoutEditorFormat::InvalidPersonal,
                        value,
                        Some(message),
                    )
                }
            }
        } else if project_file.exists() {
            let value = read_layout_editor_source(&project_file)?;
            (
                project_file,
                ProjectLayoutEditorFormat::ProjectConfig,
                value,
                None,
            )
        } else {
            let path = save_local_layout(&self.config_paths, &project_path, &effective_layout)?;
            let value = read_layout_editor_source(&path)?;
            (
                path,
                ProjectLayoutEditorFormat::PersonalReplace,
                value,
                None,
            )
        };

        let mut editor = CodeEditorState::new(
            path.clone(),
            CodeEditorConfig::new(
                "Edit project layout TOML",
                CodeEditorLanguageMode::Explicit(EditorLanguageId::Toml),
            )
            .placeholder_text("Edit layout TOML...")
            .with_rows(24)
            .with_soft_wrap(false),
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
        self.layout_toml_editor = Some(LayoutEditorSession::new(
            LayoutEditorTarget::Project {
                project_id,
                path,
                format,
            },
            editor,
        ));
        self.finish_opening_layout_editor();
        Ok(())
    }

    pub(super) fn finish_opening_layout_editor(&mut self) {
        self.reset_layout_toml_input();
        self.layout_toml_input_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
    }

    pub fn set_layout_toml_editor_value(&mut self, value: impl Into<String>) {
        if let Some(session) = &mut self.layout_toml_editor {
            session.editor_mut().set_value(value);
            self.reset_layout_toml_input();
        }
    }

    pub fn save_layout_toml_editor(&mut self) -> Result<(), RootViewError> {
        let Some(session) = self.layout_toml_editor.clone() else {
            return Ok(());
        };
        let editor = session.editor();

        match session.target() {
            LayoutEditorTarget::Default => {
                let template = match toml::from_str::<DefaultLayoutTemplate>(editor.value()) {
                    Ok(template) => template,
                    Err(error) => {
                        self.set_layout_toml_editor_error(
                            "toml",
                            format!("failed to parse layout TOML: {error}"),
                        );
                        return Ok(());
                    }
                };
                if let Err(error) = template.validate() {
                    self.set_layout_toml_editor_error(
                        "layout",
                        format!("invalid layout TOML: {error}"),
                    );
                    return Ok(());
                }
                if let Err(error) = self.default_layout_state.save(template) {
                    let message = error.to_string();
                    self.set_layout_toml_editor_error("layout", message.clone());
                    self.load_error = Some(message);
                    return Ok(());
                }
            }
            LayoutEditorTarget::Project { path, format, .. } => {
                if let Err((source, message)) =
                    validate_project_editor_source(path, *format, editor.value())
                {
                    self.set_layout_toml_editor_error(source, message);
                    return Ok(());
                }
                if let Err(error) = write_layout_file_atomic(path, editor.value()) {
                    let message = error.to_string();
                    self.set_layout_toml_editor_error("filesystem", message.clone());
                    self.load_error = Some(message);
                    return Ok(());
                }
            }
        }

        self.layout_toml_editor = None;
        self.reset_layout_toml_input();
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn cancel_layout_toml_editor(&mut self) {
        self.layout_toml_editor = None;
        self.reset_layout_toml_input();
        self.queue_selected_terminal_focus();
        self.sync_input_owner_state();
    }

    pub(super) fn layout_toml_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        let editor = self.layout_toml_editor.as_ref()?.editor();

        let input = if let Some(input) = &self.layout_toml_input {
            input.clone()
        } else {
            let input = cx.new(|cx| code_editor_input_state(window, cx, editor));
            let subscription = cx.subscribe_in(&input, window, Self::on_layout_toml_input_event);
            self.layout_toml_input = Some(input.clone());
            self.layout_toml_input_subscription = Some(subscription);
            input
        };

        if self.layout_toml_input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.layout_toml_input_needs_focus = false;
        }

        Some(input)
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
                if let Some(session) = &mut self.layout_toml_editor {
                    session
                        .editor_mut()
                        .set_value(input.read(cx).value().to_string());
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}

fn read_layout_editor_source(path: &Path) -> Result<String, RootViewError> {
    fs::read_to_string(path).map_err(|source| {
        RootViewError::LayoutTomlEditor(format!(
            "failed to read layout TOML at {}: {source}",
            path.display()
        ))
    })
}

fn validate_project_editor_source(
    path: &Path,
    format: ProjectLayoutEditorFormat,
    source: &str,
) -> Result<(), (&'static str, String)> {
    match format {
        ProjectLayoutEditorFormat::ProjectConfig => {
            let layout = toml::from_str::<ProjectLayout>(source)
                .map_err(|error| ("toml", format!("failed to parse layout TOML: {error}")))?;
            layout
                .validate()
                .map_err(|error| ("layout", format!("invalid layout TOML: {error}")))
        }
        ProjectLayoutEditorFormat::PersonalPatch
        | ProjectLayoutEditorFormat::PersonalReplace
        | ProjectLayoutEditorFormat::InvalidPersonal => {
            let personal = parse_personal_layout(path, source)
                .map_err(|error| ("personal-layout", error.to_string()))?;
            match (format, personal) {
                (ProjectLayoutEditorFormat::PersonalPatch, PersonalLayout::Patch(_))
                | (ProjectLayoutEditorFormat::PersonalReplace, PersonalLayout::Replace(_))
                | (ProjectLayoutEditorFormat::InvalidPersonal, _) => Ok(()),
                (ProjectLayoutEditorFormat::PersonalPatch, PersonalLayout::Replace(_)) => Err((
                    "personal-layout",
                    "personal layout editor requires mode = \"patch\"".to_string(),
                )),
                (ProjectLayoutEditorFormat::PersonalReplace, PersonalLayout::Patch(_)) => Err((
                    "personal-layout",
                    "personal layout editor requires mode = \"replace\"".to_string(),
                )),
                (ProjectLayoutEditorFormat::ProjectConfig, _) => unreachable!(),
            }
        }
    }
}
