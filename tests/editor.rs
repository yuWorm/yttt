use gpui_component::{
    highlighter::LanguageRegistry,
    input::{DisplayMap, FoldRange, Rope},
};
use yttt::ui::editor::{
    CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, EditorDiagnostic,
    EditorDiagnosticSeverity, EditorLanguageCatalog, EditorLanguageId,
    EditorLanguageResolutionSource, register_builtin_editor_languages,
};

#[test]
fn toml_language_registration_registers_toml_highlighter() {
    register_builtin_editor_languages();

    assert!(LanguageRegistry::singleton().language("toml").is_some());
}

#[test]
fn extended_language_registration_adds_fish_and_gdscript_highlighters() {
    register_builtin_editor_languages();

    assert!(LanguageRegistry::singleton().language("fish").is_some());
    assert!(LanguageRegistry::singleton().language("gdscript").is_some());
}

#[gpui::test]
fn code_editor_fold_projection_hides_folded_lines(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let text = Rope::from_str("fn render() {\n    if enabled {\n        draw();\n    }\n}\n");
        let mut display_map = DisplayMap::new(gpui::Font::default(), gpui::px(14.0), None);
        display_map.set_text(&text, cx);
        display_map.set_fold_candidates(vec![FoldRange::new(0, 4)]);
        display_map.set_folded(0, true);

        assert!(display_map.is_folded_at(0));
        assert!(!display_map.is_buffer_line_hidden(0));
        assert!(display_map.is_buffer_line_hidden(1));
        assert!(display_map.is_buffer_line_hidden(2));
        assert!(display_map.is_buffer_line_hidden(3));
        assert!(!display_map.is_buffer_line_hidden(4));
    });
}
#[test]
fn code_editor_state_tracks_value_dirty_and_errors() {
    let mut state = CodeEditorState::new(
        "/tmp/layout.toml",
        CodeEditorConfig::new("Edit layout TOML", "toml"),
        "name = \"yttt\"",
    );

    assert_eq!(state.path().to_string_lossy(), "/tmp/layout.toml");
    assert_eq!(state.language(), "toml");
    assert_eq!(state.value(), "name = \"yttt\"");
    assert!(!state.is_dirty());
    assert_eq!(state.error(), None);

    state.set_error("parse failed");
    assert_eq!(state.error(), Some("parse failed"));

    state.set_value("name = \"changed\"");
    assert_eq!(state.value(), "name = \"changed\"");
    assert!(state.is_dirty());
    assert_eq!(state.error(), None);

    state.mark_saved();
    assert!(!state.is_dirty());
}

#[test]
fn code_editor_config_exposes_configured_tab_size() {
    let config =
        CodeEditorConfig::new("Project file", CodeEditorLanguageMode::Auto).with_tab_size(8);

    assert_eq!(config.tab_size(), 8);
}

#[test]
fn marking_captured_value_saved_keeps_newer_edits_dirty() {
    let mut editor = CodeEditorState::new(
        "/tmp/project.rs",
        CodeEditorConfig::new("Project file", CodeEditorLanguageMode::Auto),
        "old",
    );
    editor.set_value("first edit");
    let captured = editor.value().to_string();
    editor.set_value("newer edit");

    editor.mark_value_saved(captured);

    assert!(editor.is_dirty());
    assert_eq!(editor.value(), "newer edit");
    assert_eq!(editor.saved_value(), "first edit");
}

#[test]
fn replacing_from_disk_resets_value_baseline_and_stale_feedback() {
    let mut editor = CodeEditorState::new(
        "/tmp/project.rs",
        CodeEditorConfig::new("Project file", CodeEditorLanguageMode::Auto),
        "old",
    );
    editor.set_value("dirty");
    editor.set_error("save failed");
    editor.set_diagnostics(vec![EditorDiagnostic::new(
        EditorDiagnosticSeverity::Warning,
        "disk",
        "changed externally",
    )]);

    editor.replace_from_disk("fresh");

    assert_eq!(editor.value(), "fresh");
    assert_eq!(editor.saved_value(), "fresh");
    assert!(!editor.is_dirty());
    assert_eq!(editor.error(), None);
    assert!(editor.diagnostics().is_empty());
}

