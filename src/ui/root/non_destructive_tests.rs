use std::{collections::HashMap, fs};

use gpui::{EntityId, TestAppContext};
use tempfile::tempdir;

use super::*;

#[derive(Clone)]
struct RuntimeSnapshot {
    layout: ProjectLayout,
    terminal_entities: HashMap<String, EntityId>,
}

fn runtime_snapshot(root: &RootView) -> RuntimeSnapshot {
    let project_id = root.workspace.selected_project_id().unwrap();
    RuntimeSnapshot {
        layout: root.workspace.project(project_id).unwrap().layout.clone(),
        terminal_entities: root
            .terminal_panes
            .iter()
            .map(|(key, pane)| (key.clone(), pane.entity_id()))
            .collect(),
    }
}

fn assert_runtime_unchanged(root: &RootView, expected: &RuntimeSnapshot) {
    let actual = runtime_snapshot(root);
    assert_eq!(actual.layout, expected.layout);
    assert_eq!(actual.terminal_entities, expected.terminal_entities);
}

#[gpui::test]
fn layout_default_does_not_drop_terminal_entities(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(project_path, dev_fixture_layout())
        .unwrap();

    let (root, cx) = cx.add_window_view(|_, _| {
        RootView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });

    root.update(cx, |root, _| {
        let before = runtime_snapshot(root);
        assert!(
            !before.terminal_entities.is_empty(),
            "render must create real terminal pane entities"
        );

        root.run_command(CommandId::LayoutDefaultEdit).unwrap();
        let updated = root
            .layout_toml_editor_value()
            .unwrap()
            .replace("title = \"Shell\"", "title = \"Saved Default\"");
        root.set_layout_toml_editor_value(updated);
        root.save_layout_toml_editor().unwrap();
        assert_runtime_unchanged(root, &before);

        root.run_command(CommandId::LayoutDefaultReload).unwrap();
        assert_runtime_unchanged(root, &before);

        root.run_command(CommandId::LayoutDefaultReset).unwrap();
        assert_runtime_unchanged(root, &before);

        root.run_command(CommandId::LayoutSaveCurrent).unwrap();
        root.run_command(CommandId::LayoutResetLocalOverride)
            .unwrap();
        assert_runtime_unchanged(root, &before);
    });
}
