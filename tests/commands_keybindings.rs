use std::path::PathBuf;

use gpui::Keystroke;
use tempfile::tempdir;
use yttt::commands::{
    ActiveSurface, CommandContext, CommandId, default_registry, dispatch_workspace_command,
};
use yttt::config::{
    keybindings::{
        KEYBINDINGS_SCHEMA_VERSION, Keybinding, KeybindingLoadWarning, KeybindingsConfig,
        default_keybindings, load_keybindings, save_keybindings,
    },
    paths::AppConfigPaths,
};
use yttt::model::{
    ids::ProjectId,
    layout::LayoutNode,
    project::{ProjectDescriptor, ProjectLocation},
    workspace::{TabStartState, Workspace},
};
use yttt::ui::i18n::{Locale, UiText};
use yttt::ui::interaction::actions::{
    BindableActionId, CompiledKeybindingAction, app_startup_keybindings, bindable_registry,
    compile_layered_keybinding_specs, default_ui_keybinding_specs, layered_ui_keybinding_specs,
    runtime_command_for_keystroke,
};
use yttt::ui::interaction::{
    input_owner::InputOwnerKind, key_dispatch::workspace_command_for_keystroke,
};
use yttt::ui::settings::keybinding_display::{
    KeybindingDisplayPlatform, display_keybindings_for_platform, recorded_keybinding,
};
use yttt::ui::settings::keybindings::{
    KeybindingDiagnosticKind, KeybindingEditError, KeybindingOrigin, KeybindingProfile,
    KeybindingsEditorState,
};
use yttt::ui::vim::{
    VIM_CONTROL_CONTEXT, VIM_NORMAL_CONTEXT, VIM_PALETTE_NORMAL_CONTEXT,
    VIM_PROJECT_PANEL_NORMAL_CONTEXT, VIM_PROJECT_TREE_NORMAL_CONTEXT, VIM_PROJECTS_NORMAL_CONTEXT,
};
use yttt::ui::workbench::shell::split_view::visible_pane_titles;
use yttt_terminal::{TERMINAL_HINT_KEY_CONTEXT, TERMINAL_SEARCH_KEY_CONTEXT};

fn local_project(path: PathBuf) -> ProjectDescriptor {
    let location = ProjectLocation::local(path);
    ProjectDescriptor::new(
        ProjectId::from_legacy_location(&location.display_path()),
        location,
    )
}

#[test]
fn default_registry_contains_core_commands() {
    let registry = default_registry();

    assert!(registry.contains(CommandId::ProjectOpen));
    assert!(registry.contains(CommandId::PaneSplitVertical));
    assert!(registry.contains(CommandId::TabRename));
    assert!(registry.contains(CommandId::CommandPaletteOpen));
    assert!(registry.contains(CommandId::SettingsOpen));
    assert!(registry.contains(CommandId::ProjectOpenedPalette));
}

#[test]
fn file_and_project_panel_commands_are_registered() {
    let registry = default_registry();

    for (command, id) in [
        (CommandId::FileFind, "file.find"),
        (CommandId::FileSave, "file.save"),
        (CommandId::ProjectPanelToggle, "project_panel.toggle"),
        (CommandId::ProjectPanelRefresh, "project_panel.refresh"),
    ] {
        assert!(registry.contains(command));
        assert_eq!(command.as_str(), id);
    }

    let config = default_keybindings();
    assert_has_config_binding(&config, "cmd-p", "file.find");
    assert_has_config_binding(&config, "ctrl-p", "file.find");
    assert_has_config_binding(&config, "cmd-shift-p", "command_palette.open");
    assert_has_config_binding(&config, "ctrl-shift-p", "command_palette.open");
    assert_has_config_binding(&config, "cmd-s", "file.save");
    assert_has_config_binding(&config, "ctrl-s", "file.save");
    assert_has_config_binding(&config, "cmd-shift-e", "project_panel.toggle");
    assert_has_config_binding(&config, "ctrl-shift-e", "project_panel.toggle");
    assert_has_ui_binding("cmd-p", "file.find");
    assert_has_ui_binding("ctrl-p", "file.find");
    assert_has_ui_binding("cmd-shift-p", "command_palette.open");
    assert_has_ui_binding("ctrl-shift-p", "command_palette.open");
    assert_has_ui_binding("cmd-s", "file.save");
    assert_has_ui_binding("ctrl-s", "file.save");
    assert_has_ui_binding("cmd-shift-e", "project_panel.toggle");
    assert_has_ui_binding("ctrl-shift-e", "project_panel.toggle");
    assert!(config.conflicts().is_empty());
}

#[test]
fn command_availability_tracks_active_surface() {
    let no_project = CommandContext {
        has_selected_project: false,
        active_surface: ActiveSurface::None,
    };
    let no_surface = CommandContext {
        has_selected_project: true,
        active_surface: ActiveSurface::None,
    };
    let terminal = CommandContext {
        has_selected_project: true,
        active_surface: ActiveSurface::Terminal,
    };
    let file = CommandContext {
        has_selected_project: true,
        active_surface: ActiveSurface::File,
    };

    assert!(
        !CommandId::FileSave
            .availability_for_context(no_project)
            .enabled
    );
    assert!(
        !CommandId::FileSave
            .availability_for_context(no_surface)
            .enabled
    );
    assert!(
        !CommandId::FileSave
            .availability_for_context(terminal)
            .enabled
    );
    assert!(CommandId::FileSave.availability_for_context(file).enabled);
    assert!(!CommandId::FileSave.availability(true).enabled);

    for command in [
        CommandId::ProjectPanelToggle,
        CommandId::ProjectPanelRefresh,
    ] {
        assert!(!command.availability_for_context(no_project).enabled);
        assert!(command.availability_for_context(no_surface).enabled);
        assert!(command.availability_for_context(terminal).enabled);
        assert!(command.availability_for_context(file).enabled);
    }

    assert!(
        !CommandId::TabNew
            .availability_for_context(no_project)
            .enabled
    );
    assert!(
        CommandId::TabNew
            .availability_for_context(no_surface)
            .enabled
    );
    assert!(CommandId::TabNew.availability_for_context(terminal).enabled);
    assert!(!CommandId::TabNew.availability_for_context(file).enabled);

    for command in [CommandId::TabClose, CommandId::TabNext, CommandId::TabPrev] {
        assert!(!command.availability_for_context(no_surface).enabled);
        assert!(command.availability_for_context(terminal).enabled);
        assert!(command.availability_for_context(file).enabled);
    }

    for command in [
        CommandId::PaneFocusLeft,
        CommandId::PaneFocusRight,
        CommandId::PaneFocusUp,
        CommandId::PaneFocusDown,
    ] {
        assert!(!command.availability_for_context(no_surface).enabled);
        assert!(command.availability_for_context(terminal).enabled);
        assert!(command.availability_for_context(file).enabled);
    }

    for command in [
        CommandId::TabRename,
        CommandId::PaneSplitHorizontal,
        CommandId::PaneSplitVertical,
        CommandId::PaneClose,
        CommandId::PaneResizeLeft,
        CommandId::PaneResizeRight,
        CommandId::PaneResizeUp,
        CommandId::PaneResizeDown,
        CommandId::PaneRename,
        CommandId::PanePalette,
    ] {
        assert!(command.availability_for_context(terminal).enabled);
        assert!(!command.availability_for_context(file).enabled);
    }
}

