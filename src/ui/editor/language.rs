const GDSCRIPT_HIGHLIGHTS: &str = r#"
(comment) @comment
(string) @string
(integer) @number
(float) @number
(true) @boolean
(false) @boolean
(function_definition name: (name) @function)
(class_definition name: (name) @type)
"#;

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
    registry.register(
        "fish",
        &LanguageConfig::new(
            "fish",
            tree_sitter_fish::language(),
            Vec::new(),
            tree_sitter_fish::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    );
    registry.register(
        "gdscript",
        &LanguageConfig::new(
            "gdscript",
            tree_sitter_gdscript::LANGUAGE.into(),
            Vec::new(),
            GDSCRIPT_HIGHLIGHTS,
            "",
            "",
        ),
    );
}
