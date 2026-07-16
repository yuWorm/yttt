use std::path::Path;

use gpui_component::highlighter::LanguageRegistry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EditorLanguageId {
    PlainText,
    Toml,
    Json,
    Jsonc,
    Yaml,
    Markdown,
    Html,
    Vue,
    Xml,
    Css,
    Scss,
    Astro,
    Svelte,
    Ejs,
    Erb,
    Graphql,
    Sql,
    Proto,
    Diff,
    CMake,
    Bash,
    Powershell,
    Fish,
    Gdscript,
    Rust,
    Go,
    Python,
    C,
    Cpp,
    CSharp,
    Java,
    Kotlin,
    Scala,
    Ruby,
    Php,
    Lua,
    Swift,
    Zig,
    Javascript,
    Typescript,
    Tsx,
    Make,
    Dockerfile,
}

impl EditorLanguageId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlainText => "plain_text",
            Self::Toml => "toml",
            Self::Json => "json",
            Self::Jsonc => "jsonc",
            Self::Yaml => "yaml",
            Self::Markdown => "markdown",
            Self::Html => "html",
            Self::Vue => "vue",
            Self::Xml => "xml",
            Self::Css => "css",
            Self::Scss => "scss",
            Self::Astro => "astro",
            Self::Svelte => "svelte",
            Self::Ejs => "ejs",
            Self::Erb => "erb",
            Self::Graphql => "graphql",
            Self::Sql => "sql",
            Self::Proto => "proto",
            Self::Diff => "diff",
            Self::CMake => "cmake",
            Self::Bash => "bash",
            Self::Powershell => "powershell",
            Self::Fish => "fish",
            Self::Gdscript => "gdscript",
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Python => "python",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "csharp",
            Self::Java => "java",
            Self::Kotlin => "kotlin",
            Self::Scala => "scala",
            Self::Ruby => "ruby",
            Self::Php => "php",
            Self::Lua => "lua",
            Self::Swift => "swift",
            Self::Zig => "zig",
            Self::Javascript => "javascript",
            Self::Typescript => "typescript",
            Self::Tsx => "tsx",
            Self::Make => "make",
            Self::Dockerfile => "dockerfile",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        builtin_language_definitions()
            .into_iter()
            .find(|definition| {
                definition.id.as_str() == value
                    || definition.aliases.iter().any(|alias| *alias == value)
            })
            .map(|definition| definition.id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorLanguageDefinition {
    pub id: EditorLanguageId,
    pub display_name: &'static str,
    pub aliases: Vec<&'static str>,
    pub extensions: Vec<&'static str>,
    pub filenames: Vec<&'static str>,
    pub first_line_patterns: Vec<&'static str>,
    pub highlighter_name: String,
    pub enabled_by_default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorLanguageResolutionSource {
    Explicit,
    UserOverride,
    Filename,
    Extension,
    FirstLine,
    Fallback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorLanguageResolution {
    pub language_id: EditorLanguageId,
    pub highlighter_name: String,
    pub source: EditorLanguageResolutionSource,
    pub matched_rule: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorLanguageCatalog {
    definitions: Vec<EditorLanguageDefinition>,
}

impl EditorLanguageCatalog {
    pub fn builtin() -> Self {
        Self {
            definitions: builtin_language_definitions(),
        }
    }

    pub fn all_languages(&self) -> &[EditorLanguageDefinition] {
        &self.definitions
    }

    pub fn language(&self, id: EditorLanguageId) -> Option<&EditorLanguageDefinition> {
        self.definitions.iter().find(|language| language.id == id)
    }

    pub fn resolve_for_path(
        &self,
        path: impl AsRef<Path>,
        content: Option<&str>,
    ) -> EditorLanguageResolution {
        let path = path.as_ref();
        if let Some(resolution) = self.resolve_by_filename(path) {
            return resolution;
        }
        if let Some(resolution) = self.resolve_by_extension(path) {
            return resolution;
        }
        if let Some(resolution) = self.resolve_by_first_line(content) {
            return resolution;
        }
        self.fallback_resolution()
    }

    pub fn resolve_for_name_or_extension(&self, value: &str) -> Option<EditorLanguageResolution> {
        let value = value.trim().trim_start_matches('.');
        if value.is_empty() {
            return None;
        }

        self.definitions
            .iter()
            .find(|definition| {
                definition.id.as_str() == value
                    || definition.aliases.iter().any(|alias| *alias == value)
                    || definition
                        .extensions
                        .iter()
                        .any(|extension| *extension == value)
            })
            .map(|definition| {
                self.resolution_for(
                    definition,
                    EditorLanguageResolutionSource::Explicit,
                    Some(value.to_string()),
                )
            })
    }

    pub fn resolve_explicit(&self, id: EditorLanguageId) -> EditorLanguageResolution {
        let definition = self
            .language(id)
            .or_else(|| self.language(EditorLanguageId::PlainText))
            .expect("builtin catalog must include plain text");
        self.resolution_for(
            definition,
            EditorLanguageResolutionSource::Explicit,
            Some(id.as_str().to_string()),
        )
    }

    #[doc(hidden)]
    pub fn set_highlighter_for_test(&mut self, id: EditorLanguageId, highlighter_name: &str) {
        if let Some(definition) = self
            .definitions
            .iter_mut()
            .find(|definition| definition.id == id)
        {
            definition.highlighter_name = highlighter_name.to_string();
        }
    }

    fn resolve_by_filename(&self, path: &Path) -> Option<EditorLanguageResolution> {
        let filename = path.file_name()?.to_str()?;
        self.definitions
            .iter()
            .find(|definition| {
                definition
                    .filenames
                    .iter()
                    .any(|candidate| filename.eq_ignore_ascii_case(candidate))
            })
            .map(|definition| {
                self.resolution_for(
                    definition,
                    EditorLanguageResolutionSource::Filename,
                    Some(filename.to_string()),
                )
            })
    }

    fn resolve_by_extension(&self, path: &Path) -> Option<EditorLanguageResolution> {
        let filename = path.file_name()?.to_str()?;
        self.definitions
            .iter()
            .flat_map(|definition| {
                definition.extensions.iter().filter_map(move |extension| {
                    if matches_extension(filename, extension) {
                        Some((definition, *extension))
                    } else {
                        None
                    }
                })
            })
            .max_by_key(|(_, extension)| extension.len())
            .map(|(definition, extension)| {
                self.resolution_for(
                    definition,
                    EditorLanguageResolutionSource::Extension,
                    Some(extension.to_string()),
                )
            })
    }

    fn resolve_by_first_line(&self, content: Option<&str>) -> Option<EditorLanguageResolution> {
        let first_line = content?.lines().next()?.trim();
        self.definitions
            .iter()
            .flat_map(|definition| {
                definition
                    .first_line_patterns
                    .iter()
                    .filter_map(move |pattern| {
                        first_line
                            .contains(pattern)
                            .then_some((definition, *pattern))
                    })
            })
            .next()
            .map(|(definition, pattern)| {
                self.resolution_for(
                    definition,
                    EditorLanguageResolutionSource::FirstLine,
                    Some(pattern.to_string()),
                )
            })
    }

    fn fallback_resolution(&self) -> EditorLanguageResolution {
        let definition = self
            .language(EditorLanguageId::PlainText)
            .expect("builtin catalog must include plain text");
        self.resolution_for(
            definition,
            EditorLanguageResolutionSource::Fallback,
            Some("plain_text".to_string()),
        )
    }

    fn resolution_for(
        &self,
        definition: &EditorLanguageDefinition,
        source: EditorLanguageResolutionSource,
        matched_rule: Option<String>,
    ) -> EditorLanguageResolution {
        EditorLanguageResolution {
            language_id: definition.id,
            highlighter_name: valid_highlighter_or_text(&definition.highlighter_name),
            source,
            matched_rule,
        }
    }
}

fn matches_extension(filename: &str, extension: &str) -> bool {
    let start = filename.len().saturating_sub(extension.len());
    let Some(suffix) = filename.get(start..) else {
        return false;
    };

    suffix.eq_ignore_ascii_case(extension)
        && (start == 0 || filename.as_bytes().get(start - 1) == Some(&b'.'))
}

fn valid_highlighter_or_text(highlighter_name: &str) -> String {
    let registry = LanguageRegistry::singleton();
    if registry
        .languages()
        .iter()
        .any(|language| language.as_ref() == highlighter_name)
    {
        highlighter_name.to_string()
    } else {
        "text".to_string()
    }
}

fn builtin_language_definitions() -> Vec<EditorLanguageDefinition> {
    vec![
        language(
            EditorLanguageId::PlainText,
            "Plain Text",
            &["text", "txt"],
            &["txt"],
            &[],
            &[],
            "text",
        ),
        language(
            EditorLanguageId::Toml,
            "TOML",
            &[],
            &["toml"],
            &[
                "Cargo.toml",
                "layout.toml",
                "settings.toml",
                "keybindings.toml",
            ],
            &[],
            "toml",
        ),
        language(
            EditorLanguageId::Json,
            "JSON",
            &[],
            &["json", "slnf"],
            &["package.json", "tsconfig.json"],
            &[],
            "json",
        ),
        language(
            EditorLanguageId::Jsonc,
            "JSONC",
            &[],
            &["jsonc"],
            &[],
            &[],
            "json",
        ),
        language(
            EditorLanguageId::Yaml,
            "YAML",
            &["yml"],
            &["yaml", "yml"],
            &[],
            &[],
            "yaml",
        ),
        language(
            EditorLanguageId::Markdown,
            "Markdown",
            &["md"],
            &["md", "markdown", "mdx"],
            &[],
            &[],
            "markdown",
        ),
        language(
            EditorLanguageId::Html,
            "HTML",
            &["htm"],
            &["html", "htm", "xhtml"],
            &[],
            &[],
            "html",
        ),
        language(
            EditorLanguageId::Vue,
            "Vue",
            &[],
            &["vue"],
            &[],
            &[],
            "html",
        ),
        language(
            EditorLanguageId::Xml,
            "XML",
            &["xaml", "msbuild"],
            &[
                "xml",
                "xsd",
                "xsl",
                "xslt",
                "svg",
                "xaml",
                "axaml",
                "csproj",
                "fsproj",
                "vbproj",
                "vcxproj",
                "vcxproj.filters",
                "props",
                "targets",
                "resx",
                "resw",
                "config",
                "nuspec",
                "manifest",
                "appxmanifest",
                "application",
                "wixproj",
                "wxs",
                "wxi",
                "wxl",
                "natvis",
                "ruleset",
                "ps1xml",
                "psc1",
                "cdxml",
                "slnx",
                "pubxml",
                "runsettings",
                "testsettings",
                "proj",
                "projitems",
            ],
            &[],
            &[],
            "xml",
        ),
        language(EditorLanguageId::Css, "CSS", &[], &["css"], &[], &[], "css"),
        language(
            EditorLanguageId::Scss,
            "SCSS",
            &[],
            &["scss"],
            &[],
            &[],
            "css",
        ),
        language(
            EditorLanguageId::Astro,
            "Astro",
            &[],
            &["astro"],
            &[],
            &[],
            "astro",
        ),
        language(
            EditorLanguageId::Svelte,
            "Svelte",
            &[],
            &["svelte"],
            &[],
            &[],
            "svelte",
        ),
        language(EditorLanguageId::Ejs, "EJS", &[], &["ejs"], &[], &[], "ejs"),
        language(EditorLanguageId::Erb, "ERB", &[], &["erb"], &[], &[], "erb"),
        language(
            EditorLanguageId::Graphql,
            "GraphQL",
            &["gql"],
            &["graphql", "gql"],
            &[],
            &[],
            "graphql",
        ),
        language(EditorLanguageId::Sql, "SQL", &[], &["sql"], &[], &[], "sql"),
        language(
            EditorLanguageId::Proto,
            "Protocol Buffers",
            &["protobuf"],
            &["proto"],
            &[],
            &[],
            "proto",
        ),
        language(
            EditorLanguageId::Diff,
            "Diff",
            &["patch"],
            &["diff", "patch"],
            &[],
            &[],
            "diff",
        ),
        language(
            EditorLanguageId::CMake,
            "CMake",
            &[],
            &["cmake"],
            &["CMakeLists.txt"],
            &[],
            "cmake",
        ),
        language(
            EditorLanguageId::Bash,
            "Bash",
            &["sh", "shell"],
            &["sh", "bash"],
            &[".bashrc", ".zshrc"],
            &["bash", "/bin/sh"],
            "bash",
        ),
        language(
            EditorLanguageId::Powershell,
            "PowerShell",
            &["ps", "pwsh"],
            &["ps1", "psm1", "psd1", "pssc"],
            &[],
            &["pwsh", "powershell"],
            "powershell",
        ),
        language(
            EditorLanguageId::Fish,
            "Fish",
            &["fish"],
            &["fish"],
            &[],
            &["fish"],
            "fish",
        ),
        language(
            EditorLanguageId::Gdscript,
            "GDScript",
            &["gd"],
            &["gd"],
            &[],
            &[],
            "gdscript",
        ),
        language(EditorLanguageId::Go, "Go", &[], &["go"], &[], &[], "go"),
        language(
            EditorLanguageId::Python,
            "Python",
            &["py"],
            &["py", "pyi", "pyw"],
            &[],
            &["python"],
            "python",
        ),
        language(EditorLanguageId::C, "C", &[], &["c", "h"], &[], &[], "c"),
        language(
            EditorLanguageId::Cpp,
            "C++",
            &["c++", "cc"],
            &["cc", "cpp", "cxx", "hpp", "hxx"],
            &[],
            &[],
            "cpp",
        ),
        language(
            EditorLanguageId::CSharp,
            "C#",
            &["c#", "cs"],
            &["cs", "csx"],
            &[],
            &[],
            "csharp",
        ),
        language(
            EditorLanguageId::Java,
            "Java",
            &[],
            &["java"],
            &[],
            &[],
            "java",
        ),
        language(
            EditorLanguageId::Kotlin,
            "Kotlin",
            &["kt"],
            &["kt", "kts"],
            &[],
            &[],
            "kotlin",
        ),
        language(
            EditorLanguageId::Scala,
            "Scala",
            &[],
            &["scala", "sc"],
            &[],
            &[],
            "scala",
        ),
        language(
            EditorLanguageId::Ruby,
            "Ruby",
            &["rb"],
            &["rb"],
            &[],
            &[],
            "ruby",
        ),
        language(
            EditorLanguageId::Php,
            "PHP",
            &[],
            &["php", "phtml"],
            &[],
            &["php"],
            "php",
        ),
        language(EditorLanguageId::Lua, "Lua", &[], &["lua"], &[], &[], "lua"),
        language(
            EditorLanguageId::Swift,
            "Swift",
            &[],
            &["swift"],
            &[],
            &[],
            "swift",
        ),
        language(EditorLanguageId::Zig, "Zig", &[], &["zig"], &[], &[], "zig"),
        language(
            EditorLanguageId::Rust,
            "Rust",
            &["rs"],
            &["rs"],
            &[],
            &[],
            "rust",
        ),
        language(
            EditorLanguageId::Javascript,
            "JavaScript",
            &["js"],
            &["js", "jsx", "mjs", "cjs"],
            &[],
            &[],
            "javascript",
        ),
        language(
            EditorLanguageId::Typescript,
            "TypeScript",
            &["ts"],
            &["ts", "mts", "cts", "d.ts"],
            &[],
            &[],
            "typescript",
        ),
        language(EditorLanguageId::Tsx, "TSX", &[], &["tsx"], &[], &[], "tsx"),
        language(
            EditorLanguageId::Make,
            "Make",
            &["makefile"],
            &["mk"],
            &["Makefile"],
            &[],
            "make",
        ),
        language(
            EditorLanguageId::Dockerfile,
            "Dockerfile",
            &["docker"],
            &["dockerfile"],
            &["Dockerfile"],
            &[],
            "text",
        ),
    ]
}

fn language(
    id: EditorLanguageId,
    display_name: &'static str,
    aliases: &[&'static str],
    extensions: &[&'static str],
    filenames: &[&'static str],
    first_line_patterns: &[&'static str],
    highlighter_name: &'static str,
) -> EditorLanguageDefinition {
    EditorLanguageDefinition {
        id,
        display_name,
        aliases: aliases.to_vec(),
        extensions: extensions.to_vec(),
        filenames: filenames.to_vec(),
        first_line_patterns: first_line_patterns.to_vec(),
        highlighter_name: highlighter_name.to_string(),
        enabled_by_default: true,
    }
}
