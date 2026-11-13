import XCTest

final class AppearanceTests: XCTestCase {
    func testSizesStayInRange() {
        XCTAssertEqual(AppearanceSettings.terminalSize(4), 8)
        XCTAssertEqual(AppearanceSettings.terminalSize(30), 20)
        XCTAssertEqual(AppearanceSettings.terminalSize(13), 13)
        XCTAssertEqual(AppearanceSettings.editorSize(2), 10)
        XCTAssertEqual(AppearanceSettings.editorSize(99), 24)
    }

    func testDefaultsAreWhatTheAppShowedBefore() {
        XCTAssertEqual(AppearanceSettings.defaultTerminalSize, 11)
        XCTAssertEqual(AppearanceSettings.defaultEditorSize, 14)
        XCTAssertEqual(AppearanceSettings.defaultFont, "system")
        XCTAssertEqual(AppearanceSettings.defaultTheme, "system")
    }

    func testEveryFontLoadsAndUnknownOnesFallBackToTheSystemMonospace() {
        for choice in AppearanceSettings.fonts {
            let font = AppearanceSettings.font(choice.id, size: 12)
            XCTAssertEqual(font.pointSize, 12, choice.id)
            XCTAssertTrue(font.fontDescriptor.symbolicTraits.contains(.traitMonoSpace)
                          || ["menlo", "courier", "jetbrains"].contains(choice.id), choice.id)
            XCTAssertTrue(AppearanceSettings.font(choice.id, size: 12, bold: true)
                .fontDescriptor.symbolicTraits.contains(.traitBold), choice.id)
        }
        let fallback = AppearanceSettings.font("from-a-later-version", size: 12)
        XCTAssertEqual(fallback.fontName, AppearanceSettings.font("system", size: 12).fontName)
    }

    func testColorsComeFromTheirHexValue() {
        var (r, g, b, a): (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0)
        UIColor(rgb: 0xFF8000).getRed(&r, green: &g, blue: &b, alpha: &a)
        XCTAssertEqual([r, g, b, a], [1, 128.0 / 255, 0, 1])
    }
}