#[test]
fn editor_owner_dispatches_every_command_available_for_files() {
    let file_context = CommandContext {
        has_selected_project: true,
        active_surface: ActiveSurface::File,
    };

    for &command in CommandId::ALL {
        let actual = workspace_command_for_keystroke(
            InputOwnerKind::Editor,
            &Keystroke::parse("cmd-s").unwrap(),
            |_| Some(command),
            |_| true,
        );
        let expected = command
            .availability_for_context(file_context)
            .enabled
            .then_some(command);

        assert_eq!(
            actual, expected,
            "{command:?} editor dispatch must match file availability"
        );
    }
}

#[test]
fn modal_input_owners_block_project_file_save() {
    for owner in [
        InputOwnerKind::Settings,
        InputOwnerKind::Dialog,
        InputOwnerKind::Palette,
    ] {
        let actual = workspace_command_for_keystroke(
            owner,
            &Keystroke::parse("cmd-s").unwrap(),
            |_| Some(CommandId::FileSave),
            |_| false,
        );
        assert_eq!(actual, None, "{owner:?} must block project-file save");
    }
}

#[test]
fn notification_settings_command_is_available_without_project() {
    let availability = CommandId::SettingsNotifications.availability(false);

    assert!(availability.enabled);
    assert!(availability.disabled_reason.is_none());
}

#[test]
fn create_project_command_is_registered_and_available_without_project() {
    let registry = default_registry();
    let command = CommandId::ProjectCreate;

    assert!(registry.contains(command));
    assert_eq!(command.as_str(), "project.create");
    assert!(command.availability(false).enabled);
    assert_eq!(command.presentation().title, "Create Project");
}

#[test]
fn settings_open_command_is_available_without_project() {
    let availability = CommandId::SettingsOpen.availability(false);

    assert!(availability.enabled);
    assert!(availability.disabled_reason.is_none());
    assert_eq!(CommandId::SettingsOpen.as_str(), "settings.open");
    assert_eq!(
        CommandId::SettingsOpen.presentation().title,
        "Open Settings"
    );
}

#[test]
fn layout_default_commands_are_registered_and_available_without_project() {
    let registry = default_registry();

    for (command, id) in [
        (CommandId::LayoutDefaultEdit, "layout.default.edit"),
        (CommandId::LayoutDefaultReset, "layout.default.reset"),
        (CommandId::LayoutDefaultReload, "layout.default.reload"),
    ] {
        assert!(registry.contains(command));
        assert_eq!(command.as_str(), id);
        assert!(command.availability(false).enabled);
    }
}

#[test]
fn layout_project_commands_require_selected_project() {
    for (command, id) in [
        (CommandId::LayoutProjectEdit, "layout.project.edit"),
        (
            CommandId::LayoutResetLocalOverride,
            "layout.reset_local_override",
        ),
    ] {
        assert_eq!(command.as_str(), id);
        assert!(!command.availability(false).enabled);
        assert_eq!(
            command.availability(false).disabled_reason,
            Some("Open a project first")
        );
        assert!(command.availability(true).enabled);
    }
}

#[test]
fn parses_keybinding_toml() {
    let source = r#"
        [[bindings]]
        keys = "cmd-p"
        command = "command_palette.open"
    "#;

    let config: KeybindingsConfig = toml::from_str(source).unwrap();

    assert_eq!(config.bindings.len(), 1);
    assert_eq!(config.bindings[0].keys, "cmd-p");
    assert_eq!(config.bindings[0].command, "command_palette.open");
}

#[test]
fn detects_duplicate_keybindings() {
    let source = r#"
        [[bindings]]
        keys = "cmd-p"
        command = "command_palette.open"

        [[bindings]]
        keys = "CMD-P"
        command = "project.palette"
    "#;
    let config: KeybindingsConfig = toml::from_str(source).unwrap();

    let conflicts = config.conflicts();

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].keys, "cmd-p");
}

#[test]
fn default_keybindings_include_palette_shortcuts() {
    let config = default_keybindings();

    assert_has_config_binding(&config, "cmd-p", "file.find");
    assert_has_config_binding(&config, "cmd-shift-p", "command_palette.open");
    assert_has_config_binding(&config, "ctrl-k", "pane.palette");
    assert_has_config_binding(&config, "cmd-alt-p", "project.opened_palette");
    assert_has_config_binding(&config, "ctrl-alt-p", "project.opened_palette");
    assert_has_ui_binding("cmd-alt-p", "project.opened_palette");
    assert_has_ui_binding("ctrl-alt-p", "project.opened_palette");
}

#[test]
fn default_keybindings_include_settings_shortcuts() {
    let config = default_keybindings();

    assert_has_config_binding(&config, "cmd-,", "settings.open");
    assert_has_config_binding(&config, "ctrl-,", "settings.open");
    assert_has_ui_binding("cmd-,", "settings.open");
    assert_has_ui_binding("ctrl-,", "settings.open");
}

#[test]
fn keybinding_display_filters_shortcuts_by_platform() {
    let keys = vec!["cmd-p".to_string(), "ctrl-p".to_string()];

    assert_eq!(
        display_keybindings_for_platform(&keys, KeybindingDisplayPlatform::Mac),
        vec!["cmd-p".to_string()]
    );
    assert_eq!(
        display_keybindings_for_platform(&keys, KeybindingDisplayPlatform::Other),
        vec!["ctrl-p".to_string()]
    );
}

#[test]
fn keybinding_display_keeps_neutral_or_fallback_shortcuts() {
    let neutral = vec!["enter".to_string(), "escape".to_string()];
    assert_eq!(
        display_keybindings_for_platform(&neutral, KeybindingDisplayPlatform::Mac),
        neutral
    );

    let fallback = vec!["cmd-p".to_string()];
    assert_eq!(
        display_keybindings_for_platform(&fallback, KeybindingDisplayPlatform::Other),
        fallback
    );
}

#[test]
fn default_keybindings_include_pane_navigation_shortcuts() {
    let config = default_keybindings();

    for (keys, command) in [
        ("cmd-alt-left", "pane.focus_left"),
        ("cmd-alt-right", "pane.focus_right"),
        ("cmd-alt-up", "pane.focus_up"),
        ("cmd-alt-down", "pane.focus_down"),
        ("ctrl-alt-left", "pane.focus_left"),
        ("ctrl-alt-right", "pane.focus_right"),
        ("ctrl-alt-up", "pane.focus_up"),
        ("ctrl-alt-down", "pane.focus_down"),
        ("cmd-alt-shift-left", "pane.resize_left"),
        ("cmd-alt-shift-right", "pane.resize_right"),
        ("cmd-alt-shift-up", "pane.resize_up"),
        ("cmd-alt-shift-down", "pane.resize_down"),
        ("ctrl-alt-shift-left", "pane.resize_left"),
        ("ctrl-alt-shift-right", "pane.resize_right"),
        ("ctrl-alt-shift-up", "pane.resize_up"),
        ("ctrl-alt-shift-down", "pane.resize_down"),
    ] {
        assert_has_config_binding(&config, keys, command);
        assert_has_ui_binding(keys, command);
    }
}

