use std::fs;

use tempfile::tempdir;
use yttt::config::{
    layout_loader::{
        LayoutNodeOverride, LayoutOverride, MergeWarning, PaneOverride, TabOverride,
        export_project_layout, load_recent_projects, merge_layouts, open_project_config,
        save_local_layout,
    },
    paths::AppConfigPaths,
};
use yttt::model::layout::ProjectLayout;

#[test]
fn parses_project_layout_with_split_and_agent_pane() {
    let source = r#"
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

        [tabs.layout]
        type = "pane"
        id = "codex"
        title = "Codex"
        command = "codex"
        kind = "agent"
        notify_on_exit = true
    "#;

    let layout: ProjectLayout = toml::from_str(source).unwrap();

    assert_eq!(layout.project.name, "yttt");
    assert_eq!(layout.project.default_tab.as_deref(), Some("dev"));
    assert_eq!(layout.tabs.len(), 2);
    assert_eq!(layout.tabs[1].layout.pane_id(), Some("codex"));
}

#[test]
fn parses_optional_pane_detector_field() {
    let source = r#"
        [project]
        name = "yttt"
        default_tab = "agent"

        [[tabs]]
        id = "agent"
        title = "Agent"
        layout = { type = "pane", id = "codex", title = "Codex", command = "codex", kind = "agent", detector = "codex" }
    "#;

    let layout: ProjectLayout = toml::from_str(source).unwrap();

    assert_eq!(
        layout.tabs[0]
            .layout
            .find_pane("codex")
            .unwrap()
            .detector
            .as_deref(),
        Some("codex")
    );
}

#[test]
fn rejects_duplicate_tab_ids() {
    let source = r#"
        [project]
        name = "yttt"
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"
        layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }

        [[tabs]]
        id = "dev"
        title = "Duplicate"
        layout = { type = "pane", id = "dup", title = "Dup", command = "$SHELL" }
    "#;

    let layout: ProjectLayout = toml::from_str(source).unwrap();

    assert!(layout.validate().is_err());
}

#[test]
fn rejects_invalid_split_ratio() {
    let source = r#"
        [project]
        name = "yttt"
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"

        [tabs.layout]
        type = "split"
        direction = "vertical"
        ratio = 1.5
        left = { type = "pane", id = "left", title = "Left", command = "$SHELL" }
        right = { type = "pane", id = "right", title = "Right", command = "$SHELL" }
    "#;

    let layout: ProjectLayout = toml::from_str(source).unwrap();

    assert!(layout.validate().is_err());
}

#[test]
fn rejects_default_tab_that_does_not_exist() {
    let source = r#"
        [project]
        name = "yttt"
        default_tab = "missing"

        [[tabs]]
        id = "dev"
        title = "Dev"
        layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }
    "#;

    let layout: ProjectLayout = toml::from_str(source).unwrap();

    assert!(layout.validate().is_err());
}

#[test]
fn local_override_renames_tab_and_command_by_id() {
    let base = sample_layout();
    let override_layout = LayoutOverride {
        tabs: vec![TabOverride {
            id: "dev".to_string(),
            title: Some("Development".to_string()),
            layout: Some(LayoutNodeOverride::Pane(PaneOverride {
                id: "server".to_string(),
                title: None,
                command: Some("pnpm dev".to_string()),
                kind: None,
                notify_on_exit: None,
                detector: Some("codex".to_string()),
            })),
        }],
        ..Default::default()
    };

    let result = merge_layouts(&base, &override_layout).unwrap();
    let dev = result.layout.tab("dev").unwrap();

    assert_eq!(dev.title, "Development");
    assert_eq!(dev.layout.find_pane("server").unwrap().command, "pnpm dev");
    assert_eq!(
        dev.layout.find_pane("server").unwrap().detector.as_deref(),
        Some("codex")
    );
    assert!(result.warnings.is_empty());
}

#[test]
fn stale_override_ids_are_reported_and_ignored() {
    let base = sample_layout();
    let override_layout = LayoutOverride {
        tabs: vec![TabOverride {
            id: "missing".to_string(),
            title: Some("Missing".to_string()),
            layout: None,
        }],
        ..Default::default()
    };

    let result = merge_layouts(&base, &override_layout).unwrap();

    assert_eq!(result.layout.tabs.len(), 2);
    assert_eq!(
        result.warnings,
        vec![MergeWarning::StaleTabOverride("missing".to_string())]
    );
}

#[test]
fn config_missing_project_layout_creates_default_layout_in_app_config() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("sample-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let opened = open_project_config(&paths, &project_dir).unwrap();

    assert_eq!(opened.layout.project.name, "sample-project");
    assert!(paths.local_layout_file(&project_dir).exists());
    assert!(!project_dir.join(".yttt/layout.toml").exists());
}

#[test]
fn config_invalid_project_toml_returns_visible_load_error() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("broken-project");
    let project_config_dir = project_dir.join(".yttt");
    fs::create_dir_all(&project_config_dir).unwrap();
    fs::write(project_config_dir.join("layout.toml"), "[project\n").unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let err = open_project_config(&paths, &project_dir).unwrap_err();

    assert!(err.to_string().contains("failed to parse project layout"));
    assert!(err.to_string().contains("layout.toml"));
}

#[test]
fn config_recent_projects_are_stored_in_app_config() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("recent-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    open_project_config(&paths, &project_dir).unwrap();
    let recent = load_recent_projects(&paths).unwrap();

    assert_eq!(recent.projects.len(), 1);
    assert_eq!(recent.projects[0].title, "recent-project");
    assert_eq!(recent.projects[0].path, project_dir.canonicalize().unwrap());
    assert!(paths.recent_projects_file().exists());
}

#[test]
fn save_current_layout_writes_app_local_override() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("save-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let layout = sample_layout();

    let path = save_local_layout(&paths, &project_dir, &layout).unwrap();
    let saved: ProjectLayout = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(path, paths.local_layout_file(&project_dir));
    assert_eq!(saved, layout);
}

#[test]
fn export_project_config_writes_project_layout_toml() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("export-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let layout = sample_layout();

    let path = export_project_layout(&paths, &project_dir, &layout).unwrap();
    let saved: ProjectLayout = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(path, project_dir.join(".yttt/layout.toml"));
    assert_eq!(saved, layout);
}

#[test]
fn save_layout_creates_parent_directories() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("nested").join("save-project");
    fs::create_dir_all(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("missing").join("config"));

    let path = save_local_layout(&paths, &project_dir, &sample_layout()).unwrap();

    assert!(path.parent().unwrap().exists());
}

fn sample_layout() -> ProjectLayout {
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
