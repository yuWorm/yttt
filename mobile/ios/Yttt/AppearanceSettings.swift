import UIKit

/// The appearance settings' keys, ranges and defaults, and the fonts.
enum AppearanceSettings {
    static let terminalFontKey = "terminal.font"
    static let terminalSizeKey = "terminal.fontSize"
    static let terminalThemeKey = "terminal.theme"
    static let editorFontKey = "editor.font"
    static let editorSizeKey = "editor.fontSize"
    static let editorThemeKey = "editor.theme"
    static let editorWrapKey = "editor.wrap"
    static let editorLineNumbersKey = "editor.lineNumbers"

    static let defaultFont = "system"
    static let defaultTheme = "system"
    static let defaultTerminalSize = 11.0
    static let defaultEditorSize = 14.0
    static let terminalSizes = 8.0...20.0
    static let editorSizes = 10.0...24.0

    static func terminalSize(_ size: Double) -> Double { min(max(size, terminalSizes.lowerBound), terminalSizes.upperBound) }
    static func editorSize(_ size: Double) -> Double { min(max(size, editorSizes.lowerBound), editorSizes.upperBound) }

    struct FontChoice: Identifiable {
        let id: String
        let name: String
    }

    /// JetBrains Mono comes with the app (OFL); the others are iOS's own.
    static let fonts = [
        FontChoice(id: "system", name: "系统等宽（SF Mono）"),
        FontChoice(id: "jetbrains", name: "JetBrains Mono"),
        FontChoice(id: "menlo", name: "Menlo"),
        FontChoice(id: "courier", name: "Courier New"),
    ]

    static func fontName(_ id: String) -> String {
        fonts.first { $0.id == id }?.name ?? fonts[0].name
    }

    /// A font by its id; an unknown one (a setting from a later version) is
    /// the system's monospace.
    static func font(_ id: String, size: Double, bold: Bool = false) -> UIFont {
        let name: String? = switch id {
        case "jetbrains": bold ? "JetBrainsMono-Bold" : "JetBrainsMono-Regular"
        case "menlo": bold ? "Menlo-Bold" : "Menlo-Regular"
        case "courier": bold ? "CourierNewPS-BoldMT" : "CourierNewPSMT"
        default: nil
        }
        return name.flatMap { UIFont(name: $0, size: size) }
            ?? UIFont.monospacedSystemFont(ofSize: size, weight: bold ? .bold : .regular)
    }
}

extension UIColor {
    /// From `0xRRGGBB`, as the core's themes give colors.
    convenience init(rgb: UInt32) {
        self.init(red: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
                  blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
    }
}