#[test]
fn default_keybindings_include_tab_new_shortcuts() {
    let config = default_keybindings();

    assert_has_config_binding(&config, "cmd-t", "tab.new");
    assert_has_config_binding(&config, "ctrl-t", "tab.new");
    assert_has_ui_binding("cmd-t", "tab.new");
    assert_has_ui_binding("ctrl-t", "tab.new");
}

#[test]
fn default_keybindings_reserve_ctrl_w_for_vim_sequences() {
    let config = default_keybindings();

    assert_has_config_binding(&config, "cmd-w", "pane.close");
    assert_has_ui_binding("cmd-w", "pane.close");
    assert!(
        !config
            .bindings
            .iter()
            .any(|binding| { binding.keys == "ctrl-w" && binding.command == "pane.close" })
    );
    assert!(
        !default_ui_keybinding_specs()
            .iter()
            .any(|binding| { binding.keys == "ctrl-w" && binding.command == CommandId::PaneClose })
    );
}

#[test]
fn layered_keybindings_apply_sparse_overrides_sequences_and_unbinds() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "cmd-j"
        command = "tab.palette"
        context = "Workspace"
        unbind = true

        [[bindings]]
        keys = "cmd-l"
        command = "tab.palette"
        context = "Workspace"

        [[bindings]]
        keys = "g x"
        command = "tab.next"
        context = "WorkspaceVim && !Input"
    "#,
    )
    .unwrap();
    let registry = bindable_registry();
    let compiled = compile_layered_keybinding_specs(&config, &registry);
    let specs = layered_ui_keybinding_specs(&config, &registry);

    assert!(compiled.iter().any(|binding| {
        binding.keys == "cmd-j"
            && binding.context.as_deref() == Some("Workspace")
            && binding.action
                == CompiledKeybindingAction::Unbind(Some(BindableActionId::Command(
                    CommandId::TabPalette,
                )))
    }));
    assert!(specs.iter().any(|spec| {
        spec.keys == "cmd-l"
            && spec.command == CommandId::TabPalette
            && spec.context.as_deref() == Some("Workspace")
    }));
    assert!(!specs.iter().any(|spec| {
        spec.keys == "cmd-j"
            && spec.command == CommandId::TabPalette
            && spec.context.as_deref() == Some("Workspace")
    }));
    assert!(specs.iter().any(|spec| {
        spec.keys == "g x"
            && spec.command == CommandId::TabNext
            && spec.context.as_deref() == Some("WorkspaceVim && !Input")
    }));
}

#[test]
fn configurable_leader_expands_default_and_user_sequences() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        leader = "ctrl-space"

        [[bindings]]
        keys = "<leader> x"
        command = "tab.next"
        context = "YtttVim && yttt_vim_scope == global && yttt_vim_control == true"
        "#,
    )
    .unwrap();

    let specs = layered_ui_keybinding_specs(&config, &bindable_registry());

    assert!(specs.iter().any(|spec| {
        spec.keys == "ctrl-space f f"
            && spec.command == CommandId::FileFind
            && spec.context.as_deref() == Some(VIM_CONTROL_CONTEXT)
    }));
    assert!(specs.iter().any(|spec| {
        spec.keys == "ctrl-space x"
            && spec.command == CommandId::TabNext
            && spec.context.as_deref() == Some(VIM_CONTROL_CONTEXT)
    }));
    assert!(!specs.iter().any(|spec| spec.keys.contains("<leader>")));
}

#[test]
fn conflicts_compare_resolved_leader_sequences() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        leader = "ctrl-space"

        [[bindings]]
        keys = "<leader> p"
        command = "tab.palette"
        context = "Workspace"

        [[bindings]]
        keys = "ctrl-space p"
        command = "pane.palette"
        context = "Workspace"
        "#,
    )
    .unwrap();

    let conflicts = config.conflicts();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].keys, "ctrl-space p");
}

#[test]
fn recursive_leader_is_rejected() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        leader = "<leader>"
        "#,
    )
    .unwrap();

    assert_eq!(
        config.invalid_bindings(),
        vec!["invalid leader key \"<leader>\"".to_string()]
    );
}

#[test]
fn layered_keybindings_use_last_binding_in_the_same_context() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "cmd-p"
        command = "tab.palette"
        context = "Workspace"
    "#,
    )
    .unwrap();

    let specs = layered_ui_keybinding_specs(&config, &bindable_registry());

    assert!(specs.iter().any(|spec| {
        spec.keys == "cmd-p"
            && spec.command == CommandId::TabPalette
            && spec.context.as_deref() == Some("Workspace")
    }));
    assert!(!specs.iter().any(|spec| {
        spec.keys == "cmd-p"
            && spec.command == CommandId::FileFind
            && spec.context.as_deref() == Some("Workspace")
    }));
}

#[test]
fn action_specific_unbind_preserves_another_action_in_the_same_slot() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        schema_version = 5

        [[bindings]]
        keys = "cmd-p"
        command = "tab.palette"
        context = "Workspace"

        [[bindings]]
        keys = "cmd-p"
        command = "file.find"
        context = "Workspace"
        unbind = true
        "#,
    )
    .unwrap();

    let compiled = compile_layered_keybinding_specs(&config, &bindable_registry());

    assert!(compiled.iter().any(|binding| {
        binding.keys == "cmd-p"
            && binding.context.as_deref() == Some("Workspace")
            && binding.action
                == CompiledKeybindingAction::Bind(BindableActionId::Command(CommandId::TabPalette))
    }));
    assert!(compiled.iter().any(|binding| {
        binding.keys == "cmd-p"
            && binding.context.as_deref() == Some("Workspace")
            && binding.action
                == CompiledKeybindingAction::Unbind(Some(BindableActionId::Command(
                    CommandId::FileFind,
                )))
    }));
}

#[test]
fn contexts_allow_the_same_keys_to_bind_different_actions() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "j"
        command = "settings.vim.next_group"
        context = "YtttSettingsVim && !Input"

        [[bindings]]
        keys = "j"
        command = "terminal.vi.motion.down"
        context = "YtttTerminalVi"
    "#,
    )
    .unwrap();

    assert!(config.conflicts().is_empty());
    let specs = layered_ui_keybinding_specs(&config, &bindable_registry());
    assert!(specs.iter().any(|spec| {
        spec.keys == "j"
            && spec.command.as_str() == "settings.vim.next_group"
            && spec.context.as_deref() == Some("YtttSettingsVim && !Input")
    }));
    assert!(specs.iter().any(|spec| {
        spec.keys == "j"
            && spec.command.as_str() == "terminal.vi.motion.down"
            && spec.context.as_deref() == Some("YtttTerminalVi")
    }));
}

