use yttt::config::layout_loader::{
    merge_layouts, LayoutNodeOverride, LayoutOverride, MergeWarning, PaneOverride, TabOverride,
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
            })),
        }],
        ..Default::default()
    };

    let result = merge_layouts(&base, &override_layout).unwrap();
    let dev = result.layout.tab("dev").unwrap();

    assert_eq!(dev.title, "Development");
    assert_eq!(
        dev.layout.find_pane("server").unwrap().command,
        "pnpm dev"
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
