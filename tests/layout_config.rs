use std::fs;

use tempfile::tempdir;
use yttt::config::{
    default_layout::{DefaultLayoutState, DefaultLayoutTemplate, LayoutLoadWarning},
    layout_loader::{
        LayoutNodeOverride, LayoutOverride, LayoutSource, MergeWarning, PaneOverride,
        PersonalLayout, ProjectOpenError, TabOverride, export_project_layout, load_recent_projects,
        merge_layouts, open_project_config, parse_personal_layout, reset_local_override,
        save_local_layout, serialize_personal_patch, serialize_personal_replace,
    },
    paths::AppConfigPaths,
};
use yttt::model::layout::{LayoutError, ProjectLayout};

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
fn rejects_duplicate_pane_ids_in_the_same_tab() {
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
        ratio = 0.5
        left = { type = "pane", id = "shell", title = "Left", command = "$SHELL" }
        right = { type = "pane", id = "shell", title = "Right", command = "$SHELL" }
    "#;

    let layout: ProjectLayout = toml::from_str(source).unwrap();

    assert_eq!(
        layout.validate(),
        Err(LayoutError::DuplicatePaneId {
            tab_id: "dev".to_string(),
            pane_id: "shell".to_string(),
        })
    );
}

#[test]
fn rejects_duplicate_pane_ids_in_a_nested_split() {
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
        ratio = 0.5
        left = { type = "pane", id = "shell", title = "Left", command = "$SHELL" }

        [tabs.layout.right]
        type = "split"
        direction = "vertical"
        ratio = 0.5
        left = { type = "pane", id = "server", title = "Server", command = "cargo run" }
        right = { type = "pane", id = "shell", title = "Nested", command = "$SHELL" }
    "#;

    let layout: ProjectLayout = toml::from_str(source).unwrap();

    assert_eq!(
        layout.validate(),
        Err(LayoutError::DuplicatePaneId {
            tab_id: "dev".to_string(),
            pane_id: "shell".to_string(),
        })
    );
}

#[test]
fn allows_the_same_pane_id_in_different_tabs() {
    let source = r#"
        [project]
        name = "yttt"
        default_tab = "one"

        [[tabs]]
        id = "one"
        title = "One"
        layout = { type = "pane", id = "shell", title = "One", command = "$SHELL" }

        [[tabs]]
        id = "two"
        title = "Two"
        layout = { type = "pane", id = "shell", title = "Two", command = "$SHELL" }
    "#;

    let layout: ProjectLayout = toml::from_str(source).unwrap();

    assert_eq!(layout.validate(), Ok(()));
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
fn personal_layout_v1_parses_patch_and_applies_project_default_tab() {
    let path = std::path::Path::new("/tmp/personal-layout.toml");
    let source = r#"
        version = 1
        mode = "patch"

        [layout.project]
        default_tab = "agent"

        [[layout.tabs]]
        id = "agent"
        title = "Personal Agent"
    "#;

    let PersonalLayout::Patch(patch) = parse_personal_layout(path, source).unwrap() else {
        panic!("expected patch personal layout");
    };
    let merged = merge_layouts(&sample_layout(), &patch).unwrap();

    assert_eq!(merged.layout.project.default_tab.as_deref(), Some("agent"));
    assert_eq!(merged.layout.tab("agent").unwrap().title, "Personal Agent");
}

#[test]
fn personal_layout_v1_parses_replace_and_validates_domain_layout() {
    let path = std::path::Path::new("/tmp/personal-layout.toml");
    let source = r#"
        version = 1
        mode = "replace"

        [layout.project]
        name = "personal"
        default_tab = "shell"

        [[layout.tabs]]
        id = "shell"
        title = "Shell"
        layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }
    "#;

    let PersonalLayout::Replace(layout) = parse_personal_layout(path, source).unwrap() else {
        panic!("expected replacement personal layout");
    };

    assert_eq!(layout.project.name, "personal");
    assert_eq!(layout.project.default_tab.as_deref(), Some("shell"));
    assert_eq!(layout.tabs[0].layout.pane_id(), Some("shell"));
}

#[test]
fn personal_layout_v1_maps_toml_syntax_errors_to_parse_errors() {
    let path = std::path::Path::new("/tmp/personal-layout.toml");

    let error = parse_personal_layout(path, "[layout").unwrap_err();

    assert!(matches!(
        error,
        ProjectOpenError::PersonalOverrideParse {
            path: error_path,
            ..
        } if error_path == path
    ));
}

