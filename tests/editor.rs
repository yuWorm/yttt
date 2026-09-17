use std::ops::Range;

use gpui::HighlightStyle;
use gpui_component::{
    highlighter::{HighlightTheme, SyntaxHighlighter},
    input::{DisplayMap, FoldRange, InputEdit, Point, Rope},
};
use yttt::ui::editor::{
    CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, EditorDiagnostic,
    EditorDiagnosticSeverity, EditorLanguageCatalog, EditorLanguageId,
    EditorLanguageResolutionSource, register_builtin_editor_languages,
};

fn assert_token_color(
    styles: &[(Range<usize>, HighlightStyle)],
    source: &str,
    token: &str,
    expected: HighlightStyle,
) {
    let start = source.find(token).expect("token should occur in source");
    let end = start + token.len();
    let color = expected
        .color
        .expect("the default theme should color the expected capture");

    assert!(
        styles.iter().any(|(range, style)| {
            range.start <= start && range.end >= end && style.color == Some(color)
        }),
        "{token:?} should use the expected syntax color"
    );
}

#[test]
fn tsx_highlighter_styles_typescript_and_jsx_tokens() {
    let source = r#"type Props = { title: string };
export function Card({ title }: Props) {
    return <section data-title={title}><Widget label={title} /></section>;
}
"#;
    let theme = HighlightTheme::default_dark();
    let text = Rope::from_str(source);
    let mut highlighter = SyntaxHighlighter::new("tsx");

    assert!(highlighter.update(None, &text, None));
    let styles = highlighter.styles(&(0..source.len()), &theme);

    assert_token_color(&styles, source, "type", theme.style("keyword").unwrap());
    assert_token_color(&styles, source, "Props", theme.style("type").unwrap());
    assert_token_color(&styles, source, "section", theme.style("tag").unwrap());
    assert_token_color(
        &styles,
        source,
        "data-title",
        theme.style("attribute").unwrap(),
    );
}

#[test]
fn jsx_highlighter_styles_dom_tags_and_components() {
    let source = r#"const Card = () => <section data-title="hello"><Widget /></section>;"#;
    let theme = HighlightTheme::default_dark();
    let mut highlighter = SyntaxHighlighter::new("javascript");
    highlighter.update(None, &Rope::from_str(source), None);
    let styles = highlighter.styles(&(0..source.len()), &theme);

    assert_token_color(&styles, source, "section", theme.style("tag").unwrap());
    assert_token_color(&styles, source, "Widget", theme.style("type").unwrap());
    assert_token_color(
        &styles,
        source,
        "data-title",
        theme.style("attribute").unwrap(),
    );
}

#[test]
fn vue_highlighter_styles_template_and_embedded_tokens() {
    let source = r#"<template>
  <Card v-if="ready" :title="message" @click.stop="handleClick">
    {{ message.toUpperCase() }}
  </Card>
</template>
<script lang="tsx">
type Props = { message: string };
const Panel = ({ message }: Props) => <section>{message}</section>;
</script>
<style lang="scss">
@use "palette" as *;
.card { color: $accent; }
</style>
"#;
    let mut theme = HighlightTheme::default_dark();
    // The upstream palette leaves variables uncolored; make the SCSS capture
    // observable independently of the surrounding Vue and TypeScript captures.
    std::sync::Arc::make_mut(&mut theme).style.syntax.variable =
        Some(toml::from_str("color = \"#ff00ff\"").unwrap());
    let text = Rope::from_str(source);
    let mut highlighter = SyntaxHighlighter::new("vue");

    assert!(highlighter.update(None, &text, None));
    let styles = highlighter.styles(&(0..source.len()), &theme);

    assert_token_color(&styles, source, "Card", theme.style("tag").unwrap());
    assert_token_color(&styles, source, "v-if", theme.style("keyword").unwrap());
    assert_token_color(
        &styles,
        source,
        "toUpperCase",
        theme.style("function.method").unwrap(),
    );
    assert_token_color(&styles, source, "type", theme.style("keyword").unwrap());
    assert_token_color(&styles, source, "Props", theme.style("type").unwrap());
    assert_token_color(&styles, source, "@use", theme.style("keyword").unwrap());
    assert_token_color(&styles, source, "$accent", theme.style("variable").unwrap());
}

#[test]
fn vue_directive_expressions_do_not_inherit_html_string_colors() {
    let source =
        r#"<template><div title="tooltip" :title="ready ? 'yes' : 'no'"></div></template>"#;
    let theme = HighlightTheme::default_dark();
    let string_style = theme.style("string").unwrap();
    let mut highlighter = SyntaxHighlighter::new("vue");
    highlighter.update(None, &Rope::from_str(source), None);
    let styles = highlighter.styles(&(0..source.len()), &theme);

    assert_token_color(&styles, source, "tooltip", string_style);
    assert_token_color(&styles, source, "'yes'", string_style);
    let variable = source.find("ready").unwrap();
    assert!(
        styles.iter().all(|(range, style)| {
            !range.contains(&variable) || style.color != string_style.color
        }),
        "a directive expression is code, not an HTML string"
    );
}

