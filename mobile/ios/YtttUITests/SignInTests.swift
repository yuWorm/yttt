import XCTest

/// Signs in to a relay account, then pairs through the relay alone, against
/// a local stack: a control plane with its console, a relay
/// node and a spike Host signed in to the same account (`spike_host --account`).
/// Start from a fresh install. Pass, with the TEST_RUNNER_ prefix:
/// YTTT_CONTROL_URL, YTTT_EMAIL, YTTT_PASSWORD and YTTT_PAIR_CODE.
final class SignInTests: XCTestCase {
    private let app = XCUIApplication()
    private let safari = XCUIApplication(bundleIdentifier: "com.apple.SafariViewService")
    private let environment = ProcessInfo.processInfo.environment

    override func setUp() {
        continueAfterFailure = false
        // Through the relay only, so pairing works only with a ticket.
        app.launchEnvironment["YTTT_DEBUG_RELAY_ONLY"] = "1"
        app.launch()
    }

    private func shot(_ name: String, of target: XCUIApplication? = nil) {
        let attachment = XCTAttachment(screenshot: (target ?? app).screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func tree(_ name: String, of target: XCUIApplication) {
        let attachment = XCTAttachment(string: target.debugDescription)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func value(_ name: String) throws -> String {
        try XCTUnwrap(environment[name], "set TEST_RUNNER_\(name)")
    }

    private func replace(_ field: XCUIElement, with text: String) {
        field.tap()
        if let current = field.value as? String, !current.isEmpty, current != field.placeholderValue {
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count))
        }
        field.typeText(text)
    }

    /// Types into a web page's field: the page takes focus a moment after the tap.
    func testSignInThenPairThroughTheRelay() throws {
        let controlURL = try value("YTTT_CONTROL_URL")
        let email = try value("YTTT_EMAIL")
        let password = try value("YTTT_PASSWORD")
        let code = try value("YTTT_PAIR_CODE")

        // First run: the pairing page starts with signing in.
        let step = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "先登录 Relay 账号")).firstMatch
        XCTAssertTrue(step.waitForExistence(timeout: 20))
        shot("01-pair-signed-out")
        step.tap()

        let urlField = app.textFields["https://control.example.com"]
        XCTAssertTrue(urlField.waitForExistence(timeout: 5))
        replace(urlField, with: controlURL)
        shot("02-sign-in")

        // The console's page opens in Safari inside the app; closing it
        // cancels. (The test cannot type in the browser's own process; the
        // browser sign-in is tried on a device.)
        app.buttons["在浏览器中登录"].tap()
        XCTAssertTrue(safari.webViews.textFields.firstMatch.waitForExistence(timeout: 20), "the console's sign-in form")
        shot("03-console-sign-in", of: safari)
        // The browser's own button: the app's tree carries it, but only a tap
        // at its place reaches the browser's process.
        let cancel = app.buttons["取消"].firstMatch
        XCTAssertTrue(cancel.waitForExistence(timeout: 5))
        let frame = cancel.frame
        app.coordinate(withNormalizedOffset: .zero)
            .withOffset(CGVector(dx: frame.midX, dy: frame.midY)).tap()
        XCTAssertTrue(app.buttons["在浏览器中登录"].waitForExistence(timeout: 10))
        sleep(2)
        shot("03b-after-cancel")
        let enabled = expectation(for: NSPredicate(format: "isEnabled == true"), evaluatedWith: app.buttons["在浏览器中登录"])
        wait(for: [enabled], timeout: 5)

        // Email and password here instead.
        let emailField = app.textFields["邮箱"]
        emailField.tap()
        emailField.typeText(email)
        let passwordField = app.secureTextFields["密码"]
        passwordField.tap()
        passwordField.typeText(password)
        shot("04-password")
        app.buttons["登录"].tap()

        // Back on the pairing page, signed in.
        let signedIn = app.staticTexts["已登录 Relay 账号"]
        XCTAssertTrue(signedIn.waitForExistence(timeout: 30))
        XCTAssertTrue(app.staticTexts[email].exists)
        shot("05-pair-signed-in")

        // Pairing goes through the relay node, the spike Host confirming.
        let codeField = app.textFields["YTTT-PAIR:1:…"]
        codeField.tap()
        codeField.typeText(code)
        app.buttons["配对"].tap()
        XCTAssertTrue(app.tabBars.buttons["设置"].waitForExistence(timeout: 90), "paired and connected")
        shot("06-overview")

        app.tabBars.buttons["设置"].tap()
        XCTAssertTrue(app.staticTexts[email].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["手机经这个账号的 Relay 连接电脑。"].waitForExistence(timeout: 15))
        shot("07-settings-account")
    }
}