#[test]
fn bindable_catalog_covers_commands_and_modal_ui_actions() {
    let actions = BindableActionId::all().collect::<Vec<_>>();
    for &command in CommandId::ALL {
        assert!(
            actions.contains(&BindableActionId::Command(command)),
            "missing bindable action for {}",
            command.as_str()
        );
    }
    for id in [
        "palette.select_next",
        "project_tree.new_file",
        "tab.close_all",
        "terminal.vi.motion.down",
        "vim.mode.normal",
        "vim.mode.terminal",
        "editor.vim.motion.down",
        "settings.vim.next_group",
        "projects.focus",
        "projects.vim.previous",
        "projects.vim.next",
        "projects.vim.first",
        "projects.vim.last",
        "project_tree.vim.down",
        "project_tree.vim.left",
        "project_tree.vim.right",
        "project_tree.vim.open",
        "project_tree.collapse_all",
        "project_tree.toggle_hidden",
    ] {
        assert!(
            BindableActionId::from_str_id(id).is_some(),
            "missing bindable action {id}"
        );
    }
    assert!(default_ui_keybinding_specs().iter().any(|spec| {
        spec.keys == "g t"
            && spec.command == CommandId::TabNext
            && spec.context.as_deref() == Some(VIM_NORMAL_CONTEXT)
    }));
    for (keys, command) in [
        ("ctrl-w h", CommandId::PaneFocusLeft),
        ("ctrl-w j", CommandId::PaneFocusDown),
        ("ctrl-w k", CommandId::PaneFocusUp),
        ("ctrl-w l", CommandId::PaneFocusRight),
        ("ctrl-w ctrl-h", CommandId::PaneFocusLeft),
        ("ctrl-w ctrl-j", CommandId::PaneFocusDown),
        ("ctrl-w ctrl-k", CommandId::PaneFocusUp),
        ("ctrl-w ctrl-l", CommandId::PaneFocusRight),
    ] {
        assert!(default_ui_keybinding_specs().iter().any(|spec| {
            spec.keys == keys
                && spec.command == command
                && spec.context.as_deref() == Some(VIM_NORMAL_CONTEXT)
        }));
    }
    for (keys, action) in [
        ("j", "projects.vim.next"),
        ("k", "projects.vim.previous"),
        ("g g", "projects.vim.first"),
        ("shift-g", "projects.vim.last"),
    ] {
        assert!(default_ui_keybinding_specs().iter().any(|spec| {
            spec.keys == keys
                && spec.command.as_str() == action
                && spec.context.as_deref() == Some(VIM_PROJECTS_NORMAL_CONTEXT)
        }));
    }
    for (keys, action) in [
        ("[ p", "project_panel.page.previous"),
        ("] p", "project_panel.page.next"),
        ("g p f", "project_panel.page.files"),
    ] {
        assert!(default_ui_keybinding_specs().iter().any(|spec| {
            spec.keys == keys
                && spec.command.as_str() == action
                && spec.context.as_deref() == Some(VIM_PROJECT_PANEL_NORMAL_CONTEXT)
        }));
    }
    for (keys, action) in [
        ("j", "project_tree.vim.down"),
        ("k", "project_tree.vim.up"),
        ("h", "project_tree.vim.left"),
        ("l", "project_tree.vim.right"),
        ("enter", "project_tree.vim.open"),
        ("o", "project_tree.vim.open"),
        ("g g", "project_tree.vim.first"),
        ("shift-g", "project_tree.vim.last"),
        ("z", "project_tree.collapse_all"),
        ("a", "project_tree.new_file"),
        ("shift-a", "project_tree.new_directory"),
        ("r", "project_tree.rename"),
        ("d", "project_tree.delete"),
        ("y", "project_tree.copy"),
        ("x", "project_tree.cut"),
        ("p", "project_tree.paste"),
        ("shift-h", "project_tree.toggle_hidden"),
        ("shift-r", "project_panel.refresh"),
        ("q", "project_panel.toggle"),
        ("/", "file.find"),
    ] {
        assert!(default_ui_keybinding_specs().iter().any(|spec| {
            spec.keys == keys
                && spec.command.as_str() == action
                && spec.context.as_deref() == Some(VIM_PROJECT_TREE_NORMAL_CONTEXT)
        }));
    }
    assert!(default_ui_keybinding_specs().iter().any(|spec| {
        spec.keys == "j"
            && spec.command == BindableActionId::PaletteNext
            && spec.context.as_deref() == Some(VIM_PALETTE_NORMAL_CONTEXT)
    }));
}

#[test]
fn user_keybindings_specs_skip_conflicting_keys_and_invalid_commands() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "cmd-l"
        command = "tab.palette"

        [[bindings]]
        keys = "CMD-L"
        command = "pane.palette"

        [[bindings]]
        keys = "cmd-x"
        command = "missing.command"
    "#,
    )
    .unwrap();

    let specs = yttt::ui::interaction::actions::ui_keybinding_specs_from_config(
        &config,
        &default_registry(),
    );

    assert!(specs.is_empty());
}

#[test]
fn user_keybindings_specs_map_non_default_command_actions() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "cmd-alt-k"
        command = "settings.keybindings"
    "#,
    )
    .unwrap();

    let specs = yttt::ui::interaction::actions::ui_keybinding_specs_from_config(
        &config,
        &default_registry(),
    );

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].keys, "cmd-alt-k");
    assert_eq!(specs[0].command, CommandId::SettingsKeybindings);
}

#[test]
fn runtime_keybinding_matcher_uses_current_config_specs_only() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "cmd-l"
        command = "tab.palette"
    "#,
    )
    .unwrap();
    let specs = yttt::ui::interaction::actions::ui_keybinding_specs_from_config(
        &config,
        &default_registry(),
    );

    assert_eq!(
        runtime_command_for_keystroke(&specs, &Keystroke::parse("cmd-l").unwrap()),
        Some(CommandId::TabPalette)
    );
    assert_eq!(
        runtime_command_for_keystroke(&specs, &Keystroke::parse("cmd-j").unwrap()),
        None
    );
}

#[test]
fn app_startup_keybindings_compile_the_complete_default_catalog() {
    let startup = app_startup_keybindings();
    assert_eq!(startup.len(), default_ui_keybinding_specs().len());
    assert!(startup.len() > default_keybindings().bindings.len());
}

#[test]
fn load_app_keybindings_missing_file_compiles_complete_defaults() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let bindings =
        yttt::ui::interaction::actions::load_app_keybindings(&paths, &bindable_registry());

    assert!(paths.keybindings_file().exists());
    assert_eq!(bindings.len(), app_startup_keybindings().len());
}

#[test]
fn load_app_keybindings_with_warnings_uses_complete_defaults() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.keybindings_file(),
        r#"
        schema_version = 5

        [[bindings]]
        keys = "cmd-p"
        command = "tab.palette"
        context = "Workspace"

        [[bindings]]
        keys = "cmd-p"
        command = "project.palette"
        context = "Workspace"
        "#,
    )
    .unwrap();

    let bindings =
        yttt::ui::interaction::actions::load_app_keybindings(&paths, &bindable_registry());

    assert_eq!(bindings.len(), app_startup_keybindings().len());
}

