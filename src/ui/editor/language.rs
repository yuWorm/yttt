const CSHARP_HIGHLIGHTS: &str = include_str!("queries/csharp.scm");

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

use std::sync::Once;

use gpui::SharedString;
use gpui_component::highlighter::{LanguageConfig, LanguageRegistry};

const DOCKERFILE_HIGHLIGHTS: &str = include_str!("queries/dockerfile.scm");
const HCL_HIGHLIGHTS: &str = include_str!("queries/hcl.scm");
const JSX_HIGHLIGHTS: &str = include_str!("queries/jsx.scm");
const VUE_HIGHLIGHTS: &str = include_str!("queries/vue.scm");
const VUE_INJECTIONS: &str = include_str!("queries/vue-injections.scm");

pub fn register_builtin_editor_languages() {
    // Registration extends existing grammars, so it must run exactly once, and
    // concurrent callers must wait for it to finish.
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(register_languages);
}

fn register_languages() {
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
        "csharp",
        &LanguageConfig::new(
            "csharp",
            tree_sitter_c_sharp::LANGUAGE.into(),
            Vec::new(),
            CSHARP_HIGHLIGHTS,
            "",
            "",
        ),
    );
    registry.register(
        "powershell",
        &LanguageConfig::new(
            "powershell",
            tree_sitter_powershell::LANGUAGE.into(),
            Vec::new(),
            tree_sitter_powershell::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    );
    registry.register(
        "xml",
        &LanguageConfig::new(
            "xml",
            tree_sitter_xml::LANGUAGE_XML.into(),
            Vec::new(),
            tree_sitter_xml::XML_HIGHLIGHT_QUERY,
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
    registry.register(
        "dockerfile",
        &LanguageConfig::new(
            "dockerfile",
            tree_sitter_containerfile::LANGUAGE.into(),
            Vec::new(),
            DOCKERFILE_HIGHLIGHTS,
            "",
            "",
        ),
    );
    registry.register(
        "hcl",
        &LanguageConfig::new(
            "hcl",
            tree_sitter_hcl::LANGUAGE.into(),
            Vec::new(),
            HCL_HIGHLIGHTS,
            "",
            "",
        ),
    );
    registry.register(
        "nix",
        &LanguageConfig::new(
            "nix",
            tree_sitter_nix::LANGUAGE.into(),
            Vec::new(),
            tree_sitter_nix::HIGHLIGHTS_QUERY,
            tree_sitter_nix::INJECTIONS_QUERY,
            "",
        ),
    );
    registry.register(
        "scss",
        &LanguageConfig::new(
            "scss",
            arborium_scss::language().into(),
            Vec::new(),
            arborium_scss::HIGHLIGHTS_QUERY.as_str(),
            "",
            "",
        ),
    );
    registry.register(
        "vue",
        &LanguageConfig::new(
            "vue",
            tree_sitter_vue_next::LANGUAGE.into(),
            ["javascript", "typescript", "tsx", "css", "scss"]
                .map(SharedString::new_static)
                .to_vec(),
            VUE_HIGHLIGHTS,
            VUE_INJECTIONS,
            "",
        ),
    );
    for language in ["javascript", "tsx"] {
        append_jsx_highlights(registry, language);
    }
}

fn append_jsx_highlights(registry: &LanguageRegistry, language: &str) {
    let Some(mut config) = registry.language(language) else {
        return;
    };
    config.highlights = format!("{}\n{JSX_HIGHLIGHTS}", config.highlights).into();
    registry.register(language, &config);
}
