import Runestone
import SwiftUI
import TreeSitterBashRunestone
import TreeSitterCPPRunestone
import TreeSitterCRunestone
import TreeSitterCSharpRunestone
import TreeSitterCSSRunestone
import TreeSitterGoRunestone
import TreeSitterHTMLRunestone
import TreeSitterJavaRunestone
import TreeSitterJavaScriptRunestone
import TreeSitterJSONRunestone
import TreeSitterLuaRunestone
import TreeSitterMarkdownRunestone
import TreeSitterPHPRunestone
import TreeSitterPythonRunestone
import TreeSitterRubyRunestone
import TreeSitterRustRunestone
import TreeSitterSQLRunestone
import TreeSitterSwiftRunestone
import TreeSitterTOMLRunestone
import TreeSitterTSXRunestone
import TreeSitterTypeScriptRunestone
import TreeSitterYAMLRunestone
import UIKit

/// How the terminal draws: its font and colors.
struct TerminalLook: Equatable {
    var font: String
    var size: Double
    var theme: Theme

    var uiFont: UIFont { AppearanceSettings.font(font, size: size) }
}

/// How the editor draws, and what it highlights.
struct EditorLook: Equatable {
    var font: String
    var size: Double
    var theme: Theme
    var wrap: Bool
    var lineNumbers: Bool
}

/// The grammar for the core's language id; Kotlin, Dockerfiles, XML and the
/// like have none here and stay plain.
func treeSitterLanguage(_ id: String?) -> TreeSitterLanguage? {
    switch id {
    case "rust": .rust
    case "javascript": .javaScript
    case "typescript": .typeScript
    case "tsx": .tsx
    case "json": .json
    case "python": .python
    case "go": .go
    case "swift": .swift
    case "java": .java
    case "c": .c
    case "cpp": .cpp
    case "csharp": .cSharp
    case "ruby": .ruby
    case "php": .php
    case "lua": .lua
    case "shell": .bash
    case "markdown": .markdown
    case "yaml": .yaml
    case "toml": .toml
    case "html": .html
    case "css": .css
    case "sql": .sql
    default: nil
    }
}

/// A core theme as Runestone draws it. Highlight names are tree-sitter
/// captures such as `keyword.function` or `string.special`; the first part
/// picks the color.
final class EditorTheme: Runestone.Theme {
    let look: EditorLook
    let font: UIFont
    let textColor: UIColor
    let gutterBackgroundColor: UIColor
    let gutterHairlineColor: UIColor
    let lineNumberColor: UIColor
    let lineNumberFont: UIFont
    let selectedLineBackgroundColor: UIColor
    let selectedLinesLineNumberColor: UIColor
    let selectedLinesGutterBackgroundColor: UIColor
    let invisibleCharactersColor: UIColor
    let pageGuideHairlineColor: UIColor
    let pageGuideBackgroundColor: UIColor
    let markedTextBackgroundColor: UIColor
    let backgroundColor: UIColor

    init(_ look: EditorLook) {
        self.look = look
        let theme = look.theme
        font = AppearanceSettings.font(look.font, size: look.size)
        textColor = UIColor(rgb: theme.foreground)
        backgroundColor = UIColor(rgb: theme.background)
        gutterBackgroundColor = backgroundColor
        gutterHairlineColor = UIColor(rgb: theme.syntax.comment).withAlphaComponent(0.25)
        lineNumberColor = UIColor(rgb: theme.syntax.comment)
        lineNumberFont = AppearanceSettings.font(look.font, size: max(look.size - 2, 9))
        selectedLineBackgroundColor = UIColor(rgb: theme.selection).withAlphaComponent(0.35)
        selectedLinesLineNumberColor = textColor
        selectedLinesGutterBackgroundColor = selectedLineBackgroundColor
        invisibleCharactersColor = lineNumberColor.withAlphaComponent(0.5)
        pageGuideHairlineColor = gutterHairlineColor
        pageGuideBackgroundColor = backgroundColor
        markedTextBackgroundColor = UIColor(rgb: theme.selection)
    }