#[test]
fn code_editor_state_tracks_and_clears_diagnostics() {
    let mut state = CodeEditorState::new(
        "/tmp/layout.toml",
        CodeEditorConfig::new("Edit layout TOML", "toml"),
        "name = \"yttt\"",
    );

    state.set_diagnostics(vec![EditorDiagnostic::new(
        EditorDiagnosticSeverity::Error,
        "toml",
        "failed to parse TOML",
    )]);

    assert_eq!(state.diagnostics().len(), 1);
    assert_eq!(
        state.diagnostics()[0].severity,
        EditorDiagnosticSeverity::Error
    );
    assert_eq!(state.diagnostics()[0].source, "toml");

    state.set_value("name = \"fixed\"");

    assert!(state.diagnostics().is_empty());
}

#[test]
fn language_catalog_resolves_builtin_languages_from_path_and_content() {
    register_builtin_editor_languages();
    let catalog = EditorLanguageCatalog::builtin();

    let toml = catalog.resolve_for_path("layout.toml", None);
    assert_eq!(toml.language_id, EditorLanguageId::Toml);
    assert_eq!(toml.highlighter_name, "toml");
    assert_eq!(toml.source, EditorLanguageResolutionSource::Filename);

    let cargo = catalog.resolve_for_path("Cargo.toml", None);
    assert_eq!(cargo.language_id, EditorLanguageId::Toml);
    assert_eq!(cargo.source, EditorLanguageResolutionSource::Filename);

    let json = catalog.resolve_for_path("package.json", None);
    assert_eq!(json.language_id, EditorLanguageId::Json);
    assert_eq!(json.highlighter_name, "json");

    let yaml = catalog.resolve_for_path("config.yml", None);
    assert_eq!(yaml.language_id, EditorLanguageId::Yaml);
    assert_eq!(yaml.source, EditorLanguageResolutionSource::Extension);

    let rust = catalog.resolve_for_path("src/main.rs", None);
    assert_eq!(rust.language_id, EditorLanguageId::Rust);

    let typescript = catalog.resolve_for_path("types/index.d.ts", None);
    assert_eq!(typescript.language_id, EditorLanguageId::Typescript);
    assert_eq!(typescript.matched_rule.as_deref(), Some("d.ts"));

    let makefile = catalog.resolve_for_path("Makefile", None);
    assert_eq!(makefile.language_id, EditorLanguageId::Make);
    assert_eq!(makefile.highlighter_name, "make");

    let dockerfile = catalog.resolve_for_path("Dockerfile", None);
    assert_eq!(dockerfile.language_id, EditorLanguageId::Dockerfile);
    assert_eq!(dockerfile.highlighter_name, "text");

    let shebang = catalog.resolve_for_path("run", Some("#!/usr/bin/env bash\npwd\n"));
    assert_eq!(shebang.language_id, EditorLanguageId::Bash);
    assert_eq!(shebang.source, EditorLanguageResolutionSource::FirstLine);

    let unknown = catalog.resolve_for_path("README.unknown", None);
    assert_eq!(unknown.language_id, EditorLanguageId::PlainText);
    assert_eq!(unknown.highlighter_name, "text");
    assert_eq!(unknown.source, EditorLanguageResolutionSource::Fallback);
}

