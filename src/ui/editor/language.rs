use gpui_component::highlighter::{LanguageConfig, LanguageRegistry};

pub fn register_builtin_editor_languages() {
    let registry = LanguageRegistry::singleton();
    registry.register(
        "toml",
        &LanguageConfig::new(
            "toml",
            tree_sitter_toml_ng::LANGUAGE.into(),
            Vec::new(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    );
}