#[test]
fn personal_layout_v1_rejects_missing_or_unknown_header_values() {
    let path = std::path::Path::new("/tmp/personal-layout.toml");
    let fixtures = [
        ("missing version", "mode = \"patch\"\nlayout = {}"),
        ("missing mode", "version = 1\nlayout = {}"),
        (
            "unknown version",
            "version = 2\nmode = \"patch\"\nlayout = {}",
        ),
        ("unknown mode", "version = 1\nmode = \"merge\"\nlayout = {}"),
        ("missing layout", "version = 1\nmode = \"patch\""),
    ];

    for (fixture, source) in fixtures {
        let error = parse_personal_layout(path, source).unwrap_err();
        assert!(
            matches!(
                error,
                ProjectOpenError::PersonalOverrideValidation {
                    path: ref error_path,
                    ..
                } if error_path == path
            ),
            "fixture {fixture} returned {error:?}"
        );
    }
}

#[test]
fn personal_layout_v1_rejects_mode_body_mismatches() {
    let path = std::path::Path::new("/tmp/personal-layout.toml");
    let fixtures = [
        (
            "patch with replace-only split",
            r#"
                version = 1
                mode = "patch"

                [[layout.tabs]]
                id = "dev"

                [layout.tabs.layout]
                type = "split"
                direction = "horizontal"
                ratio = 0.5
                left = { type = "pane", id = "left", title = "Left", command = "$SHELL" }
                right = { type = "pane", id = "right", title = "Right", command = "$SHELL" }
            "#,
        ),
        (
            "replace with sparse patch",
            r#"
                version = 1
                mode = "replace"

                [[layout.tabs]]
                id = "dev"
                title = "Only a patch"
            "#,
        ),
    ];

    for (fixture, source) in fixtures {
        let error = parse_personal_layout(path, source).unwrap_err();
        assert!(
            matches!(error, ProjectOpenError::PersonalOverrideValidation { .. }),
            "fixture {fixture} returned {error:?}"
        );
    }
}

#[test]
fn personal_layout_v1_rejects_unknown_fields_at_every_patch_layer() {
    let path = std::path::Path::new("/tmp/personal-layout.toml");
    let fixtures = [
        r#"version = 1
mode = "patch"
unknown = true
layout = {}"#,
        r#"version = 1
mode = "patch"
[layout]
unknown = true"#,
        r#"version = 1
mode = "patch"
[layout.project]
unknown = true"#,
        r#"version = 1
mode = "patch"
[[layout.tabs]]
id = "dev"
unknown = true"#,
        r#"version = 1
mode = "patch"
[[layout.tabs]]
id = "dev"
[layout.tabs.layout]
type = "pane"
id = "shell"
unknown = true"#,
    ];

    for source in fixtures {
        assert!(matches!(
            parse_personal_layout(path, source),
            Err(ProjectOpenError::PersonalOverrideValidation { .. })
        ));
    }
}

#[test]
fn personal_layout_v1_rejects_unknown_fields_at_every_replace_layer() {
    let path = std::path::Path::new("/tmp/personal-layout.toml");
    let fixtures = [
        r#"version = 1
mode = "replace"
[layout]
unknown = true
[layout.project]
name = "sample"
default_tab = "dev"
[[layout.tabs]]
id = "dev"
title = "Dev"
layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }"#,
        r#"version = 1
mode = "replace"
[layout.project]
name = "sample"
default_tab = "dev"
unknown = true
[[layout.tabs]]
id = "dev"
title = "Dev"
layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }"#,
        r#"version = 1
mode = "replace"
[layout.project]
name = "sample"
default_tab = "dev"
[[layout.tabs]]
id = "dev"
title = "Dev"
unknown = true
layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }"#,
        r#"version = 1
mode = "replace"
[layout.project]
name = "sample"
default_tab = "dev"
[[layout.tabs]]
id = "dev"
title = "Dev"
layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL", unknown = true }"#,
        r#"version = 1
mode = "replace"
[layout.project]
name = "sample"
default_tab = "dev"
[[layout.tabs]]
id = "dev"
title = "Dev"
[layout.tabs.layout]
type = "split"
direction = "horizontal"
ratio = 0.5
unknown = true
left = { type = "pane", id = "left", title = "Left", command = "$SHELL" }
right = { type = "pane", id = "right", title = "Right", command = "$SHELL" }"#,
    ];

    for source in fixtures {
        assert!(matches!(
            parse_personal_layout(path, source),
            Err(ProjectOpenError::PersonalOverrideValidation { .. })
        ));
    }
}

