use super::EditorLanguageId;
use tree_sitter::{Language, Node, Parser};

/// A named declaration that can appear in the editor breadcrumb bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorSymbol {
    pub name: String,
    pub kind: EditorSymbolKind,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorSymbolKind {
    Module,
    Type,
    Function,
    Constant,
}

impl EditorSymbolKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Type => "type",
            Self::Function => "function",
            Self::Constant => "constant",
        }
    }
}

/// Extracts navigable declarations from supported source text using its Tree-sitter grammar.
///
/// Symbols are flat because their source ranges encode containment. Use
/// [`breadcrumbs_at`] to derive the outer-to-inner path for a cursor line.
pub fn document_symbols(language_id: EditorLanguageId, source: &str) -> Vec<EditorSymbol> {
    let Some(language) = parser_language(language_id) else {
        return Vec::new();
    };

    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let mut symbols = Vec::new();
    collect_symbols(tree.root_node(), source, &mut symbols);
    symbols.sort_unstable_by(|left, right| {
        left.start_line
            .cmp(&right.start_line)
            .then(left.start_column.cmp(&right.start_column))
            .then(right.end_line.cmp(&left.end_line))
    });
    symbols
}

/// Returns the declaration path containing `line`, ordered outermost first.
pub fn breadcrumbs_at(symbols: &[EditorSymbol], line: usize) -> Vec<EditorSymbol> {
    let mut breadcrumbs = symbols
        .iter()
        .filter(|symbol| symbol.start_line <= line && line <= symbol.end_line)
        .cloned()
        .collect::<Vec<_>>();
    breadcrumbs.sort_unstable_by(|left, right| {
        left.start_line
            .cmp(&right.start_line)
            .then(right.end_line.cmp(&left.end_line))
            .then(left.start_column.cmp(&right.start_column))
    });
    breadcrumbs
}