    func textColor(for highlightName: String) -> UIColor? {
        let syntax = look.theme.syntax
        let color: UInt32? = switch highlightName.split(separator: ".").first.map(String.init) ?? highlightName {
        case "comment": syntax.comment
        case "keyword", "include", "conditional", "repeat", "exception", "storageclass": syntax.keyword
        case "string", "character", "escape": syntax.string
        case "number", "float", "boolean": syntax.number
        case "function", "method", "constructor": syntax.function
        case "type", "tag", "namespace", "module": syntax.typeName
        case "constant", "attribute", "label", "property": syntax.constant
        case "text":
            // Markdown: headings and links stand out, the rest is text.
            highlightName.hasPrefix("text.title") ? syntax.keyword
                : highlightName.hasPrefix("text.uri") || highlightName.hasPrefix("text.reference") ? syntax.function
                : highlightName.hasPrefix("text.literal") ? syntax.string : nil
        default: nil
        }
        return color.map { UIColor(rgb: $0) }
    }

    func fontTraits(for highlightName: String) -> FontTraits {
        if highlightName.hasPrefix("comment") { return .italic }
        if highlightName.hasPrefix("text.title") || highlightName.hasPrefix("text.strong") { return .bold }
        return []
    }
}

private func systemDark(_ scheme: ColorScheme) -> Bool { scheme == .dark }

/// The settings page's appearance section.
struct AppearanceSection: View {
    @AppStorage(AppearanceSettings.terminalThemeKey) private var terminalTheme = AppearanceSettings.defaultTheme
    @AppStorage(AppearanceSettings.editorThemeKey) private var editorTheme = AppearanceSettings.defaultTheme

    var body: some View {
        Section("外观") {
            NavigationLink { TerminalAppearanceView() } label: {
                HStack(spacing: 12) {
                    IconTile(systemName: "terminal", color: .black)
                    LabeledContent("终端", value: themeName(terminalTheme))
                }
            }
            NavigationLink { EditorAppearanceView() } label: {
                HStack(spacing: 12) {
                    IconTile(systemName: "chevron.left.forwardslash.chevron.right", color: .purple)
                    LabeledContent("编辑器", value: themeName(editorTheme))
                }
            }
        }
    }
}

func themeName(_ id: String) -> String {
    themeChoices().first { $0.id == id }?.name ?? themeChoices()[0].name
}

/// A theme picker row: the name, and a strip of the palette.
private struct ThemeRow: View {
    let choice: ThemeChoice
    let selected: Bool
    let systemDark: Bool

    var body: some View {
        let palette = theme(id: choice.id, systemDark: systemDark)
        HStack(spacing: 12) {
            HStack(spacing: 0) {
                ForEach([palette.background, palette.foreground] + Array(palette.ansi[1...6]), id: \.self) { color in
                    Color(uiColor: UIColor(rgb: color)).frame(width: 8, height: 22)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(.separator))
            Text(choice.name).foregroundStyle(.primary)
            Spacer()
            if selected { Image(systemName: "checkmark").foregroundStyle(.tint) }
        }
    }
}

private struct ThemeSection: View {
    @Binding var selection: String
    let systemDark: Bool

    var body: some View {
        Section("主题") {
            ForEach(themeChoices(), id: \.id) { choice in
                Button { selection = choice.id } label: {
                    ThemeRow(choice: choice, selected: selection == choice.id, systemDark: systemDark)
                }
            }
        }
    }
}

private struct FontSection: View {
    @Binding var font: String
    @Binding var size: Double
    let sizes: ClosedRange<Double>

    var body: some View {
        Section("字体") {
            Picker("字体", selection: $font) {
                ForEach(AppearanceSettings.fonts) { choice in
                    Text(choice.name).font(Font(AppearanceSettings.font(choice.id, size: 15))).tag(choice.id)
                }
            }
            .pickerStyle(.navigationLink)
            Stepper(value: $size, in: sizes, step: 1) {
                LabeledContent("字号", value: "\(Int(size))")
            }
        }
    }
}

/// A few lines of colored output, drawn with the terminal's look.
private struct TerminalPreview: View {
    let look: TerminalLook

