use std::{cell::RefCell, fs, path::PathBuf, rc::Rc, time::Duration};

use gpui::AppContext as _;
use tempfile::tempdir;
use yttt::{
    config::{
        bars::{ShellBarModule, ShellBarsSettings, save_bars},
        paths::AppConfigPaths,
    },
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

fn focus_surface_window(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    cx.cx.refresh().unwrap();
    cx.run_until_parked();
    let mut matching = cx.windows().into_iter().filter_map(|window| {
        let mut candidate = gpui::VisualTestContext::from_window(window, &cx.cx);
        candidate.refresh().unwrap();
        candidate.debug_bounds(selector).map(|_| candidate)
    });
    let mut target = matching
        .next()
        .unwrap_or_else(|| panic!("missing native surface: {selector}"));
    assert!(
        matching.next().is_none(),
        "duplicate native surface: {selector}"
    );
    drop(matching);
    target.update(|window, _| window.activate_window());
    target.run_until_parked();
    *cx = target;
}

#[gpui::test]
fn performance_metrics_are_cached_independently_of_bar_configuration(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    let mut bars_without_metrics = ShellBarsSettings::default();
    bars_without_metrics.window.layout = Default::default();
    bars_without_metrics.status.layout = Default::default();
    save_bars(&paths, &bars_without_metrics).unwrap();

    let mut metric_bars = bars_without_metrics.clone();
    metric_bars.window.layout.left = vec![
        ShellBarModule::AppCpu,
        ShellBarModule::AppMemory,
        ShellBarModule::SystemCpu,
        ShellBarModule::SystemMemory,
    ];
    let metric_draft = toml::to_string_pretty(&metric_bars).unwrap();

    let view_paths = paths.clone();
    let workspace = workspace_with_sample_project();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (first_component_root, main_cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| {
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, view_paths)
        });
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    main_cx.run_until_parked();
    main_cx.refresh().unwrap();

    let assert_cached_performance =
        |view: &gpui::Entity<WorkbenchView>, view_cx: &gpui::VisualTestContext| {
            view_cx.read(|app| {
                let metrics = view
                    .read(app)
                    .visible_performance_info()
                    .expect("the shared sampler should publish a cached metric sample");
                let application = metrics
                    .application
                    .expect("application metrics should be available without bar modules");
                let system = metrics
                    .system
                    .expect("system metrics should be available without bar modules");

                assert_eq!(application.projects.value, "1");
                assert_eq!(application.terminals.value, "2");
                assert_eq!(application.tabs.value, "2");
                assert_eq!(application.editors.value, "0");
                assert_ne!(application.cpu.value, "—");
                assert!(application.cpu.value.ends_with('%'));
                assert_ne!(application.memory.value, "—");
                assert!(application.memory.value.ends_with(" MiB"));
                assert_ne!(system.cpu.value, "—");
                assert!(system.cpu.value.ends_with('%'));
                assert_ne!(system.memory.value, "—");
                assert!(system.memory.value.ends_with('%'));
            });
        };

    assert_cached_performance(&root, &main_cx);
    let first_window = main_cx.update(|window, _| window.window_handle());
    let second_root_slot = Rc::new(RefCell::new(None));
    let second_root_slot_for_window = second_root_slot.clone();
    let second_paths = paths.clone();
    let second_workspace = workspace_with_sample_project();
    let (_second_component_root, second_cx) = main_cx.cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| {
            WorkbenchView::with_workspace_for_test_and_config_paths(second_workspace, second_paths)
        });
        *second_root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let second_root = second_root_slot.borrow_mut().take().unwrap();
    assert_cached_performance(&second_root, &second_cx);

    first_window
        .update(second_cx, |_, window, _| window.remove_window())
        .unwrap();
    second_cx.cx.refresh().unwrap();
    assert!(
        !second_cx.windows().contains(&first_window),
        "the first workbench window should close"
    );
    drop(root);
    drop(first_component_root);
    second_cx
        .background_executor
        .advance_clock(Duration::from_secs(1));
    second_cx.run_until_parked();
    assert_cached_performance(&second_root, &second_cx);

    let root = second_root;
    let mut main_cx = second_cx;
    for selector in [
        "window-bar-app-cpu",
        "window-bar-app-memory",
        "window-bar-system-cpu",
        "window-bar-system-memory",
    ] {
        assert!(
            main_cx.debug_bounds(selector).is_none(),
            "{selector} should not render before its bar module is configured"
        );
    }

    let persisted_before_draft = fs::read_to_string(paths.bars_file()).unwrap();
    root.update(main_cx, |root, cx| {
        root.open_bars_toml_editor().unwrap();
        root.set_layout_toml_editor_value(metric_draft.clone());
        cx.notify();
    });
    main_cx.run_until_parked();
    focus_surface_window(&mut main_cx, "layout-editor-window");
    assert!(main_cx.debug_bounds("bars-editor-preview").is_some());
    for selector in [
        "window-bar-app-cpu",
        "window-bar-app-memory",
        "window-bar-system-cpu",
        "window-bar-system-memory",
    ] {
        assert!(
            main_cx.debug_bounds(selector).is_some(),
            "{selector} should render in the unsaved bars preview"
        );
    }
    assert_eq!(
        fs::read_to_string(paths.bars_file()).unwrap(),
        persisted_before_draft,
        "the preview must use the cached sample without saving the draft"
    );
    assert_cached_performance(&root, &main_cx);

    root.update(main_cx, |root, cx| {
        root.save_layout_toml_editor().unwrap();
        cx.notify();
    });
    main_cx.run_until_parked();
    focus_surface_window(&mut main_cx, "window-bar");
    for selector in [
        "window-bar-app-cpu",
        "window-bar-app-memory",
        "window-bar-system-cpu",
        "window-bar-system-memory",
    ] {
        assert!(
            main_cx.debug_bounds(selector).is_some(),
            "{selector} should render after its bar template is saved"
        );
    }
    assert_cached_performance(&root, &main_cx);

    let bars_without_metrics_draft = toml::to_string_pretty(&bars_without_metrics).unwrap();
    root.update(main_cx, |root, cx| {
        root.open_bars_toml_editor().unwrap();
        root.set_layout_toml_editor_value(bars_without_metrics_draft.clone());
        root.save_layout_toml_editor().unwrap();
        cx.notify();
    });
    main_cx.run_until_parked();
    main_cx.refresh().unwrap();
    for selector in [
        "window-bar-app-cpu",
        "window-bar-app-memory",
        "window-bar-system-cpu",
        "window-bar-system-memory",
    ] {
        assert!(
            main_cx.debug_bounds(selector).is_none(),
            "{selector} should disappear when its bar module is removed"
        );
    }
    assert_cached_performance(&root, &main_cx);

    let mut status_disabled_bars = bars_without_metrics;
    status_disabled_bars.status.enabled = false;
    status_disabled_bars.status.layout.left = metric_bars.window.layout.left.clone();
    let status_disabled_draft = toml::to_string_pretty(&status_disabled_bars).unwrap();
    root.update(main_cx, |root, cx| {
        root.open_bars_toml_editor().unwrap();
        root.set_layout_toml_editor_value(status_disabled_draft.clone());
        root.save_layout_toml_editor().unwrap();
        cx.notify();
    });
    main_cx.run_until_parked();
    main_cx.refresh().unwrap();
    assert!(
        main_cx.debug_bounds("status-bar").is_none(),
        "the status bar should not render when disabled"
    );
    assert_cached_performance(&root, &main_cx);
}

fn english_test_config_paths(temp: &tempfile::TempDir) -> AppConfigPaths {
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let device_settings_file = paths.config_dir().join("device/settings.toml");
    fs::create_dir_all(device_settings_file.parent().unwrap()).unwrap();
    fs::write(
        device_settings_file,
        r#"
[general]
language = "en"
onboarding_completed = true
performance_metrics_enabled = false
system_performance_metrics_enabled = false
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