fn collect_symbols(node: Node<'_>, source: &str, symbols: &mut Vec<EditorSymbol>) {
    if symbols.len() == 256 {
        return;
    }

    if let Some(kind) = symbol_kind(node.kind()) {
        if let Some(name) = symbol_name(node, source) {
            let start = node.start_position();
            let end = node.end_position();
            symbols.push(EditorSymbol {
                name,
                kind,
                start_line: start.row,
                start_column: start.column,
                end_line: end.row,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(child, source, symbols);
    }
}

fn symbol_kind(node_kind: &str) -> Option<EditorSymbolKind> {
    match node_kind {
        "module"
        | "module_definition"
        | "namespace_definition"
        | "namespace_declaration"
        | "mod_item" => Some(EditorSymbolKind::Module),
        "class"
        | "class_definition"
        | "class_declaration"
        | "class_specifier"
        | "struct_definition"
        | "struct_declaration"
        | "struct_specifier"
        | "struct_item"
        | "union_specifier"
        | "union_item"
        | "enum_definition"
        | "enum_declaration"
        | "enum_specifier"
        | "enum_item"
        | "trait_declaration"
        | "trait_item"
        | "interface_declaration"
        | "protocol_declaration"
        | "object_declaration"
        | "type_declaration"
        | "type_alias_declaration"
        | "type_item" => Some(EditorSymbolKind::Type),
        "function_definition"
        | "function_declaration"
        | "function_item"
        | "method"
        | "method_declaration"
        | "method_definition"
        | "constructor_definition"
        | "constructor_declaration"
        | "init_declaration" => Some(EditorSymbolKind::Function),
        "const_item" | "const_declaration" | "constant_declaration" => {
            Some(EditorSymbolKind::Constant)
        }
        _ => None,
    }
}

fn symbol_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .or_else(|| first_identifier_child(node))
        .and_then(|name| name.utf8_text(source.as_bytes()).ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn first_identifier_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier"
                | "type_identifier"
                | "field_identifier"
                | "property_identifier"
                | "scoped_identifier"
                | "simple_identifier"
        ) {
            return Some(child);
        }
        if !matches!(
            child.kind(),
            "body" | "block" | "compound_statement" | "class_body" | "declaration_list"
        ) && let Some(identifier) = first_identifier_child(child)
        {
            return Some(identifier);
        }
    }
    None
}

fn parser_language(language_id: EditorLanguageId) -> Option<Language> {
    Some(match language_id {
        EditorLanguageId::PlainText
        | EditorLanguageId::Toml
        | EditorLanguageId::Json
        | EditorLanguageId::Jsonc
        | EditorLanguageId::Yaml
        | EditorLanguageId::Markdown
        | EditorLanguageId::Html
        | EditorLanguageId::Vue
        | EditorLanguageId::Xml
        | EditorLanguageId::Css
        | EditorLanguageId::Scss
        | EditorLanguageId::Astro
        | EditorLanguageId::Svelte
        | EditorLanguageId::Ejs
        | EditorLanguageId::Erb
        | EditorLanguageId::Graphql
        | EditorLanguageId::Sql
        | EditorLanguageId::Proto
        | EditorLanguageId::Diff
        | EditorLanguageId::CMake
        | EditorLanguageId::Powershell
        | EditorLanguageId::Make
        | EditorLanguageId::Dockerfile => return None,
        EditorLanguageId::Bash => tree_sitter_bash::LANGUAGE.into(),
        EditorLanguageId::Fish => tree_sitter_fish::language(),
        EditorLanguageId::Gdscript => tree_sitter_gdscript::LANGUAGE.into(),
        EditorLanguageId::Rust => tree_sitter_rust::LANGUAGE.into(),
        EditorLanguageId::Go => tree_sitter_go::LANGUAGE.into(),
        EditorLanguageId::Python => tree_sitter_python::LANGUAGE.into(),
        EditorLanguageId::C => tree_sitter_c::LANGUAGE.into(),
        EditorLanguageId::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        EditorLanguageId::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        EditorLanguageId::Java => tree_sitter_java::LANGUAGE.into(),
        EditorLanguageId::Kotlin => tree_sitter_kotlin_sg::LANGUAGE.into(),
        EditorLanguageId::Scala => tree_sitter_scala::LANGUAGE.into(),
        EditorLanguageId::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        EditorLanguageId::Php => tree_sitter_php::LANGUAGE_PHP.into(),
        EditorLanguageId::Lua => tree_sitter_lua::LANGUAGE.into(),
        EditorLanguageId::Swift => tree_sitter_swift::LANGUAGE.into(),
        EditorLanguageId::Zig => tree_sitter_zig::LANGUAGE.into(),
        EditorLanguageId::Javascript => tree_sitter_javascript::LANGUAGE.into(),
        EditorLanguageId::Typescript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        EditorLanguageId::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::{breadcrumbs_at, document_symbols};
    use crate::ui::editor::EditorLanguageId;

    #[test]
    fn extracts_navigable_symbols_for_supported_languages() {
        let fixtures = [
            (EditorLanguageId::Rust, "struct Root {}", "Root"),
            (EditorLanguageId::Go, "func render() {}", "render"),
            (
                EditorLanguageId::Python,
                "class Root:\n    def render(self):\n        pass",
                "Root",
            ),
            (
                EditorLanguageId::C,
                "int render(void) { return 0; }",
                "render",
            ),
            (
                EditorLanguageId::Cpp,
                "class Root { void render() {} };",
                "Root",
            ),
            (
                EditorLanguageId::CSharp,
                "class Root { void Render() {} }",
                "Root",
            ),
            (
                EditorLanguageId::Java,
                "class Root { void render() {} }",
                "Root",
            ),
            (
                EditorLanguageId::Kotlin,
                "class Root { fun render() {} }",
                "Root",
            ),
            (
                EditorLanguageId::Scala,
                "class Root { def render = 1 }",
                "Root",
            ),
            (
                EditorLanguageId::Ruby,
                "class Root\n  def render; end\nend",
                "Root",
            ),
            (
                EditorLanguageId::Php,
                "<?php function render() {}",
                "render",
            ),
            (EditorLanguageId::Lua, "function render() end", "render"),
            (
                EditorLanguageId::Swift,
                "struct Root { func render() {} }",
                "Root",
            ),
            (EditorLanguageId::Zig, "fn render() void {}", "render"),
            (EditorLanguageId::Bash, "render() { :; }", "render"),
            (EditorLanguageId::Fish, "function render\nend", "render"),
            (
                EditorLanguageId::Gdscript,
                "func render():\n    pass",
                "render",
            ),
            (
                EditorLanguageId::Javascript,
                "function render() {}",
                "render",
            ),
            (
                EditorLanguageId::Typescript,
                "function render(): void {}",
                "render",
            ),
            (
                EditorLanguageId::Tsx,
                "function render(): JSX.Element { return <div />; }",
                "render",
            ),
        ];

        for (language, source, expected_name) in fixtures {
            let symbols = document_symbols(language, source);
            assert!(
                symbols.iter().any(|symbol| symbol.name == expected_name),
                "{language:?} did not expose {expected_name:?}: {symbols:?}"
            );
        }
    }

    #[test]
    fn returns_outer_to_inner_breadcrumbs() {
        let symbols = document_symbols(EditorLanguageId::Rust, "mod root {\n    fn render() {}\n}");

        assert_eq!(
            breadcrumbs_at(&symbols, 1)
                .into_iter()
                .map(|symbol| symbol.name)
                .collect::<Vec<_>>(),
            vec!["root", "render"]
        );
    }
}
