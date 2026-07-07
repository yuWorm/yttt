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