#[test]
fn personal_layout_v1_serializes_patch_and_replace_through_strict_wire_dtos() {
    let path = std::path::Path::new("/tmp/personal-layout.toml");
    let patch = LayoutOverride {
        project: Some(yttt::config::layout_loader::ProjectOverride {
            name: None,
            default_tab: Some("agent".to_string()),
        }),
        ..Default::default()
    };

    let patch_source = serialize_personal_patch(&patch).unwrap();
    let replace_source = serialize_personal_replace(&sample_layout()).unwrap();

    assert!(patch_source.contains("version = 1"));
    assert!(patch_source.contains("mode = \"patch\""));
    assert_eq!(
        parse_personal_layout(path, &patch_source).unwrap(),
        PersonalLayout::Patch(patch)
    );
    assert!(replace_source.contains("version = 1"));
    assert!(replace_source.contains("mode = \"replace\""));
    assert_eq!(
        parse_personal_layout(path, &replace_source).unwrap(),
        PersonalLayout::Replace(sample_layout())
    );
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
fn config_global_default_uses_template_without_creating_personal_file() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("global-default-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut default_state = DefaultLayoutState::load_or_create(&paths);
    let mut template = DefaultLayoutTemplate::builtin();
    template.tabs[0].title = "Global Shell".to_string();
    default_state.save(template).unwrap();

    let opened = open_project_config(&paths, &project_dir, &mut default_state).unwrap();

    assert_eq!(opened.layout.project.name, "global-default-project");
    assert_eq!(opened.layout.tabs[0].title, "Global Shell");
    assert_eq!(
        opened.layout_source,
        LayoutSource::GlobalDefault(paths.default_layout_file())
    );
    assert!(opened.warnings.is_empty());
    assert!(!paths.local_layout_file(&opened.path).exists());
}

#[test]
fn config_global_default_project_config_does_not_read_broken_global_file() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("project-wins");
    fs::create_dir_all(project_dir.join(".yttt")).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut default_state = DefaultLayoutState::load_or_create(&paths);
    fs::write(
        project_dir.join(".yttt/layout.toml"),
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();
    fs::write(paths.default_layout_file(), "[project").unwrap();

    let opened = open_project_config(&paths, &project_dir, &mut default_state).unwrap();

    assert_eq!(opened.layout, sample_layout());
    assert_eq!(
        opened.layout_source,
        LayoutSource::ProjectConfig(paths.project_layout_file(&opened.path))
    );
    assert!(opened.warnings.is_empty());
}

#[test]
fn personal_layout_precedence_applies_patch_to_global_default() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("global-patch");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut default_state = DefaultLayoutState::load_or_create(&paths);
    let canonical_project = project_dir.canonicalize().unwrap();
    let local = paths.local_layout_file(&canonical_project);
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    let patch = LayoutOverride {
        tabs: vec![TabOverride {
            id: "shell".to_string(),
            title: Some("Personal Shell".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    fs::write(&local, serialize_personal_patch(&patch).unwrap()).unwrap();

    let opened = open_project_config(&paths, &project_dir, &mut default_state).unwrap();

    assert_eq!(opened.layout.tabs[0].title, "Personal Shell");
    assert_eq!(
        opened.layout_source,
        LayoutSource::GlobalDefaultWithPersonalPatch {
            global: paths.default_layout_file(),
            local,
        }
    );
}

#[test]
fn personal_layout_precedence_applies_patch_to_project_config() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("project-patch");
    fs::create_dir_all(project_dir.join(".yttt")).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut default_state = DefaultLayoutState::load_or_create(&paths);
    fs::write(
        project_dir.join(".yttt/layout.toml"),
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();
    let canonical_project = project_dir.canonicalize().unwrap();
    let local = paths.local_layout_file(&canonical_project);
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    let patch = LayoutOverride {
        tabs: vec![TabOverride {
            id: "dev".to_string(),
            title: Some("Personal Dev".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    fs::write(&local, serialize_personal_patch(&patch).unwrap()).unwrap();

    let opened = open_project_config(&paths, &project_dir, &mut default_state).unwrap();

    assert_eq!(opened.layout.tab("dev").unwrap().title, "Personal Dev");
    assert_eq!(
        opened.layout_source,
        LayoutSource::ProjectConfigWithPersonalPatch {
            project: paths.project_layout_file(&opened.path),
            local,
        }
    );
}

#[test]
fn personal_layout_precedence_replace_wins_over_global_and_project_bases() {
    for with_project_config in [false, true] {
        let temp = tempdir().unwrap();
        let project_dir = temp.path().join(if with_project_config {
            "replace-project"
        } else {
            "replace-global"
        });
        fs::create_dir_all(&project_dir).unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let mut default_state = DefaultLayoutState::load_or_create(&paths);
        if with_project_config {
            fs::create_dir_all(project_dir.join(".yttt")).unwrap();
            fs::write(
                project_dir.join(".yttt/layout.toml"),
                toml::to_string_pretty(&sample_layout()).unwrap(),
            )
            .unwrap();
        }
        let mut replacement = sample_layout();
        replacement.project.name = "personal replacement".to_string();
        let canonical_project = project_dir.canonicalize().unwrap();
        let local = paths.local_layout_file(&canonical_project);
        fs::create_dir_all(local.parent().unwrap()).unwrap();
        fs::write(&local, serialize_personal_replace(&replacement).unwrap()).unwrap();

        let opened = open_project_config(&paths, &project_dir, &mut default_state).unwrap();

        assert_eq!(opened.layout, replacement);
        assert_eq!(opened.layout_source, LayoutSource::PersonalReplace(local));
        assert!(opened.warnings.is_empty());
    }
}

#[test]
fn personal_layout_warning_preserves_base_and_rejects_legacy_file() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("invalid-personal");
    fs::create_dir_all(project_dir.join(".yttt")).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut default_state = DefaultLayoutState::load_or_create(&paths);
    fs::write(
        project_dir.join(".yttt/layout.toml"),
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();
    let canonical_project = project_dir.canonicalize().unwrap();
    let local = paths.local_layout_file(&canonical_project);
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    fs::write(&local, toml::to_string_pretty(&sample_layout()).unwrap()).unwrap();

    let opened = open_project_config(&paths, &project_dir, &mut default_state).unwrap();

    assert_eq!(opened.layout, sample_layout());
    assert_eq!(
        opened.layout_source,
        LayoutSource::ProjectConfig(paths.project_layout_file(&opened.path))
    );
    assert!(matches!(
        opened.warnings.as_slice(),
        [LayoutLoadWarning::PersonalOverrideValidation { path, message }]
            if path == &local && message.contains("version")
    ));
}

#[test]
fn personal_layout_warning_reports_stale_tab_and_pane_with_path() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("stale-personal");
    fs::create_dir_all(project_dir.join(".yttt")).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut default_state = DefaultLayoutState::load_or_create(&paths);
    fs::write(
        project_dir.join(".yttt/layout.toml"),
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();
    let canonical_project = project_dir.canonicalize().unwrap();
    let local = paths.local_layout_file(&canonical_project);
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    let patch = LayoutOverride {
        tabs: vec![
            TabOverride {
                id: "missing-tab".to_string(),
                title: Some("Missing".to_string()),
                layout: None,
            },
            TabOverride {
                id: "dev".to_string(),
                title: None,
                layout: Some(LayoutNodeOverride::Pane(PaneOverride {
                    id: "missing-pane".to_string(),
                    command: Some("echo missing".to_string()),
                    ..Default::default()
                })),
            },
        ],
        ..Default::default()
    };
    fs::write(&local, serialize_personal_patch(&patch).unwrap()).unwrap();

    let opened = open_project_config(&paths, &project_dir, &mut default_state).unwrap();

    assert_eq!(
        opened.warnings,
        vec![
            LayoutLoadWarning::StaleOverrideTab {
                path: local.clone(),
                tab_id: "missing-tab".to_string(),
            },
            LayoutLoadWarning::StaleOverridePane {
                path: local,
                pane_id: "missing-pane".to_string(),
            },
        ]
    );
}

#[test]
fn reset_local_override_is_idempotent_and_restores_inheritance_on_reopen() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("reset-local");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut default_state = DefaultLayoutState::load_or_create(&paths);
    let canonical_project = project_dir.canonicalize().unwrap();
    save_local_layout(&paths, &canonical_project, &sample_layout()).unwrap();

    reset_local_override(&paths, &canonical_project).unwrap();
    reset_local_override(&paths, &canonical_project).unwrap();
    let opened = open_project_config(&paths, &project_dir, &mut default_state).unwrap();

    assert_eq!(
        opened.layout_source,
        LayoutSource::GlobalDefault(paths.default_layout_file())
    );
    assert!(!paths.local_layout_file(&canonical_project).exists());
}

#[test]
fn config_project_layout_reports_project_config_source() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("project-source");
    let project_config_dir = project_dir.join(".yttt");
    fs::create_dir_all(&project_config_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let project_layout_file = project_config_dir.join("layout.toml");
    fs::write(
        &project_layout_file,
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();

    let opened = open_project_for_test(&paths, &project_dir);

    assert_eq!(
        opened.layout_source,
        LayoutSource::ProjectConfig(paths.project_layout_file(&opened.path))
    );
}

#[test]
fn config_project_layout_merges_app_local_override() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("override-source");
    let project_config_dir = project_dir.join(".yttt");
    fs::create_dir_all(&project_config_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::write(
        project_config_dir.join("layout.toml"),
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();
    let opened_project_dir = project_dir.canonicalize().unwrap();
    let local_layout_file = paths.local_layout_file(&opened_project_dir);
    fs::create_dir_all(local_layout_file.parent().unwrap()).unwrap();
    let override_layout = LayoutOverride {
        tabs: vec![TabOverride {
            id: "dev".to_string(),
            title: Some("Development".to_string()),
            layout: Some(LayoutNodeOverride::Pane(PaneOverride {
                id: "server".to_string(),
                command: Some("pnpm dev".to_string()),
                ..Default::default()
            })),
        }],
        ..Default::default()
    };
    fs::write(
        &local_layout_file,
        serialize_personal_patch(&override_layout).unwrap(),
    )
    .unwrap();

    let opened = open_project_for_test(&paths, &project_dir);

    assert_eq!(opened.layout.tab("dev").unwrap().title, "Development");
    assert_eq!(
        opened
            .layout
            .tab("dev")
            .unwrap()
            .layout
            .find_pane("server")
            .unwrap()
            .command,
        "pnpm dev"
    );
    assert_eq!(
        opened.layout_source,
        LayoutSource::ProjectConfigWithPersonalPatch {
            project: paths.project_layout_file(&opened.path),
            local: local_layout_file,
        }
    );
}

#[test]
fn config_invalid_app_local_override_is_ignored_when_project_config_exists() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("invalid-local-override");
    let project_config_dir = project_dir.join(".yttt");
    fs::create_dir_all(&project_config_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::write(
        project_config_dir.join("layout.toml"),
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();
    let local_layout_file = paths.local_layout_file(&project_dir.canonicalize().unwrap());
    fs::create_dir_all(local_layout_file.parent().unwrap()).unwrap();
    fs::write(&local_layout_file, "[not valid toml").unwrap();

    let opened = open_project_for_test(&paths, &project_dir);

    assert_eq!(opened.layout.project.name, "yttt");
    assert_eq!(
        opened.layout_source,
        LayoutSource::ProjectConfig(paths.project_layout_file(&opened.path))
    );
    assert!(matches!(
        opened.warnings.as_slice(),
        [LayoutLoadWarning::PersonalOverrideParse { path, .. }]
            if path == &local_layout_file
    ));
}

#[test]
fn config_personal_replace_reports_personal_source() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("local-source");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let local_layout_file = save_local_layout(&paths, &project_dir, &sample_layout()).unwrap();

    let opened = open_project_for_test(&paths, &project_dir);

    assert_eq!(
        opened.layout_source,
        LayoutSource::PersonalReplace(local_layout_file)
    );
}

#[test]
fn config_invalid_project_toml_returns_visible_load_error() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("broken-project");
    let project_config_dir = project_dir.join(".yttt");
    fs::create_dir_all(&project_config_dir).unwrap();
    fs::write(project_config_dir.join("layout.toml"), "[project\n").unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let mut default_state = DefaultLayoutState::load_or_create(&paths);
    let err = open_project_config(&paths, &project_dir, &mut default_state).unwrap_err();

    assert!(err.to_string().contains("failed to parse project layout"));
    assert!(err.to_string().contains("layout.toml"));
}

#[test]
fn config_recent_projects_are_stored_in_app_config() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("recent-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    open_project_for_test(&paths, &project_dir);
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
    let saved = parse_personal_layout(&path, &fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(path, paths.local_layout_file(&project_dir));
    assert_eq!(saved, PersonalLayout::Replace(layout));
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

fn open_project_for_test(
    paths: &AppConfigPaths,
    project_dir: &std::path::Path,
) -> yttt::config::layout_loader::ProjectOpenConfig {
    let mut default_state = DefaultLayoutState::load_or_create(paths);
    open_project_config(paths, project_dir, &mut default_state).unwrap()
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
