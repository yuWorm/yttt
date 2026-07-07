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