#[test]
fn missing_keybindings_file_writes_sparse_overrides() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let loaded = load_keybindings(&paths, &bindable_registry()).unwrap();

    assert_eq!(loaded.config, KeybindingsConfig::default());
    assert!(loaded.warnings.is_empty());
    assert!(paths.keybindings_file().exists());
    let persisted: KeybindingsConfig =
        toml::from_str(&std::fs::read_to_string(paths.keybindings_file()).unwrap()).unwrap();
    assert_eq!(persisted, KeybindingsConfig::default());
}

#[test]
fn legacy_default_keybindings_migrate_to_sparse_overrides() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    let mut legacy = legacy_v2_default_keybindings();
    legacy.schema_version = 0;
    legacy.bindings.retain(|binding| {
        !matches!(
            binding.command.as_str(),
            "file.save" | "project_panel.toggle" | "project.opened_palette"
        )
    });
    std::fs::write(
        paths.keybindings_file(),
        toml::to_string_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config, KeybindingsConfig::default());
    let persisted: KeybindingsConfig =
        toml::from_str(&std::fs::read_to_string(paths.keybindings_file()).unwrap()).unwrap();
    assert_eq!(persisted, loaded.config);
}

#[test]
fn schema_one_defaults_migrate_to_sparse_overrides() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut legacy = legacy_v2_default_keybindings();
    legacy.schema_version = 1;
    legacy
        .bindings
        .retain(|binding| binding.command != "project.opened_palette");
    save_keybindings(&paths, &legacy).unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config, KeybindingsConfig::default());
    let persisted: KeybindingsConfig =
        toml::from_str(&std::fs::read_to_string(paths.keybindings_file()).unwrap()).unwrap();
    assert_eq!(persisted, loaded.config);
}

#[test]
fn schema_two_defaults_migrate_to_sparse_overrides() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let legacy = legacy_v2_default_keybindings();
    save_keybindings(&paths, &legacy).unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config, KeybindingsConfig::default());
}

#[test]
fn schema_three_defaults_migrate_to_sparse_overrides() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let legacy = legacy_v3_default_keybindings();
    save_keybindings(&paths, &legacy).unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config, KeybindingsConfig::default());
}

#[test]
fn schema_four_sparse_config_remains_sparse() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    save_keybindings(
        &paths,
        &KeybindingsConfig {
            schema_version: 4,
            leader: "comma".to_string(),
            bindings: Vec::new(),
        },
    )
    .unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config.schema_version, KEYBINDINGS_SCHEMA_VERSION);
    assert_eq!(loaded.config.leader, "comma");
    assert!(loaded.config.bindings.is_empty());
    let persisted: KeybindingsConfig =
        toml::from_str(&std::fs::read_to_string(paths.keybindings_file()).unwrap()).unwrap();
    assert_eq!(persisted, loaded.config);
}

#[test]
fn schema_four_ctrl_w_close_is_removed_for_vim_sequences() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let stale = KeybindingsConfig {
        schema_version: 4,
        leader: "space".to_string(),
        bindings: vec![
            Keybinding {
                keys: "ctrl-w".to_string(),
                command: "pane.close".to_string(),
                context: Some("Workspace".to_string()),
                unbind: false,
            },
            Keybinding {
                keys: "cmd-l".to_string(),
                command: "tab.palette".to_string(),
                context: Some("Workspace".to_string()),
                unbind: false,
            },
            Keybinding {
                keys: "cmd-g".to_string(),
                command: "tab.next".to_string(),
                context: Some("WorkspaceVim".to_string()),
                unbind: false,
            },
            Keybinding {
                keys: "cmd-h".to_string(),
                command: "tab.prev".to_string(),
                context: Some("Workspace && !WorkspaceVim".to_string()),
                unbind: false,
            },
        ],
    };
    save_keybindings(&paths, &stale).unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config.schema_version, KEYBINDINGS_SCHEMA_VERSION);
    assert!(
        !loaded
            .config
            .bindings
            .iter()
            .any(|binding| { binding.keys == "ctrl-w" && binding.command == "pane.close" })
    );
    assert!(loaded.config.bindings.iter().any(|binding| {
        binding.keys == "cmd-l"
            && binding.command == "tab.palette"
            && binding.context.as_deref() == Some("Workspace")
            && !binding.unbind
    }));
    assert!(loaded.config.bindings.iter().any(|binding| {
        binding.keys == "cmd-g"
            && binding.command == "tab.next"
            && binding.context.as_deref() == Some("YtttVim && yttt_vim_scope == global")
            && !binding.unbind
    }));
    assert!(loaded.config.bindings.iter().any(|binding| {
        binding.keys == "cmd-h"
            && binding.command == "tab.prev"
            && binding.context.as_deref()
                == Some("Workspace && !(YtttVim && yttt_vim_scope == global)")
            && !binding.unbind
    }));
    let specs = layered_ui_keybinding_specs(&loaded.config, &default_registry());
    assert!(
        !specs
            .iter()
            .any(|binding| { binding.keys == "ctrl-w" && binding.command == CommandId::PaneClose })
    );
    let persisted: KeybindingsConfig =
        toml::from_str(&std::fs::read_to_string(paths.keybindings_file()).unwrap()).unwrap();
    assert_eq!(persisted, loaded.config);
}

#[test]
fn custom_legacy_keybindings_are_versioned_without_restoring_defaults() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let legacy = KeybindingsConfig {
        schema_version: 0,
        leader: "space".to_string(),
        bindings: vec![Keybinding {
            keys: "cmd-l".to_string(),
            command: "tab.palette".to_string(),
            context: None,
            unbind: false,
        }],
    };
    save_keybindings(&paths, &legacy).unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config.schema_version, KEYBINDINGS_SCHEMA_VERSION);
    let specs = layered_ui_keybinding_specs(&loaded.config, &default_registry());
    assert_eq!(
        specs
            .iter()
            .filter(|binding| binding.context.as_deref() == Some("Workspace"))
            .count(),
        1
    );
    assert!(specs.iter().any(|binding| {
        binding.keys == "cmd-l"
            && binding.command == CommandId::TabPalette
            && binding.context.as_deref() == Some("Workspace")
    }));
    assert!(loaded.config.bindings.iter().any(|binding| {
        binding.unbind && binding.keys == "cmd-o" && binding.command == "project.open"
    }));
    assert_eq!(
        load_keybindings(&paths, &default_registry())
            .unwrap()
            .config,
        loaded.config
    );
}

#[test]
fn custom_schema_one_keybindings_do_not_gain_default_shortcuts() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let custom = KeybindingsConfig {
        schema_version: 1,
        leader: "space".to_string(),
        bindings: vec![Keybinding {
            keys: "cmd-l".to_string(),
            command: "tab.palette".to_string(),
            context: None,
            unbind: false,
        }],
    };
    save_keybindings(&paths, &custom).unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config.schema_version, KEYBINDINGS_SCHEMA_VERSION);
    let specs = layered_ui_keybinding_specs(&loaded.config, &default_registry());
    assert_eq!(
        specs
            .iter()
            .filter(|binding| binding.context.as_deref() == Some("Workspace"))
            .count(),
        1
    );
    assert!(specs.iter().any(|binding| {
        binding.keys == "cmd-l"
            && binding.command == CommandId::TabPalette
            && binding.context.as_deref() == Some("Workspace")
    }));
    assert!(loaded.config.bindings.iter().any(|binding| {
        binding.unbind && binding.keys == "cmd-o" && binding.command == "project.open"
    }));
}

