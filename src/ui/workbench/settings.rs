mod view;
pub(super) use view::{settings_button, settings_overlay};

use super::*;

impl WorkbenchView {
    pub fn system_notifications_enabled(&self) -> bool {
        self.system_notifications_enabled
    }

    pub fn terminal_show_scrollbar(&self) -> bool {
        self.app_settings.terminal.show_scrollbar
    }
    pub fn terminal_cursor_shape(&self) -> TerminalCursorShape {
        self.app_settings.terminal.cursor_shape
    }

    pub fn terminal_cursor_blinking(&self) -> bool {
        self.app_settings.terminal.cursor_blinking
    }

    pub fn terminal_hide_mouse_when_typing(&self) -> bool {
        self.app_settings.terminal.hide_mouse_when_typing
    }

    pub fn terminal_copy_on_select(&self) -> bool {
        self.app_settings.terminal.copy_on_select
    }

    pub fn terminal_osc52_policy(&self) -> TerminalOsc52Policy {
        self.app_settings.terminal.osc52_policy
    }

    pub fn terminal_kitty_keyboard(&self) -> bool {
        self.app_settings.terminal.kitty_keyboard
    }

    pub fn editor_auto_detect_language(&self) -> bool {
        self.app_settings.editor.auto_detect_language
    }

    pub fn editor_default_language(&self) -> &str {
        &self.app_settings.editor.default_language
    }

    pub fn editor_autosave(&self) -> EditorAutosave {
        self.app_settings.editor.autosave
    }

    pub fn editor_lsp_enabled(&self) -> bool {
        self.app_settings.editor.lsp.enabled
    }

    pub fn editor_lsp_command(&self) -> &str {
        &self.app_settings.editor.lsp.command
    }

    pub fn settings_is_open(&self) -> bool {
        self.settings.settings_page.is_open
    }

    pub fn open_settings(&mut self) {
        self.close_palette();
        self.settings.settings_page.is_open = true;
        self.settings.settings_search_input_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
    }

    pub fn close_settings(&mut self) {
        self.settings.settings_page.is_open = false;
        self.reset_settings_search_input();
        self.sync_input_owner_state();
    }

    pub fn set_system_notifications_enabled(
        &mut self,
        enabled: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.notifications.system = enabled;
        save_settings(&self.config_paths, &self.app_settings)?;
        self.system_notifications_enabled = enabled;
        Ok(())
    }

    pub fn new_tab_command_picker_enabled(&self) -> bool {
        self.app_settings.general.new_tab_command_picker_enabled
    }

    pub fn new_tab_commands(&self) -> &[String] {
        &self.app_settings.general.new_tab_commands
    }

    pub fn set_new_tab_command_picker_enabled(
        &mut self,
        enabled: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.general.new_tab_command_picker_enabled = enabled;
        save_settings(&self.config_paths, &self.app_settings)?;
        Ok(())
    }

    pub fn add_new_tab_command(&mut self, command: &str) -> Result<bool, WorkbenchError> {
        let command = command.trim();
        if command.is_empty()
            || self
                .app_settings
                .general
                .new_tab_commands
                .iter()
                .any(|existing| existing == command)
        {
            return Ok(false);
        }

        self.app_settings
            .general
            .new_tab_commands
            .push(command.to_string());
        save_settings(&self.config_paths, &self.app_settings)?;
        self.settings.settings_new_tab_command_input = None;
        Ok(true)
    }

    pub fn remove_new_tab_command(&mut self, index: usize) -> Result<bool, WorkbenchError> {
        if index >= self.app_settings.general.new_tab_commands.len() {
            return Ok(false);
        }

        self.app_settings.general.new_tab_commands.remove(index);
        save_settings(&self.config_paths, &self.app_settings)?;
        Ok(true)
    }

    pub fn set_language(&mut self, language: LanguageSetting) -> Result<(), WorkbenchError> {
        self.app_settings.general.language = language;
        save_settings(&self.config_paths, &self.app_settings)?;
        self.ui_text = ui_text_for_language(language);
        if let Ok(loaded) = load_keybindings(&self.config_paths, &self.command_registry) {
            self.settings.keybinding_warning_lines =
                format_keybinding_warning_lines(&loaded.warnings, &self.ui_text);
        }
        self.reset_palette_input();
        self.reset_settings_search_input();
        Ok(())
    }