#[test]
fn language_catalog_resolves_expanded_editor_languages() {
    register_builtin_editor_languages();
    let catalog = EditorLanguageCatalog::builtin();
    let cases = [
        ("main.go", EditorLanguageId::Go, "go"),
        ("main.py", EditorLanguageId::Python, "python"),
        ("main.c", EditorLanguageId::C, "c"),
        ("main.cpp", EditorLanguageId::Cpp, "cpp"),
        ("main.cs", EditorLanguageId::CSharp, "csharp"),
        ("Main.java", EditorLanguageId::Java, "java"),
        ("main.kt", EditorLanguageId::Kotlin, "kotlin"),
        ("main.scala", EditorLanguageId::Scala, "scala"),
        ("main.rb", EditorLanguageId::Ruby, "ruby"),
        ("main.php", EditorLanguageId::Php, "php"),
        ("main.lua", EditorLanguageId::Lua, "lua"),
        ("main.swift", EditorLanguageId::Swift, "swift"),
        ("main.zig", EditorLanguageId::Zig, "zig"),
        ("config.fish", EditorLanguageId::Fish, "fish"),
        ("player.gd", EditorLanguageId::Gdscript, "gdscript"),
        ("index.html", EditorLanguageId::Html, "html"),
        ("App.vue", EditorLanguageId::Vue, "html"),
        ("document.xml", EditorLanguageId::Xml, "html"),
        ("styles.css", EditorLanguageId::Css, "css"),
        ("styles.scss", EditorLanguageId::Scss, "css"),
        ("page.astro", EditorLanguageId::Astro, "astro"),
        ("Widget.svelte", EditorLanguageId::Svelte, "svelte"),
        ("view.ejs", EditorLanguageId::Ejs, "ejs"),
        ("view.erb", EditorLanguageId::Erb, "erb"),
        ("schema.graphql", EditorLanguageId::Graphql, "graphql"),
        ("query.sql", EditorLanguageId::Sql, "sql"),
        ("messages.proto", EditorLanguageId::Proto, "proto"),
        ("change.patch", EditorLanguageId::Diff, "diff"),
        ("module.cmake", EditorLanguageId::CMake, "cmake"),
    ];

    for (path, expected_language, expected_highlighter) in cases {
        let resolution = catalog.resolve_for_path(path, None);
        assert_eq!(resolution.language_id, expected_language, "{path}");
        assert_eq!(resolution.highlighter_name, expected_highlighter, "{path}");
        assert_eq!(
            resolution.source,
            EditorLanguageResolutionSource::Extension,
            "{path}"
        );
    }
}

#[test]
fn language_catalog_falls_back_to_text_for_missing_highlighter() {
    register_builtin_editor_languages();
    let mut catalog = EditorLanguageCatalog::builtin();
    catalog.set_highlighter_for_test(EditorLanguageId::Toml, "not-registered");

    let resolution = catalog.resolve_for_path("layout.toml", None);

    assert_eq!(resolution.language_id, EditorLanguageId::Toml);
    assert_eq!(resolution.highlighter_name, "text");
}

#[test]
fn code_editor_state_resolves_language_from_path_with_catalog() {
    register_builtin_editor_languages();
    let catalog = EditorLanguageCatalog::builtin();

    let state = CodeEditorState::new_with_catalog(
        "/tmp/settings.toml",
        CodeEditorConfig::new("Settings", CodeEditorLanguageMode::Auto),
        "theme = \"one-dark-theme\"",
        &catalog,
    );

    assert_eq!(state.language_id(), EditorLanguageId::Toml);
    assert_eq!(state.language(), "toml");
    assert_eq!(
        state.resolved_language().source,
        EditorLanguageResolutionSource::Filename
    );
}

#[test]
fn code_editor_state_explicit_language_wins_over_path_detection() {
    register_builtin_editor_languages();
    let catalog = EditorLanguageCatalog::builtin();

    let state = CodeEditorState::new_with_catalog(
        "/tmp/settings.toml",
        CodeEditorConfig::new(
            "Settings",
            CodeEditorLanguageMode::Explicit(EditorLanguageId::Json),
        ),
        "{}",
        &catalog,
    );

    assert_eq!(state.language_id(), EditorLanguageId::Json);
    assert_eq!(state.language(), "json");
    assert_eq!(
        state.resolved_language().source,
        EditorLanguageResolutionSource::Explicit
    );
}