#[test]
fn save_keybindings_writes_user_toml() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let config = KeybindingsConfig {
        schema_version: KEYBINDINGS_SCHEMA_VERSION,
        leader: "space".to_string(),
        bindings: vec![Keybinding {
            keys: "cmd-l".to_string(),
            command: "tab.palette".to_string(),
            context: None,
            unbind: false,
        }],
    };

    save_keybindings(&paths, &config).unwrap();

    let saved = std::fs::read_to_string(paths.keybindings_file()).unwrap();
    assert!(saved.contains("cmd-l"));
    assert!(saved.contains("tab.palette"));
}

#[test]
fn keybindings_editor_lists_commands_with_current_keys() {
    let editor = KeybindingsEditorState::new(default_keybindings(), default_registry());

    let row = editor
        .rows()
        .into_iter()
        .find(|row| row.command == CommandId::CommandPaletteOpen)
        .unwrap();

    assert_eq!(row.title, "Open Command Palette");
    assert!(row.keys.contains(&"cmd-shift-p".to_string()));
    assert!(row.keys.contains(&"ctrl-shift-p".to_string()));
    if cfg!(target_os = "macos") {
        assert_eq!(row.display_keys(), vec!["cmd-shift-p".to_string()]);
    } else {
        assert_eq!(row.display_keys(), vec!["ctrl-shift-p".to_string()]);
    }
}

#[test]
fn keybindings_editor_lists_non_command_ui_actions() {
    let editor = KeybindingsEditorState::new(KeybindingsConfig::default(), bindable_registry());

    let rows = editor.rows();
    let terminal_motion = rows
        .iter()
        .find(|row| row.command_id == "terminal.vi.motion.down")
        .expect("terminal Vi motions must be user-rebindable");
    assert!(terminal_motion.keys.contains(&"j".to_string()));
    assert!(
        rows.iter()
            .any(|row| row.command_id == "settings.vim.next_group")
    );
    assert!(
        rows.iter()
            .any(|row| row.command_id == "project_tree.new_file")
    );
}

#[test]
fn keybindings_editor_localizes_command_presentations() {
    let editor = KeybindingsEditorState::new(default_keybindings(), default_registry());
    let text = UiText::new(Locale::Chinese);

    let row = editor
        .rows_with_text(&text)
        .into_iter()
        .find(|row| row.command == CommandId::CommandPaletteOpen)
        .unwrap();

    assert_eq!(row.title, "打开命令面板");
    assert_eq!(row.description, "搜索并运行命令");
}

#[test]
fn recorded_keybindings_accept_shortcuts_and_ignore_incomplete_input() {
    assert_eq!(
        recorded_keybinding(&Keystroke::parse("cmd-shift-k").unwrap()).as_deref(),
        Some("cmd-shift-k")
    );
    assert_eq!(
        recorded_keybinding(&Keystroke::parse("enter").unwrap()).as_deref(),
        Some("enter")
    );
    assert_eq!(
        recorded_keybinding(&Keystroke {
            key: "k".to_string(),
            key_char: Some("k".to_string()),
            ..Default::default()
        })
        .as_deref(),
        Some("k")
    );
    assert!(recorded_keybinding(&Keystroke::parse("shift").unwrap()).is_none());
    assert!(recorded_keybinding(&Keystroke::parse("k").unwrap()).is_none());
}

#[test]
fn keybindings_editor_edits_deletes_and_resets_command_keys() {
    let mut editor = KeybindingsEditorState::new(default_keybindings(), default_registry());

    editor.set_command_keys(CommandId::TabPalette, vec!["cmd-l".to_string()]);
    assert_eq!(
        editor.command_keys(CommandId::TabPalette),
        vec!["cmd-l".to_string()]
    );

    editor.delete_command_keys(CommandId::TabPalette);
    assert!(editor.command_keys(CommandId::TabPalette).is_empty());

    editor.reset_command_keys(CommandId::TabPalette);
    assert!(
        editor
            .command_keys(CommandId::TabPalette)
            .contains(&"cmd-j".to_string())
    );
}

#[test]
fn saving_unchanged_action_keys_preserves_context_assignments() {
    let config = KeybindingsConfig::default();
    let mut editor = KeybindingsEditorState::new(config.clone(), bindable_registry());
    let action = BindableActionId::Command(CommandId::PaneFocusLeft);
    let keys = editor.action_keys(action);

    assert!(keys.contains(&"ctrl-w h".to_string()));
    editor.set_action_keys(action, keys);

    assert_eq!(editor.config(), &config);
}

#[test]
fn keybindings_editor_profiles_keep_the_complete_catalog_and_inherit_base_bindings() {
    let editor = KeybindingsEditorState::new(KeybindingsConfig::default(), bindable_registry());
    let text = UiText::new(Locale::English);
    let base_rows = editor.rows_for_profile(KeybindingProfile::Base, &text);
    let vim_rows = editor.rows_for_profile(KeybindingProfile::Vim, &text);

    assert_eq!(base_rows.len(), BindableActionId::all().count());
    assert_eq!(
        base_rows.iter().map(|row| row.command).collect::<Vec<_>>(),
        vim_rows.iter().map(|row| row.command).collect::<Vec<_>>()
    );

    let action = BindableActionId::Command(CommandId::CommandPaletteOpen);
    let base = base_rows.iter().find(|row| row.command == action).unwrap();
    let vim = vim_rows.iter().find(|row| row.command == action).unwrap();
    assert!(
        base.assignments
            .iter()
            .all(|assignment| assignment.origin != KeybindingOrigin::Inherited)
    );
    assert!(vim.assignments.iter().any(|assignment| {
        assignment.origin == KeybindingOrigin::Inherited
            && assignment.keys == "cmd-shift-p"
            && !assignment.shadowed
    }));
}

#[test]
fn editing_the_vim_profile_creates_a_difference_without_mutating_base_bindings() {
    let mut editor = KeybindingsEditorState::new(KeybindingsConfig::default(), bindable_registry());
    let action = BindableActionId::Command(CommandId::TabPalette);
    let base_before = editor.action_keys_for_profile(action, KeybindingProfile::Base);
    let vim_before = editor.action_keys_for_profile(action, KeybindingProfile::Vim);

    editor.set_action_keys_for_profile(
        action,
        KeybindingProfile::Vim,
        vec!["ctrl-alt-shift-y".to_string()],
    );

    assert_eq!(
        editor.action_keys_for_profile(action, KeybindingProfile::Base),
        base_before
    );
    assert_eq!(
        editor.action_keys_for_profile(action, KeybindingProfile::Vim),
        vec!["ctrl-alt-shift-y".to_string()]
    );
    assert!(editor.config().bindings.iter().any(|binding| {
        binding.command == action.as_str()
            && binding.keys == "ctrl-alt-shift-y"
            && binding.context.as_deref() == Some(yttt::ui::vim::VIM_PROFILE_CONTEXT)
            && !binding.unbind
    }));

    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    editor.save(&paths).unwrap();
    let loaded = load_keybindings(&paths, &bindable_registry()).unwrap();
    let mut editor = KeybindingsEditorState::new(loaded.config, bindable_registry());
    assert_eq!(
        editor.action_keys_for_profile(action, KeybindingProfile::Vim),
        vec!["ctrl-alt-shift-y".to_string()]
    );
    assert_eq!(
        editor.action_keys_for_profile(action, KeybindingProfile::Base),
        base_before
    );

    editor.reset_action_keys_for_profile(action, KeybindingProfile::Vim);
    assert_eq!(
        editor.action_keys_for_profile(action, KeybindingProfile::Vim),
        vim_before
    );
    assert_eq!(
        editor.action_keys_for_profile(action, KeybindingProfile::Base),
        base_before
    );
}