    var body: some View {
        let theme = look.theme
        let color = { (index: Int) in Color(uiColor: UIColor(rgb: theme.ansi[index])) }
        let font = Font(look.uiFont)
        VStack(alignment: .leading, spacing: 2) {
            (Text("~/yttt ").foregroundColor(color(4)) + Text("main ").foregroundColor(color(5))
                + Text("$ ").foregroundColor(Color(uiColor: UIColor(rgb: theme.foreground))) + Text("cargo test"))
            (Text("   Compiling ").foregroundColor(color(2)) + Text("yttt-mobile-core v0.5.7"))
            (Text("test result: ").foregroundColor(Color(uiColor: UIColor(rgb: theme.foreground)))
                + Text("ok").foregroundColor(color(10)) + Text(". 36 passed; ") + Text("0 failed").foregroundColor(color(1)))
            (Text("warning: ").foregroundColor(color(3)) + Text("unused variable ") + Text("`x`").foregroundColor(color(6)))
            HStack(spacing: 0) {
                Text("$ ")
                Rectangle().fill(Color(uiColor: UIColor(rgb: theme.cursor))).frame(width: look.size * 0.6, height: look.size * 1.2)
            }
        }
        .font(font)
        .foregroundColor(Color(uiColor: UIColor(rgb: theme.foreground)))
        .lineLimit(1)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(Color(uiColor: UIColor(rgb: theme.background)), in: RoundedRectangle(cornerRadius: 10))
        .listRowInsets(EdgeInsets())
        .listRowBackground(Color.clear)
    }
}

struct TerminalAppearanceView: View {
    @Environment(\.colorScheme) private var scheme
    @AppStorage(AppearanceSettings.terminalFontKey) private var font = AppearanceSettings.defaultFont
    @AppStorage(AppearanceSettings.terminalSizeKey) private var size = AppearanceSettings.defaultTerminalSize
    @AppStorage(AppearanceSettings.terminalThemeKey) private var themeId = AppearanceSettings.defaultTheme

    var body: some View {
        Form {
            Section {
                TerminalPreview(look: TerminalLook(font: font, size: AppearanceSettings.terminalSize(size),
                                                   theme: theme(id: themeId, systemDark: systemDark(scheme))))
            } footer: {
                Text("改字号会改变终端的行列数；接管时电脑上的终端也随之调整。")
            }
            FontSection(font: $font, size: $size, sizes: AppearanceSettings.terminalSizes)
            ThemeSection(selection: $themeId, systemDark: systemDark(scheme))
        }
        .navigationTitle("终端")
        .navigationBarTitleDisplayMode(.inline)
    }
}

private let previewCode = """
// 计算斐波那契数
fn fibonacci(n: u32) -> u64 {
    let mut pair = (0, 1);
    for _ in 0..n {
        pair = (pair.1, pair.0 + pair.1);
    }
    pair.0
}

const LIMIT: u32 = 42;
"""

struct EditorAppearanceView: View {
    @Environment(\.colorScheme) private var scheme
    @AppStorage(AppearanceSettings.editorFontKey) private var font = AppearanceSettings.defaultFont
    @AppStorage(AppearanceSettings.editorSizeKey) private var size = AppearanceSettings.defaultEditorSize
    @AppStorage(AppearanceSettings.editorThemeKey) private var themeId = AppearanceSettings.defaultTheme
    @AppStorage(AppearanceSettings.editorWrapKey) private var wrap = false
    @AppStorage(AppearanceSettings.editorLineNumbersKey) private var lineNumbers = true
    @State private var preview = EditorHandle()

    var body: some View {
        let look = EditorLook(font: font, size: AppearanceSettings.editorSize(size),
                              theme: theme(id: themeId, systemDark: systemDark(scheme)), wrap: wrap, lineNumbers: lineNumbers)
        Form {
            Section {
                RunestoneEditor(handle: preview, look: look, editable: false, onChange: {})
                    .frame(height: 220)
                    .clipShape(RoundedRectangle(cornerRadius: 10))
                    .listRowInsets(EdgeInsets())
                    .listRowBackground(Color.clear)
                    .onAppear { preview.setText(previewCode, language: "rust") }
            } footer: {
                Text("按文件扩展名高亮：Rust、Swift、Go、Python、JavaScript / TypeScript、C / C++、Java、JSON、YAML、TOML、Markdown、Shell 等。")
            }
            FontSection(font: $font, size: $size, sizes: AppearanceSettings.editorSizes)
            Section("显示") {
                Toggle("行号", isOn: $lineNumbers)
                Toggle("自动换行", isOn: $wrap)
            }
            ThemeSection(selection: $themeId, systemDark: systemDark(scheme))
        }
        .navigationTitle("编辑器")
        .navigationBarTitleDisplayMode(.inline)
    }
}