#[test]
fn vue_highlighter_refreshes_script_injection_after_an_edit() {
    let source = r#"<script lang="ts">const value = 1;</script>"#;
    let old_content = "const value = 1;";
    let replacement = "type Props = { title: string };";
    let start_byte = source
        .find(old_content)
        .expect("script content should exist");
    let old_end_byte = start_byte + old_content.len();
    let new_end_byte = start_byte + replacement.len();
    let new_source = format!(
        "{}{}{}",
        &source[..start_byte],
        replacement,
        &source[old_end_byte..]
    );
    let theme = HighlightTheme::default_dark();
    let old_text = Rope::from_str(source);
    let new_text = Rope::from_str(&new_source);
    let mut highlighter = SyntaxHighlighter::new("vue");

    assert!(highlighter.update(None, &old_text, None));
    assert!(highlighter.update(
        Some(InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position: Point::new(0, start_byte),
            old_end_position: Point::new(0, old_end_byte),
            new_end_position: Point::new(0, new_end_byte),
        }),
        &new_text,
        None,
    ));
    let styles = highlighter.styles(&(0..new_source.len()), &theme);

    assert_token_color(
        &styles,
        &new_source,
        "type",
        theme.style("keyword").unwrap(),
    );
    assert_token_color(&styles, &new_source, "Props", theme.style("type").unwrap());
}

#[test]
fn added_language_highlighters_style_language_specific_tokens() {
    let theme = HighlightTheme::default_dark();
    let cases = [
        (
            "dockerfile",
            "FROM rust:1.85\nRUN echo $HOME\n",
            "FROM",
            "keyword",
        ),
        (
            "hcl",
            "resource \"example\" \"main\" { count = 1 }\n",
            "resource",
            "keyword",
        ),
        ("nix", "let value = 1; in value\n", "let", "keyword"),
    ];

    for (language, source, token, capture) in cases {
        let text = Rope::from_str(source);
        let mut highlighter = SyntaxHighlighter::new(language);

        assert!(highlighter.update(None, &text, None), "{language}");
        let styles = highlighter.styles(&(0..source.len()), &theme);
        assert_token_color(&styles, source, token, theme.style(capture).unwrap());
    }
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
    assert_eq!(dockerfile.highlighter_name, "dockerfile");

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
        ("App.vue", EditorLanguageId::Vue, "vue"),
        ("document.xml", EditorLanguageId::Xml, "xml"),
        ("styles.css", EditorLanguageId::Css, "css"),
        ("styles.scss", EditorLanguageId::Scss, "scss"),
        (
            "image.containerfile",
            EditorLanguageId::Dockerfile,
            "dockerfile",
        ),
        ("Widget.svelte", EditorLanguageId::Svelte, "svelte"),
        ("view.ejs", EditorLanguageId::Ejs, "ejs"),
        ("view.erb", EditorLanguageId::Erb, "erb"),
        ("schema.graphql", EditorLanguageId::Graphql, "graphql"),
        ("main.tf", EditorLanguageId::Hcl, "hcl"),
        ("flake.nix", EditorLanguageId::Nix, "nix"),
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
fn language_catalog_resolves_windows_development_files() {
    register_builtin_editor_languages();
    let catalog = EditorLanguageCatalog::builtin();
    let cases = [
        ("script.csx", EditorLanguageId::CSharp, "csharp", "csx"),
        (
            "BUILD.PS1",
            EditorLanguageId::Powershell,
            "powershell",
            "ps1",
        ),
        ("Types.ps1xml", EditorLanguageId::Xml, "xml", "ps1xml"),
        (
            "Demo.vcxproj.filters",
            EditorLanguageId::Xml,
            "xml",
            "vcxproj.filters",
        ),
        (
            "Directory.Build.props",
            EditorLanguageId::Xml,
            "xml",
            "props",
        ),
        ("Solution.slnf", EditorLanguageId::Json, "json", "slnf"),
    ];

    for (path, expected_language, expected_highlighter, expected_rule) in cases {
        let resolution = catalog.resolve_for_path(path, None);
        assert_eq!(resolution.language_id, expected_language, "{path}");
        assert_eq!(resolution.highlighter_name, expected_highlighter, "{path}");
        assert_eq!(
            resolution.source,
            EditorLanguageResolutionSource::Extension,
            "{path}"
        );
        assert_eq!(
            resolution.matched_rule.as_deref(),
            Some(expected_rule),
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