#[test]
fn vim_profile_reports_exact_shadow_and_prefix_diagnostics() {
    let context = yttt::ui::vim::VIM_NORMAL_CONTEXT.to_string();
    let config = KeybindingsConfig {
        schema_version: KEYBINDINGS_SCHEMA_VERSION,
        leader: "space".to_string(),
        bindings: vec![
            Keybinding {
                keys: "x".to_string(),
                command: CommandId::ProjectOpen.as_str().to_string(),
                context: Some("Workspace".to_string()),
                unbind: false,
            },
            Keybinding {
                keys: "x".to_string(),
                command: CommandId::FileFind.as_str().to_string(),
                context: Some(context.clone()),
                unbind: false,
            },
            Keybinding {
                keys: "g".to_string(),
                command: CommandId::ProjectOpen.as_str().to_string(),
                context: Some(context.clone()),
                unbind: false,
            },
            Keybinding {
                keys: "g".to_string(),
                command: CommandId::FileFind.as_str().to_string(),
                context: Some(context.clone()),
                unbind: false,
            },
            Keybinding {
                keys: "g g".to_string(),
                command: CommandId::TabPalette.as_str().to_string(),
                context: Some(context),
                unbind: false,
            },
        ],
    };
    let editor = KeybindingsEditorState::new(config, bindable_registry());
    let rows = editor.rows_for_profile(KeybindingProfile::Vim, &UiText::new(Locale::English));
    let project_open = rows
        .iter()
        .find(|row| row.command == BindableActionId::Command(CommandId::ProjectOpen))
        .unwrap();
    let file_find = rows
        .iter()
        .find(|row| row.command == BindableActionId::Command(CommandId::FileFind))
        .unwrap();
    let tab_palette = rows
        .iter()
        .find(|row| row.command == BindableActionId::Command(CommandId::TabPalette))
        .unwrap();

    assert!(project_open.assignments.iter().any(|assignment| {
        assignment.keys == "x"
            && assignment.origin == KeybindingOrigin::Inherited
            && assignment.shadowed
    }));
    assert!(
        project_open
            .diagnostics
            .contains(&KeybindingDiagnosticKind::Conflict)
    );
    assert!(
        file_find
            .diagnostics
            .contains(&KeybindingDiagnosticKind::Conflict)
    );
    assert!(
        project_open
            .diagnostics
            .contains(&KeybindingDiagnosticKind::Prefix)
    );
    assert!(
        tab_palette
            .diagnostics
            .contains(&KeybindingDiagnosticKind::Prefix)
    );
}

#[test]
fn deleting_default_leader_binding_uses_configured_leader() {
    let config = KeybindingsConfig {
        schema_version: KEYBINDINGS_SCHEMA_VERSION,
        leader: "ctrl-space".to_string(),
        bindings: Vec::new(),
    };
    let mut editor = KeybindingsEditorState::new(config, bindable_registry());

    editor.delete_command_keys(CommandId::FileFind);

    let specs = layered_ui_keybinding_specs(editor.config(), &bindable_registry());
    assert!(!specs.iter().any(|spec| spec.command == CommandId::FileFind));
    assert!(editor.config().bindings.iter().any(|binding| {
        binding.unbind
            && binding.command == CommandId::FileFind.as_str()
            && binding.keys == "ctrl-space f f"
    }));
}

#[test]
fn keybindings_editor_blocks_default_conflict_from_sparse_config() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut editor = KeybindingsEditorState::new(KeybindingsConfig::default(), default_registry());
    editor.set_command_keys(CommandId::TabPalette, vec!["cmd-p".to_string()]);

    let error = editor.save(&paths).unwrap_err();

    assert_eq!(
        error,
        KeybindingEditError::ConflictingBindings(vec!["cmd-p".to_string()])
    );
    assert!(!paths.keybindings_file().exists());
}

#[test]
fn keybindings_editor_can_reassign_a_released_default_key() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut editor = KeybindingsEditorState::new(KeybindingsConfig::default(), default_registry());
    editor.set_command_keys(
        CommandId::FileFind,
        vec!["ctrl-p".to_string(), "cmd-alt-shift-f".to_string()],
    );
    editor.set_command_keys(CommandId::TabPalette, vec!["cmd-p".to_string()]);

    editor.save(&paths).unwrap();

    assert!(paths.keybindings_file().exists());
}

#[test]
fn keybindings_editor_replaces_shared_action_in_every_default_context() {
    let mut editor = KeybindingsEditorState::new(KeybindingsConfig::default(), bindable_registry());

    editor.set_action_keys(
        BindableActionId::TerminalSearchDismiss,
        vec!["q".to_string()],
    );

    assert_eq!(
        editor.action_keys(BindableActionId::TerminalSearchDismiss),
        vec!["q".to_string()]
    );
    let mut contexts = editor
        .config()
        .bindings
        .iter()
        .filter(|binding| {
            !binding.unbind && binding.keys == "q" && binding.command == "terminal.search.cancel"
        })
        .filter_map(|binding| binding.context.as_deref())
        .collect::<Vec<_>>();
    contexts.sort_unstable();
    assert_eq!(
        contexts,
        vec![TERMINAL_HINT_KEY_CONTEXT, TERMINAL_SEARCH_KEY_CONTEXT]
    );
    assert_eq!(
        editor
            .config()
            .bindings
            .iter()
            .filter(|binding| {
                binding.unbind
                    && binding.keys == "escape"
                    && binding.command == "terminal.search.cancel"
            })
            .count(),
        2
    );
}

#[test]
fn conflicting_user_bindings_are_reported() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.keybindings_file(),
        r#"
        [[bindings]]
        keys = "cmd-p"
        command = "command_palette.open"

        [[bindings]]
        keys = "CMD-P"
        command = "project.palette"
    "#,
    )
    .unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.warnings.len(), 1);
    assert!(matches!(
        &loaded.warnings[0],
        KeybindingLoadWarning::Conflict(conflict) if conflict.keys == "cmd-p"
    ));
}

#[test]
fn invalid_command_id_is_reported() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.keybindings_file(),
        r#"
        [[bindings]]
        keys = "cmd-x"
        command = "missing.command"
    "#,
    )
    .unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(
        loaded.warnings,
        vec![KeybindingLoadWarning::InvalidCommand(
            "missing.command".to_string()
        )]
    );
}

