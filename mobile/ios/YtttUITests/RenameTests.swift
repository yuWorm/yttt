import XCTest

/// Renames the paired Host on this phone, against a spike Host: pass its code
/// as TEST_RUNNER_YTTT_PAIR_CODE on a fresh install.
final class RenameTests: XCTestCase {
    private let app = XCUIApplication()

    override func setUp() {
        continueAfterFailure = false
        if let code = ProcessInfo.processInfo.environment["YTTT_PAIR_CODE"] {
            app.launchEnvironment["YTTT_DEBUG_PAIR_CODE"] = code
        }
        app.launch()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func rename(_ row: XCUIElement, to name: String) {
        row.swipeLeft()
        app.buttons["重命名"].tap()
        let field = app.alerts.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        // At the end of the text, so the deletes take all of it.
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.97, dy: 0.5)).tap()
        if let current = field.value as? String, !current.isEmpty {
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count))
        }
        field.typeText(name)
        shot("rename-\(name.isEmpty ? "clear" : name)")
        app.alerts.buttons["好"].tap()
    }

    func testRenamingAHostChangesItsNameOnThisPhoneOnly() {
        XCTAssertTrue(app.tabBars.buttons["设置"].waitForExistence(timeout: 60), "paired")
        app.tabBars.buttons["设置"].tap()
        let host = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "WormdeMac")).firstMatch
        XCTAssertTrue(host.waitForExistence(timeout: 10))

        rename(host, to: "Office Mac")
        let renamed = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Office Mac")).firstMatch
        XCTAssertTrue(renamed.waitForExistence(timeout: 5))
        XCTAssertFalse(renamed.label.contains("local"), renamed.label)
        shot("settings-renamed")
        app.tabBars.buttons["总控"].tap()
        XCTAssertTrue(app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Office Mac")).firstMatch.waitForExistence(timeout: 5),
                      "the overview's Host menu")
        shot("overview-renamed")

        // Blank goes back to the computer's own name.
        app.tabBars.buttons["设置"].tap()
        rename(renamed, to: "")
        XCTAssertTrue(app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "WormdeMac")).firstMatch.waitForExistence(timeout: 5))
    }
}