    pub fn set_terminal_shell(&mut self, shell: &str) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.shell = shell.to_string();
        self.save_app_settings_and_refresh_runtime()
    }
    pub fn add_custom_terminal_shell(&mut self, shell: &str) -> Result<bool, WorkbenchError> {
        let shell = shell.trim();
        if shell.is_empty() {
            return Ok(false);
        }

        if self
            .app_settings
            .terminal
            .custom_shells
            .iter()
            .all(|existing| existing != shell)
        {
            self.app_settings
                .terminal
                .custom_shells
                .push(shell.to_string());
        }
        self.app_settings.terminal.shell = shell.to_string();
        self.save_app_settings_and_refresh_runtime()?;
        self.settings.settings_shell_select = None;
        self.settings.settings_shell_select_subscription = None;
        self.settings.settings_custom_shell_input = None;
        Ok(true)
    }

    pub fn set_ui_font_family(&mut self, font_family: &str) -> Result<(), WorkbenchError> {
        self.app_settings.general.ui_font_family = font_family.trim().to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_ui_theme_name(&mut self, theme_name: &str) -> Result<(), WorkbenchError> {
        self.app_settings.theme.name = theme_name.to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_theme_name(
        &mut self,
        theme_name: Option<&str>,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.theme.terminal = theme_name.map(ToString::to_string);
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_icon_theme_name(&mut self, theme_name: Option<&str>) -> Result<(), WorkbenchError> {
        self.app_settings.theme.icon_theme = theme_name.map(ToString::to_string);
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn import_zed_themes_from_settings(&mut self) -> Result<(usize, usize), String> {
        let detection = detect_installed_zed_themes();
        if detection.is_empty() {
            return Err(self
                .ui_text
                .get(UiTextKey::SettingsImportZedThemesNone)
                .to_string());
        }
        let imported = import_detected_zed_themes(&detection, &self.config_paths)
            .map_err(|error| error.to_string())?;
        let ui_theme_count = imported.ui_themes.len();
        let icon_theme_count = imported
            .icon_themes
            .iter()
            .map(|package| package.theme_names.len())
            .sum();

        self.settings.settings_ui_theme_select = None;
        self.settings.settings_ui_theme_select_subscription = None;
        self.settings.settings_terminal_theme_select = None;
        self.settings.settings_terminal_theme_select_subscription = None;
        self.settings.settings_icon_theme_select = None;
        self.settings.settings_icon_theme_select_subscription = None;
        self.refresh_theme_runtime_from_settings();
        Ok((ui_theme_count, icon_theme_count))
    }

    pub fn set_terminal_font_family(&mut self, font_family: &str) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.font_family = font_family.to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_font_size(&mut self, font_size: f32) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.font_size = font_size;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_line_height(&mut self, line_height: f32) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.line_height = line_height;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_padding(&mut self, padding: f32) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.padding = padding;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_scrollback(&mut self, scrollback: usize) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.scrollback = scrollback;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_show_scrollbar(
        &mut self,
        show_scrollbar: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.show_scrollbar = show_scrollbar;
        self.save_app_settings_and_refresh_runtime()
    }
    pub fn set_terminal_cursor_shape(
        &mut self,
        cursor_shape: TerminalCursorShape,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.cursor_shape = cursor_shape;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_cursor_blinking(
        &mut self,
        cursor_blinking: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.cursor_blinking = cursor_blinking;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_hide_mouse_when_typing(
        &mut self,
        hide_mouse_when_typing: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.hide_mouse_when_typing = hide_mouse_when_typing;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_copy_on_select(
        &mut self,
        copy_on_select: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.copy_on_select = copy_on_select;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_osc52_policy(
        &mut self,
        osc52_policy: TerminalOsc52Policy,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.osc52_policy = osc52_policy;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_kitty_keyboard(
        &mut self,
        kitty_keyboard: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.terminal.kitty_keyboard = kitty_keyboard;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_auto_detect_language(
        &mut self,
        auto_detect_language: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.editor.auto_detect_language = auto_detect_language;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_font_family(
        &mut self,
        font_family: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.editor.font_family = font_family.to_string();
        self.save_app_settings_and_refresh_runtime()?;
        self.sync_editor_document_appearances(window, cx);
        Ok(())
    }

    pub fn set_editor_font_size(
        &mut self,
        font_size: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.editor.font_size = font_size.clamp(6.0, 72.0);
        self.save_app_settings_and_refresh_runtime()?;
        self.sync_editor_document_appearances(window, cx);
        Ok(())
    }

    pub fn set_editor_line_height(
        &mut self,
        line_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.editor.line_height = line_height.max(1.0);
        self.save_app_settings_and_refresh_runtime()?;
        self.sync_editor_document_appearances(window, cx);
        Ok(())
    }

    pub fn set_editor_tab_size(&mut self, tab_size: usize) -> Result<(), WorkbenchError> {
        self.app_settings.editor.tab_size = tab_size.clamp(1, 16);
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_soft_wrap(
        &mut self,
        soft_wrap: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.editor.soft_wrap = soft_wrap;
        self.save_app_settings_and_refresh_runtime()?;
        self.sync_editor_document_appearances(window, cx);
        Ok(())
    }

    pub fn set_editor_line_numbers(
        &mut self,
        line_numbers: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.editor.line_numbers = line_numbers;
        self.save_app_settings_and_refresh_runtime()?;
        self.sync_editor_document_appearances(window, cx);
        Ok(())
    }

    pub fn set_editor_autosave(&mut self, autosave: EditorAutosave) -> Result<(), WorkbenchError> {
        self.app_settings.editor.autosave = autosave;
        self.save_app_settings_and_refresh_runtime()?;
        if autosave == EditorAutosave::Off {
            self.project
                .project_editor_runtime
                .cancel_all_autosave_tasks();
        }
        Ok(())
    }

    pub fn set_editor_autosave_delay_ms(
        &mut self,
        autosave_delay_ms: u64,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.editor.autosave_delay_ms = autosave_delay_ms.max(50);
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_default_language(
        &mut self,
        default_language: &str,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.editor.default_language = default_language.trim().to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_lsp_enabled(&mut self, enabled: bool) -> Result<(), WorkbenchError> {
        self.app_settings.editor.lsp.enabled = enabled;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_lsp_command(&mut self, command: &str) -> Result<(), WorkbenchError> {
        self.app_settings.editor.lsp.command = command.trim().to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_project_panel_show_hidden(
        &mut self,
        show_hidden: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.project_panel.show_hidden = show_hidden;
        self.save_app_settings_and_refresh_runtime()?;
        let project_ids = self
            .workspace
            .opened_projects()
            .iter()
            .map(|project| project.id.clone())
            .collect::<Vec<_>>();
        for project_id in project_ids {
            self.queue_project_tree_refresh(project_id);
        }
        Ok(())
    }

    pub fn set_project_panel_default_open(
        &mut self,
        default_open: bool,
    ) -> Result<(), WorkbenchError> {
        self.app_settings.project_panel.default_open = default_open;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_settings_search_query(&mut self, query: impl Into<String>) {
        self.settings.settings_page.search_query = query.into();
        let selected_group_visible = self
            .settings
            .settings_page
            .visible_groups(&self.ui_text)
            .iter()
            .any(|group| group.id == self.settings.settings_page.selected_group);
        if !selected_group_visible
            && let Some(first_group) = self
                .settings
                .settings_page
                .visible_groups(&self.ui_text)
                .first()
        {
            self.settings.settings_page.selected_group = first_group.id;
        }
    }

    pub fn select_settings_group(&mut self, group_id: &str) -> Result<(), String> {
        let group = SettingsGroupId::from_id(group_id)
            .ok_or_else(|| format!("Unknown settings group: {group_id}"))?;
        self.settings.settings_page.selected_group = group;
        Ok(())
    }

    pub fn visible_settings_group_titles(&self) -> Vec<&'static str> {
        self.settings
            .settings_page
            .visible_groups(&self.ui_text)
            .into_iter()
            .map(|group| group.title)
            .collect()
    }

    pub fn selected_settings_group_title(&self) -> Option<&'static str> {
        Some(
            self.settings
                .settings_page
                .selected_group
                .title(&self.ui_text),
        )
    }

    pub(super) fn refresh_theme_runtime_from_settings(&mut self) {
        match load_theme_store(&self.config_paths) {
            Ok(loaded) => {
                self.theme_runtime = ThemeRuntime::resolve(&self.app_settings, &loaded.store);
            }
            Err(error) => {
                self.load_error = Some(error.to_string());
            }
        }
        match load_icon_theme(
            &self.config_paths,
            self.app_settings.theme.icon_theme.as_deref(),
        ) {
            Ok(icon_theme) => self.icon_theme = icon_theme,
            Err(error) => {
                self.icon_theme = IconTheme::default();
                self.load_error = Some(error.to_string());
            }
        }
    }

    pub(super) fn save_app_settings_and_refresh_runtime(&mut self) -> Result<(), WorkbenchError> {
        save_settings(&self.config_paths, &self.app_settings)?;
        self.refresh_theme_runtime_from_settings();
        Ok(())
    }

    pub(super) fn sync_terminal_pane_configs(&mut self, cx: &mut Context<Self>) {
        let terminal_config = self.theme_runtime.to_terminal_config();
        let theme = self.theme_runtime.ui;
        for pane in self.terminal.terminal_panes.values() {
            pane.update(cx, |pane, cx| {
                pane.update_terminal_appearance(terminal_config.clone(), theme, cx);
            });
        }
    }

    pub(super) fn sync_editor_document_appearances(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let appearance = EditorAppearance::from(&self.app_settings.editor);
        if let Some(session) = &mut self.overlays.layout_toml_editor {
            session.set_appearance(appearance.clone());
        }
        if let Some(input) = &self.overlays.layout_toml_input {
            input.update(cx, |input, input_cx| {
                input.set_soft_wrap(appearance.soft_wrap, window, input_cx);
                input.set_line_number(appearance.line_numbers, window, input_cx);
            });
        }
        let project_ids = self
            .workspace
            .opened_projects()
            .iter()
            .map(|project| project.id.clone())
            .collect::<Vec<_>>();
        let documents = project_ids
            .iter()
            .flat_map(|project_id| {
                self.project
                    .project_editor_runtime
                    .documents_for_project(project_id)
                    .map(|(_, document)| document.clone())
            })
            .collect::<Vec<_>>();
        for document in documents {
            document.update(cx, |document, document_cx| {
                document.set_appearance(appearance.clone(), window, document_cx);
            });
        }
    }

    pub(super) fn sync_gpui_component_theme(&self, cx: &mut Context<Self>) {
        ComponentTheme::global_mut(cx).apply_config(&Rc::new(
            self.theme_runtime.to_gpui_component_theme_config(),
        ));
    }

    pub(super) fn save_keybindings_editor(&mut self) -> Result<(), WorkbenchError> {
        self.settings.keybindings_editor.save(&self.config_paths)?;
        self.settings.keybinding_warning_lines.clear();
        Ok(())
    }

    pub(super) fn resolved_terminal_shell(&self) -> String {
        let candidates = detect_shell_candidates();
        resolve_default_shell(&self.app_settings.terminal.shell, &candidates)
    }

    pub(super) fn available_theme_names(&self) -> Vec<String> {
        load_theme_store(&self.config_paths)
            .map(|loaded| loaded.store.theme_names())
            .unwrap_or_else(|_| ThemeStore::builtin().theme_names())
    }

    pub(super) fn available_icon_theme_names(&self) -> Vec<String> {
        load_icon_theme_names(&self.config_paths).unwrap_or_default()
    }

    pub(super) fn available_editor_language_names(&self) -> Vec<String> {
        EditorLanguageCatalog::builtin()
            .all_languages()
            .iter()
            .map(|language| language.id.as_str().to_string())
            .collect()
    }

    pub(super) fn settings_number_value(&self, field: SettingsNumberField) -> String {
        match field {
            SettingsNumberField::FontSize => {
                format!("{:.1}", self.app_settings.terminal.font_size)
            }
            SettingsNumberField::LineHeight => {
                format!("{:.2}", self.app_settings.terminal.line_height)
            }
            SettingsNumberField::Padding => {
                format!("{:.1}", self.app_settings.terminal.padding)
            }
            SettingsNumberField::Scrollback => self.app_settings.terminal.scrollback.to_string(),
            SettingsNumberField::EditorFontSize => {
                format!("{:.1}", self.app_settings.editor.font_size)
            }
            SettingsNumberField::EditorLineHeight => {
                format!("{:.2}", self.app_settings.editor.line_height)
            }
            SettingsNumberField::EditorTabSize => self.app_settings.editor.tab_size.to_string(),
            SettingsNumberField::EditorAutosaveDelay => {
                self.app_settings.editor.autosave_delay_ms.to_string()
            }
            SettingsNumberField::ProjectPanelWidth => {
                format!("{:.0}", self.app_settings.project_panel.width)
            }
            SettingsNumberField::ProjectSidebarWidth => format!(
                "{:.0}",
                self.app_settings.project_panel.project_sidebar_width
            ),
        }
    }

    pub(super) fn apply_settings_number_value(
        &mut self,
        field: SettingsNumberField,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkbenchError> {
        let value = value.trim();
        match field {
            SettingsNumberField::FontSize => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_terminal_font_size(value)?;
                }
            }
            SettingsNumberField::LineHeight => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_terminal_line_height(value)?;
                }
            }
            SettingsNumberField::Padding => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_terminal_padding(value)?;
                }
            }
            SettingsNumberField::Scrollback => {
                if let Ok(value) = value.parse::<usize>() {
                    self.set_terminal_scrollback(value)?;
                }
            }
            SettingsNumberField::EditorFontSize => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_editor_font_size(value, window, cx)?;
                }
            }
            SettingsNumberField::EditorLineHeight => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_editor_line_height(value, window, cx)?;
                }
            }
            SettingsNumberField::EditorTabSize => {
                if let Ok(value) = value.parse::<usize>() {
                    self.set_editor_tab_size(value)?;
                }
            }
            SettingsNumberField::EditorAutosaveDelay => {
                if let Ok(value) = value.parse::<u64>() {
                    self.set_editor_autosave_delay_ms(value)?;
                }
            }
            SettingsNumberField::ProjectPanelWidth => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_project_panel_width(value)?;
                }
            }
            SettingsNumberField::ProjectSidebarWidth => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_project_sidebar_width(value)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn stepped_settings_number_value(
        &self,
        field: SettingsNumberField,
        value: &str,
        action: StepAction,
    ) -> String {
        let sign = match action {
            StepAction::Increment => 1.0,
            StepAction::Decrement => -1.0,
        };
        match field {
            SettingsNumberField::FontSize => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.terminal.font_size);
                format!("{:.1}", (value + sign).max(1.0))
            }
            SettingsNumberField::LineHeight => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.terminal.line_height);
                format!("{:.2}", (value + sign * 0.05).max(0.5))
            }
            SettingsNumberField::Padding => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.terminal.padding);
                format!("{:.1}", (value + sign).max(0.0))
            }
            SettingsNumberField::Scrollback => {
                let value = value
                    .trim()
                    .parse::<isize>()
                    .unwrap_or(self.app_settings.terminal.scrollback as isize);
                ((value + (sign as isize) * 1000).max(1000)).to_string()
            }
            SettingsNumberField::EditorFontSize => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.editor.font_size);
                format!("{:.1}", (value + sign).clamp(6.0, 72.0))
            }
            SettingsNumberField::EditorLineHeight => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.editor.line_height);
                format!("{:.2}", (value + sign * 0.05).max(1.0))
            }
            SettingsNumberField::EditorTabSize => {
                let value = value
                    .trim()
                    .parse::<isize>()
                    .unwrap_or(self.app_settings.editor.tab_size as isize);
                (value + sign as isize).clamp(1, 16).to_string()
            }
            SettingsNumberField::EditorAutosaveDelay => {
                let value = value
                    .trim()
                    .parse::<isize>()
                    .unwrap_or(self.app_settings.editor.autosave_delay_ms as isize);
                (value + (sign as isize) * 50).max(50).to_string()
            }
            SettingsNumberField::ProjectPanelWidth => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.project_panel.width);
                format!(
                    "{:.0}",
                    (value + sign * 10.0)
                        .clamp(PROJECT_FILE_PANEL_MIN_WIDTH, PROJECT_FILE_PANEL_MAX_WIDTH)
                )
            }
            SettingsNumberField::ProjectSidebarWidth => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.project_panel.project_sidebar_width);
                format!(
                    "{:.0}",
                    (value + sign * 10.0)
                        .clamp(PROJECT_SIDEBAR_MIN_WIDTH, PROJECT_SIDEBAR_MAX_WIDTH)
                )
            }
        }
    }

    pub(super) fn settings_number_field_for_input(
        &self,
        input: &Entity<InputState>,
    ) -> Option<SettingsNumberField> {
        self.settings
            .settings_number_inputs
            .iter()
            .find_map(|(field, entity)| (entity.entity_id() == input.entity_id()).then_some(*field))
    }

    pub(super) fn settings_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        if !self.settings.settings_page.is_open {
            return None;
        }

        let input = if let Some(input) = &self.settings.settings_search_input {
            input.clone()
        } else {
            let query = self.settings.settings_page.search_query.clone();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(self.ui_text.get(UiTextKey::SettingsSearchPlaceholder))
                    .default_value(query)
            });
            let subscription =
                cx.subscribe_in(&input, window, Self::on_settings_search_input_event);
            self.settings.settings_search_input = Some(input.clone());
            self.settings.settings_search_input_subscription = Some(subscription);
            input
        };

        if self.settings.settings_search_input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.settings.settings_search_input_needs_focus = false;
        }

        Some(input)
    }

    pub(super) fn settings_language_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let items = language_setting_labels();
        let selected = language_setting_label(self.app_settings.general.language).to_string();

        if let Some(select) = &self.settings.settings_language_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx
                .new(|cx| SelectState::new(SearchableVec::new(items), selected_index, window, cx));
            let subscription =
                cx.subscribe_in(&select, window, Self::on_settings_language_select_event);
            self.settings.settings_language_select = Some(select.clone());
            self.settings.settings_language_select_subscription = Some(subscription);
            select
        }
    }

    pub(super) fn settings_custom_shell_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(input) = &self.settings.settings_custom_shell_input {
            return input.clone();
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(self.ui_text.get(UiTextKey::SettingsCustomShellPlaceholder))
        });
        self.settings.settings_custom_shell_input = Some(input.clone());
        input
    }

    pub(super) fn settings_new_tab_command_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(input) = &self.settings.settings_new_tab_command_input {
            return input.clone();
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                self.ui_text
                    .get(UiTextKey::SettingsNewTabCommandPlaceholder),
            )
        });
        self.settings.settings_new_tab_command_input = Some(input.clone());
        input
    }

    pub(super) fn settings_shell_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let mut items = vec!["Auto".to_string()];
        for shell in &self.app_settings.terminal.custom_shells {
            push_unique_string(&mut items, shell.clone());
        }
        for shell in detect_shell_candidates() {
            push_unique_string(&mut items, shell);
        }
        let selected = if self.app_settings.terminal.shell == crate::config::settings::AUTO_SHELL {
            "Auto".to_string()
        } else {
            self.app_settings.terminal.shell.clone()
        };
        if selected != "Auto" {
            push_unique_string(&mut items, selected.clone());
        }

        if let Some(select) = &self.settings.settings_shell_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx
                .new(|cx| SelectState::new(SearchableVec::new(items), selected_index, window, cx));
            let subscription =
                cx.subscribe_in(&select, window, Self::on_settings_shell_select_event);
            self.settings.settings_shell_select = Some(select.clone());
            self.settings.settings_shell_select_subscription = Some(subscription);
            select
        }
    }

    pub(super) fn settings_ui_font_family_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsFontFamilySelectState> {
        if let Some(select) = &self.settings.settings_ui_font_family_select {
            return select.clone();
        }

        let selected = font_family_option_for_setting(&self.app_settings.general.ui_font_family);
        let items = font_family_options_from_system(
            &self.app_settings.general.ui_font_family,
            cx.text_system().all_font_names(),
        );
        let selected_index = selected_index_for_settings_option(&items, &selected);
        let select = cx.new(|cx| {
            SelectState::new(FontFamilyOptions::new(items), selected_index, window, cx)
                .searchable(true)
        });
        let subscription = cx.subscribe_in(
            &select,
            window,
            Self::on_settings_ui_font_family_select_event,
        );
        self.settings.settings_ui_font_family_select = Some(select.clone());
        self.settings.settings_ui_font_family_select_subscription = Some(subscription);
        select
    }

    pub(super) fn settings_ui_theme_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let items = self.available_theme_names();
        let selected = self.theme_runtime.theme_name.clone();

        if let Some(select) = &self.settings.settings_ui_theme_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), selected_index, window, cx)
                    .searchable(true)
            });
            let subscription =
                cx.subscribe_in(&select, window, Self::on_settings_ui_theme_select_event);
            self.settings.settings_ui_theme_select = Some(select.clone());
            self.settings.settings_ui_theme_select_subscription = Some(subscription);
            select
        }
    }

    pub(super) fn settings_icon_theme_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let mut items = vec![ICON_THEME_BUILTIN.to_string()];
        for theme_name in self.available_icon_theme_names() {
            push_unique_string(&mut items, theme_name);
        }
        let selected = self
            .app_settings
            .theme
            .icon_theme
            .clone()
            .unwrap_or_else(|| ICON_THEME_BUILTIN.to_string());
        push_unique_string(&mut items, selected.clone());

        if let Some(select) = &self.settings.settings_icon_theme_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), selected_index, window, cx)
                    .searchable(true)
            });
            let subscription =
                cx.subscribe_in(&select, window, Self::on_settings_icon_theme_select_event);
            self.settings.settings_icon_theme_select = Some(select.clone());
            self.settings.settings_icon_theme_select_subscription = Some(subscription);
            select
        }
    }

    pub(super) fn settings_terminal_theme_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let mut items = vec![TERMINAL_THEME_FOLLOW_UI.to_string()];
        for theme_name in self.available_theme_names() {
            push_unique_string(&mut items, theme_name);
        }
        let selected = self
            .app_settings
            .theme
            .terminal
            .clone()
            .unwrap_or_else(|| TERMINAL_THEME_FOLLOW_UI.to_string());

        if let Some(select) = &self.settings.settings_terminal_theme_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), selected_index, window, cx)
                    .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                Self::on_settings_terminal_theme_select_event,
            );
            self.settings.settings_terminal_theme_select = Some(select.clone());
            self.settings.settings_terminal_theme_select_subscription = Some(subscription);
            select
        }
    }

    pub(super) fn settings_terminal_cursor_shape_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let items = terminal_cursor_shape_labels();
        let selected = terminal_cursor_shape_label(self.terminal_cursor_shape()).to_string();
        if let Some(select) = &self.settings.settings_terminal_cursor_shape_select {
            return select.clone();
        }

        let selected_index = selected_index_for_settings_option(&items, &selected);
        let select =
            cx.new(|cx| SelectState::new(SearchableVec::new(items), selected_index, window, cx));
        let subscription = cx.subscribe_in(
            &select,
            window,
            Self::on_settings_terminal_cursor_shape_select_event,
        );
        self.settings.settings_terminal_cursor_shape_select = Some(select.clone());
        self.settings
            .settings_terminal_cursor_shape_select_subscription = Some(subscription);
        select
    }

    pub(super) fn settings_terminal_osc52_policy_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let items = terminal_osc52_policy_labels();
        let selected = terminal_osc52_policy_label(self.terminal_osc52_policy()).to_string();
        if let Some(select) = &self.settings.settings_terminal_osc52_policy_select {
            return select.clone();
        }

        let selected_index = selected_index_for_settings_option(&items, &selected);
        let select =
            cx.new(|cx| SelectState::new(SearchableVec::new(items), selected_index, window, cx));
        let subscription = cx.subscribe_in(
            &select,
            window,
            Self::on_settings_terminal_osc52_policy_select_event,
        );
        self.settings.settings_terminal_osc52_policy_select = Some(select.clone());
        self.settings
            .settings_terminal_osc52_policy_select_subscription = Some(subscription);
        select
    }

    pub(super) fn settings_editor_language_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let mut items = self.available_editor_language_names();
        let selected = self.app_settings.editor.default_language.clone();
        push_unique_string(&mut items, selected.clone());

        if let Some(select) = &self.settings.settings_editor_language_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), selected_index, window, cx)
                    .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                Self::on_settings_editor_language_select_event,
            );
            self.settings.settings_editor_language_select = Some(select.clone());
            self.settings.settings_editor_language_select_subscription = Some(subscription);
            select
        }
    }

    pub(super) fn settings_font_family_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsFontFamilySelectState> {
        if let Some(select) = &self.settings.settings_font_family_select {
            return select.clone();
        }

        let selected =
            terminal_font_family_option_for_setting(&self.app_settings.terminal.font_family);
        let items = terminal_font_family_options_from_system(
            &self.app_settings.terminal.font_family,
            cx.text_system().all_font_names(),
        );
        let selected_index = selected_index_for_settings_option(&items, &selected);
        let select = cx.new(|cx| {
            SelectState::new(FontFamilyOptions::new(items), selected_index, window, cx)
                .searchable(true)
        });
        let subscription =
            cx.subscribe_in(&select, window, Self::on_settings_font_family_select_event);
        self.settings.settings_font_family_select = Some(select.clone());
        self.settings.settings_font_family_select_subscription = Some(subscription);
        select
    }

    pub(super) fn settings_editor_font_family_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsFontFamilySelectState> {
        if let Some(select) = &self.settings.settings_editor_font_family_select {
            return select.clone();
        }

        let selected = font_family_option_for_setting(&self.app_settings.editor.font_family);
        let items = font_family_options_from_system(
            &self.app_settings.editor.font_family,
            cx.text_system().all_font_names(),
        );
        let selected_index = selected_index_for_settings_option(&items, &selected);
        let select = cx.new(|cx| {
            SelectState::new(FontFamilyOptions::new(items), selected_index, window, cx)
                .searchable(true)
        });
        let subscription = cx.subscribe_in(
            &select,
            window,
            Self::on_settings_editor_font_family_select_event,
        );
        self.settings.settings_editor_font_family_select = Some(select.clone());
        self.settings
            .settings_editor_font_family_select_subscription = Some(subscription);
        select
    }

    pub(super) fn editor_autosave_label(&self, autosave: EditorAutosave) -> &'static str {
        self.ui_text.get(match autosave {
            EditorAutosave::Off => UiTextKey::SettingsEditorAutosaveOff,
            EditorAutosave::OnFocusChange => UiTextKey::SettingsEditorAutosaveOnFocusChange,
            EditorAutosave::AfterDelay => UiTextKey::SettingsEditorAutosaveAfterDelay,
        })
    }

    pub(super) fn settings_editor_autosave_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let items = [
            EditorAutosave::Off,
            EditorAutosave::OnFocusChange,
            EditorAutosave::AfterDelay,
        ]
        .into_iter()
        .map(|autosave| self.editor_autosave_label(autosave).to_string())
        .collect::<Vec<_>>();
        let selected = self.editor_autosave_label(self.app_settings.editor.autosave);

        if let Some(select) = &self.settings.settings_editor_autosave_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, selected);
            let select = cx
                .new(|cx| SelectState::new(SearchableVec::new(items), selected_index, window, cx));
            let subscription = cx.subscribe_in(
                &select,
                window,
                Self::on_settings_editor_autosave_select_event,
            );
            self.settings.settings_editor_autosave_select = Some(select.clone());
            self.settings.settings_editor_autosave_select_subscription = Some(subscription);
            select
        }
    }

    pub(super) fn settings_number_input(
        &mut self,
        field: SettingsNumberField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(input) = self.settings.settings_number_inputs.get(&field) {
            return input.clone();
        }

        let value = self.settings_number_value(field);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        let input_subscription =
            cx.subscribe_in(&input, window, Self::on_settings_number_input_event);
        let step_subscription =
            cx.subscribe_in(&input, window, Self::on_settings_number_step_event);
        self.settings
            .settings_number_inputs
            .insert(field, input.clone());
        self.settings
            .settings_number_input_subscriptions
            .insert(field, vec![input_subscription, step_subscription]);
        input
    }

    pub(super) fn on_settings_search_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.set_settings_search_query(input.read(cx).value().to_string());
                cx.notify();
            }
            InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    pub(super) fn on_settings_language_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let Some(language) = language_setting_from_label(value) else {
            return;
        };
        if let Err(error) = self.set_language(language) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    pub(super) fn on_settings_shell_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let shell = if value == "Auto" {
            crate::config::settings::AUTO_SHELL
        } else {
            value.as_str()
        };
        if let Err(error) = self.set_terminal_shell(shell) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    pub(super) fn on_settings_ui_font_family_select_event(
        &mut self,
        _select: &Entity<SettingsFontFamilySelectState>,
        event: &SelectEvent<FontFamilyOptions>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let font_family = font_family_setting_from_option(value.as_ref());
        if let Err(error) = self.set_ui_font_family(&font_family) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    pub(super) fn on_settings_ui_theme_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        if let Err(error) = self.set_ui_theme_name(value) {
            self.load_error = Some(error.to_string());
        }
        self.sync_gpui_component_theme(cx);
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }

    pub(super) fn on_settings_icon_theme_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let icon_theme = (value != ICON_THEME_BUILTIN).then_some(value.as_str());
        if let Err(error) = self.set_icon_theme_name(icon_theme) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    pub(super) fn on_settings_terminal_theme_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let terminal_theme = (value != TERMINAL_THEME_FOLLOW_UI).then_some(value.as_str());
        if let Err(error) = self.set_terminal_theme_name(terminal_theme) {
            self.load_error = Some(error.to_string());
        }
        self.sync_gpui_component_theme(cx);
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }

    pub(super) fn on_settings_terminal_cursor_shape_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let Some(cursor_shape) = terminal_cursor_shape_from_label(value) else {
            return;
        };
        if let Err(error) = self.set_terminal_cursor_shape(cursor_shape) {
            self.load_error = Some(error.to_string());
        }
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }

    pub(super) fn on_settings_terminal_osc52_policy_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let Some(policy) = terminal_osc52_policy_from_label(value) else {
            return;
        };
        if let Err(error) = self.set_terminal_osc52_policy(policy) {
            self.load_error = Some(error.to_string());
        }
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }

    pub(super) fn on_settings_editor_language_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        if let Err(error) = self.set_editor_default_language(value) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    pub(super) fn on_settings_font_family_select_event(
        &mut self,
        _select: &Entity<SettingsFontFamilySelectState>,
        event: &SelectEvent<FontFamilyOptions>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let font_family = terminal_font_family_setting_from_option(value.as_ref());
        if let Err(error) = self.set_terminal_font_family(&font_family) {
            self.load_error = Some(error.to_string());
        }
        self.sync_gpui_component_theme(cx);
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }

    pub(super) fn on_settings_editor_font_family_select_event(
        &mut self,
        _select: &Entity<SettingsFontFamilySelectState>,
        event: &SelectEvent<FontFamilyOptions>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let font_family = font_family_setting_from_option(value.as_ref());
        if let Err(error) = self.set_editor_font_family(&font_family, window, cx) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    pub(super) fn on_settings_editor_autosave_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let autosave = [
            EditorAutosave::Off,
            EditorAutosave::OnFocusChange,
            EditorAutosave::AfterDelay,
        ]
        .into_iter()
        .find(|autosave| self.editor_autosave_label(*autosave) == value);
        let Some(autosave) = autosave else {
            return;
        };
        if let Err(error) = self.set_editor_autosave(autosave) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    pub(super) fn on_settings_number_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change | InputEvent::PressEnter { .. } | InputEvent::Blur => {
                let Some(field) = self.settings_number_field_for_input(input) else {
                    return;
                };
                let value = input.read(cx).value().to_string();
                if let Err(error) = self.apply_settings_number_value(field, &value, window, cx) {
                    self.load_error = Some(error.to_string());
                }
                self.sync_gpui_component_theme(cx);
                self.sync_terminal_pane_configs(cx);
                cx.notify();
            }
            InputEvent::Focus => {}
        }
    }

    pub(super) fn on_settings_number_step_event(
        &mut self,
        input: &Entity<InputState>,
        event: &NumberInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.settings_number_field_for_input(input) else {
            return;
        };
        let NumberInputEvent::Step(action) = event;
        let value = input.read(cx).value().to_string();
        let stepped = self.stepped_settings_number_value(field, &value, *action);
        input.update(cx, |input, cx| {
            input.set_value(stepped.clone(), window, cx);
        });
        if let Err(error) = self.apply_settings_number_value(field, &stepped, window, cx) {
            self.load_error = Some(error.to_string());
        }
        self.sync_gpui_component_theme(cx);
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }
}