#[test]
fn tab_next_command_selects_next_tab_and_marks_it_started() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::TabNext).unwrap();
    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();

    assert_eq!(project.selected_tab_id, "agent");
    assert_eq!(
        project.tab_state("agent").unwrap().start_state,
        TabStartState::Started
    );
}

#[test]
fn tab_new_command_adds_shell_tab_and_selects_it() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::TabNew).unwrap();
    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    let tab = project
        .layout
        .tabs
        .iter()
        .find(|tab| tab.id == project.selected_tab_id)
        .unwrap();

    assert_eq!(project.layout.tabs.len(), 3);
    assert_eq!(tab.id, "tab-1");
    assert_eq!(tab.title, "Tab 1");
    assert_eq!(tab.layout.pane_id(), Some("shell"));
    assert_eq!(
        project.tab_state("tab-1").unwrap().start_state,
        TabStartState::Started
    );
    assert_focused_pane(&workspace, "shell");
}

#[test]
fn tab_close_command_removes_selected_tab() {
    let mut workspace = workspace_with_sample_project();
    workspace.select_tab("agent").unwrap();

    dispatch_workspace_command(&mut workspace, CommandId::TabClose).unwrap();

    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    assert_eq!(project.selected_tab_id, "dev");
    assert!(project.layout.tab("agent").is_none());
}

#[test]
fn tab_rename_command_changes_selected_tab_title() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::TabRename).unwrap();

    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    assert_eq!(project.layout.tab("dev").unwrap().title, "Renamed Tab");
    assert!(project.layout.tab("dev").is_some());
}

#[test]
fn pane_split_vertical_command_adds_pane_to_current_tab() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::PaneSplitVertical).unwrap();

    assert_eq!(visible_pane_titles(&workspace).len(), 3);
}

#[test]
fn pane_close_command_removes_focused_pane() {
    let mut workspace = workspace_with_sample_project();
    dispatch_workspace_command(&mut workspace, CommandId::PaneSplitVertical).unwrap();

    dispatch_workspace_command(&mut workspace, CommandId::PaneClose).unwrap();

    assert_eq!(visible_pane_titles(&workspace).len(), 2);
}

#[test]
fn pane_close_command_closes_single_pane_tab_by_context() {
    let mut workspace = workspace_with_sample_project();
    workspace.select_tab("agent").unwrap();

    dispatch_workspace_command(&mut workspace, CommandId::PaneClose).unwrap();

    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    assert_eq!(project.selected_tab_id, "dev");
    assert!(project.layout.tab("agent").is_none());
}

#[test]
fn pane_rename_command_changes_focused_pane_title() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::PaneRename).unwrap();

    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    let pane = project
        .layout
        .tab("dev")
        .unwrap()
        .layout
        .find_pane("server")
        .unwrap();
    assert_eq!(pane.id, "server");
    assert_eq!(pane.title, "Renamed Pane");
}

#[test]
fn pane_focus_commands_move_between_adjacent_panes() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::PaneFocusRight).unwrap();
    assert_focused_pane(&workspace, "shell");

    dispatch_workspace_command(&mut workspace, CommandId::PaneFocusLeft).unwrap();
    assert_focused_pane(&workspace, "server");
}

#[test]
fn pane_resize_commands_adjust_current_split_ratio() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::PaneResizeRight).unwrap();
    assert_ratio(root_split_ratio(&workspace), 0.7);

    dispatch_workspace_command(&mut workspace, CommandId::PaneResizeLeft).unwrap();
    assert_ratio(root_split_ratio(&workspace), 0.65);
}

fn workspace_with_sample_project() -> Workspace {
    let mut workspace = Workspace::new();
    workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();
    workspace
}

fn root_split_ratio(workspace: &Workspace) -> f32 {
    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    let tab = project
        .layout
        .tabs
        .iter()
        .find(|tab| tab.id == project.selected_tab_id)
        .unwrap();
    match &tab.layout {
        LayoutNode::Split(split) => split.ratio,
        LayoutNode::Pane(_) => panic!("sample tab should be split"),
    }
}

fn assert_ratio(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.001,
        "expected ratio {expected}, got {actual}"
    );
}

fn legacy_v3_default_keybindings() -> KeybindingsConfig {
    let mut legacy = default_keybindings();
    legacy.schema_version = 3;
    legacy
        .bindings
        .retain(|binding| !matches!(binding.command.as_str(), "tab.next" | "tab.prev"));
    let cmd_w_index = legacy
        .bindings
        .iter()
        .position(|binding| binding.keys == "cmd-w" && binding.command == "pane.close")
        .unwrap();
    let mut ctrl_w = legacy.bindings[cmd_w_index].clone();
    ctrl_w.keys = "ctrl-w".to_string();
    legacy.bindings.insert(cmd_w_index + 1, ctrl_w);
    for binding in &mut legacy.bindings {
        binding.context = None;
    }
    legacy
}

fn legacy_v2_default_keybindings() -> KeybindingsConfig {
    let mut legacy = legacy_v3_default_keybindings();
    legacy.schema_version = 2;
    legacy
        .bindings
        .retain(|binding| binding.command != "file.find");
    for binding in &mut legacy.bindings {
        match (binding.keys.as_str(), binding.command.as_str()) {
            ("cmd-shift-p", "command_palette.open") => binding.keys = "cmd-p".to_string(),
            ("ctrl-shift-p", "command_palette.open") => binding.keys = "ctrl-p".to_string(),
            ("cmd-alt-p", "project.opened_palette") => binding.keys = "cmd-shift-p".to_string(),
            ("ctrl-alt-p", "project.opened_palette") => binding.keys = "ctrl-shift-p".to_string(),
            _ => {}
        }
    }
    legacy
}

fn assert_has_config_binding(config: &KeybindingsConfig, keys: &str, command: &str) {
    assert!(
        config
            .bindings
            .iter()
            .any(|binding| binding.keys == keys && binding.command == command),
        "expected default keybindings to include {keys} -> {command}"
    );
}

fn assert_has_ui_binding(keys: &str, command: &str) {
    assert!(
        default_ui_keybinding_specs()
            .iter()
            .any(|binding| binding.keys == keys && binding.command.as_str() == command),
        "expected GPUI keybindings to include {keys} -> {command}"
    );
}

fn assert_focused_pane(workspace: &Workspace, expected_pane_id: &str) {
    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    let tab = project.tab_state(&project.selected_tab_id).unwrap();
    assert_eq!(tab.focused_pane_id.as_deref(), Some(expected_pane_id));
}

fn sample_layout() -> yttt::model::layout::ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "yttt"
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"

        [tabs.layout]
        type = "split"
        direction = "horizontal"
        ratio = 0.65
        left = { type = "pane", id = "server", title = "server", command = "npm run dev" }
        right = { type = "pane", id = "shell", title = "shell", command = "$SHELL" }

        [[tabs]]
        id = "agent"
        title = "Agent"
        layout = { type = "pane", id = "codex", title = "Codex", command = "codex", kind = "agent", notify_on_exit = true }
    "#,
    )
    .unwrap()
}
