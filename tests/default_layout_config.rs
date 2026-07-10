use std::{fs, path::PathBuf};

use tempfile::tempdir;

use yttt::{
    config::{
        default_layout::{
            DefaultLayoutSource, DefaultLayoutState, DefaultLayoutTemplate, LayoutLoadWarning,
        },
        paths::AppConfigPaths,
    },
    model::layout::{LayoutError, PaneKind},
};

#[test]
fn default_layout_file_is_stored_in_app_config() {
    let paths = AppConfigPaths::from_config_dir("/tmp/yttt-config");

    assert_eq!(
        paths.default_layout_file(),
        PathBuf::from("/tmp/yttt-config/default-layout.toml")
    );
}

#[test]
fn builtin_template_is_valid_shell_layout() {
    let template = DefaultLayoutTemplate::builtin();

    assert_eq!(template.project.default_tab.as_deref(), Some("shell"));
    assert_eq!(template.tabs.len(), 1);
    assert_eq!(template.tabs[0].id, "shell");
    assert_eq!(template.tabs[0].layout.pane_id(), Some("shell"));
    assert_eq!(
        template.tabs[0].layout.find_pane("shell").unwrap().kind,
        PaneKind::Shell
    );
    assert_eq!(template.validate(), Ok(()));
}

#[test]
fn materialize_sets_project_name_and_preserves_builtin_ids() {
    let layout = DefaultLayoutTemplate::builtin().materialize("sample-project");

    assert_eq!(layout.project.name, "sample-project");
    assert_eq!(layout.project.default_tab.as_deref(), Some("shell"));
    assert_eq!(layout.tabs[0].id, "shell");
    assert_eq!(layout.tabs[0].layout.pane_id(), Some("shell"));
}

#[test]
fn template_validation_rejects_duplicate_pane_ids() {
    let template: DefaultLayoutTemplate = toml::from_str(
        r#"
        [project]
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"

        [tabs.layout]
        type = "split"
        direction = "horizontal"
        ratio = 0.5
        left = { type = "pane", id = "shell", title = "Left", command = "$SHELL" }
        right = { type = "pane", id = "shell", title = "Right", command = "$SHELL" }
    "#,
    )
    .unwrap();

    assert_eq!(
        template.validate(),
        Err(LayoutError::DuplicatePaneId {
            tab_id: "dev".to_string(),
            pane_id: "shell".to_string(),
        })
    );
}

#[test]
fn default_layout_state_creates_builtin_file_when_missing() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let state = DefaultLayoutState::load_or_create(&paths);

    assert_eq!(state.template(), &DefaultLayoutTemplate::builtin());
    assert_eq!(
        state.source(),
        &DefaultLayoutSource::ConfigFile(paths.default_layout_file())
    );
    assert!(state.warnings().is_empty());
    let saved: DefaultLayoutTemplate =
        toml::from_str(&fs::read_to_string(paths.default_layout_file()).unwrap()).unwrap();
    assert_eq!(saved, DefaultLayoutTemplate::builtin());
}

#[test]
fn default_layout_state_loads_valid_config_file() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut template = DefaultLayoutTemplate::builtin();
    template.tabs[0].title = "Configured Shell".to_string();
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(
        paths.default_layout_file(),
        toml::to_string_pretty(&template).unwrap(),
    )
    .unwrap();

    let state = DefaultLayoutState::load_or_create(&paths);

    assert_eq!(state.template(), &template);
    assert_eq!(
        state.source(),
        &DefaultLayoutSource::ConfigFile(paths.default_layout_file())
    );
    assert!(state.warnings().is_empty());
}

#[test]
fn default_layout_state_invalid_startup_file_falls_back_to_builtin() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(paths.default_layout_file(), "[project").unwrap();

    let state = DefaultLayoutState::load_or_create(&paths);

    assert_eq!(state.template(), &DefaultLayoutTemplate::builtin());
    assert_eq!(state.source(), &DefaultLayoutSource::BuiltIn);
    assert!(matches!(
        state.warnings(),
        [LayoutLoadWarning::GlobalDefaultParse { path, message }]
            if path == &paths.default_layout_file() && !message.is_empty()
    ));
}

#[test]
fn default_layout_state_invalid_reload_preserves_last_known_good() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut configured = DefaultLayoutTemplate::builtin();
    configured.tabs[0].title = "Known Good".to_string();
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(
        paths.default_layout_file(),
        toml::to_string_pretty(&configured).unwrap(),
    )
    .unwrap();
    let mut state = DefaultLayoutState::load_or_create(&paths);
    fs::write(paths.default_layout_file(), "[project").unwrap();

    let error = state.reload().unwrap_err();

    assert!(matches!(
        error,
        LayoutLoadWarning::GlobalDefaultParse { .. }
    ));
    assert_eq!(state.template(), &configured);
    assert_eq!(
        state.source(),
        &DefaultLayoutSource::ConfigFile(paths.default_layout_file())
    );
    assert_eq!(state.warnings(), std::slice::from_ref(&error));
}

#[test]
fn default_layout_state_reload_accepts_valid_external_change() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut state = DefaultLayoutState::load_or_create(&paths);
    let mut external = DefaultLayoutTemplate::builtin();
    external.tabs[0].title = "External".to_string();
    fs::write(
        paths.default_layout_file(),
        toml::to_string_pretty(&external).unwrap(),
    )
    .unwrap();

    state.reload().unwrap();

    assert_eq!(state.template(), &external);
    assert!(state.warnings().is_empty());
}

#[test]
fn default_layout_state_save_updates_file_and_cache() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut state = DefaultLayoutState::load_or_create(&paths);
    let mut updated = DefaultLayoutTemplate::builtin();
    updated.tabs[0].title = "Saved".to_string();

    state.save(updated.clone()).unwrap();

    assert_eq!(state.template(), &updated);
    assert_eq!(
        state.source(),
        &DefaultLayoutSource::ConfigFile(paths.default_layout_file())
    );
    let saved: DefaultLayoutTemplate =
        toml::from_str(&fs::read_to_string(paths.default_layout_file()).unwrap()).unwrap();
    assert_eq!(saved, updated);
}

#[test]
fn default_layout_state_reset_writes_and_uses_builtin() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut state = DefaultLayoutState::load_or_create(&paths);
    let mut changed = DefaultLayoutTemplate::builtin();
    changed.tabs[0].title = "Changed".to_string();
    state.save(changed).unwrap();

    state.reset().unwrap();

    assert_eq!(state.template(), &DefaultLayoutTemplate::builtin());
    let saved: DefaultLayoutTemplate =
        toml::from_str(&fs::read_to_string(paths.default_layout_file()).unwrap()).unwrap();
    assert_eq!(saved, DefaultLayoutTemplate::builtin());
}
