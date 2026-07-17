use std::{cell::RefCell, fs, path::PathBuf, rc::Rc};

use gpui::AppContext as _;
use tempfile::tempdir;
use yttt::{
    config::{paths::AppConfigPaths, settings::load_or_create_settings},
    model::{
        ids::ProjectId,
        project::{ProjectDescriptor, ProjectLocation},
        workspace::Workspace,
    },
    ui::workbench::WorkbenchView,
};

fn local_project(path: PathBuf) -> ProjectDescriptor {
    let location = ProjectLocation::local(path);
    ProjectDescriptor::new(
        ProjectId::from_legacy_location(&location.display_path()),
        location,
    )
}

#[gpui::test]
fn performance_metrics_render_sample_and_toggle_from_settings(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    let view_paths = paths.clone();
    let workspace = workspace_with_sample_project();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| {
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, view_paths)
        });
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        root.open_settings();
        cx.notify();
    });
    cx.run_until_parked();

    for selector in [
        "titlebar-performance-projects",
        "titlebar-performance-terminals",
        "titlebar-performance-tabs",
        "titlebar-performance-editors",
        "titlebar-performance-cpu",
        "titlebar-performance-memory",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "{selector} should be visible"
        );
    }
    assert!(
        cx.debug_bounds("titlebar-system-performance-metrics")
            .is_none()
    );
    cx.read(|app| {
        let metrics = root
            .read(app)
            .visible_titlebar_performance()
            .expect("enabled application metrics should be visible");
        let application = metrics
            .application
            .expect("application metrics should be enabled by default");
        assert!(metrics.system.is_none());
        assert_eq!(application.projects.value, "1");
        assert_eq!(application.projects.tooltip, "Projects: 1");
        assert_eq!(application.terminals.value, "2");
        assert_eq!(application.tabs.value, "2");
        assert_eq!(application.editors.value, "0");
        assert_eq!(application.cpu.value, "—");
        assert_eq!(application.memory.value, "—");
    });

    let system_toggle = cx
        .debug_bounds("settings-system-performance-metrics")
        .expect("general settings should expose the system performance switch");
    cx.simulate_click(system_toggle.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        let root = root.read(app);
        assert!(root.system_performance_metrics_enabled());
        let metrics = root
            .visible_titlebar_performance()
            .expect("system metrics should be visible after enabling");
        let system = metrics.system.expect("system metrics should have a sample");
        assert_ne!(system.cpu.value, "—");
        assert_ne!(system.memory.value, "—");
        assert!(system.cpu.value.ends_with('%'));
        assert!(system.memory.value.ends_with('%'));
        assert!(system.cpu.tooltip.starts_with("System CPU: "));
        assert!(system.memory.tooltip.starts_with("System memory: "));
    });
    assert!(cx.debug_bounds("titlebar-system-cpu").is_some());
    assert!(cx.debug_bounds("titlebar-system-memory").is_some());
    assert!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .general
            .system_performance_metrics_enabled
    );

    let application_toggle = cx
        .debug_bounds("settings-performance-metrics")
        .expect("application performance switch should remain available");
    cx.simulate_click(application_toggle.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        let root = root.read(app);
        assert!(!root.performance_metrics_enabled());
        let metrics = root
            .visible_titlebar_performance()
            .expect("system metrics should remain independently visible");
        assert!(metrics.application.is_none());
        assert!(metrics.system.is_some());
    });
    assert!(
        cx.debug_bounds("titlebar-application-performance-metrics")
            .is_none()
    );
    assert!(
        cx.debug_bounds("titlebar-system-performance-metrics")
            .is_some()
    );

    let system_toggle = cx
        .debug_bounds("settings-system-performance-metrics")
        .expect("system performance switch should remain available");
    cx.simulate_click(system_toggle.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        let root = root.read(app);
        assert!(!root.system_performance_metrics_enabled());
        assert!(root.visible_titlebar_performance().is_none());
    });
    assert!(cx.debug_bounds("titlebar-performance-metrics").is_none());
    assert!(
        !load_or_create_settings(&paths)
            .unwrap()
            .settings
            .general
            .system_performance_metrics_enabled
    );

    let application_toggle = cx
        .debug_bounds("settings-performance-metrics")
        .expect("application performance switch should remain available");
    cx.simulate_click(application_toggle.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        let root = root.read(app);
        assert!(root.performance_metrics_enabled());
        let metrics = root
            .visible_titlebar_performance()
            .expect("re-enabled application metrics should be visible");
        let application = metrics
            .application
            .expect("application metrics should have a sample");
        assert!(metrics.system.is_none());
        assert_ne!(application.cpu.value, "—");
        assert_ne!(application.memory.value, "—");
        assert!(application.memory.value.ends_with(" MiB"));
    });
    assert!(
        cx.debug_bounds("titlebar-application-performance-metrics")
            .is_some()
    );
}

fn english_test_config_paths(temp: &tempfile::TempDir) -> AppConfigPaths {
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(
        paths.settings_file(),
        r#"
[general]
language = "en"
"#,
    )
    .unwrap();
    paths
}

fn workspace_with_sample_project() -> Workspace {
    let mut workspace = Workspace::new();
    let layout = toml::from_str(
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
    .unwrap();
    workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), layout)
        .unwrap();
    workspace
}
