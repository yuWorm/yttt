import XCTest

/// Taps through the app against a running spike Host (crates/yttt-mobile-core
/// examples/spike_host), attaching a screenshot at each step. The app should
/// be paired with it already; to pair first, pass the Host's code as
/// TEST_RUNNER_YTTT_PAIR_CODE.
final class WalkthroughTests: XCTestCase {
    private let app = XCUIApplication()

    override func setUp() {
        continueAfterFailure = true
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

    private func tree(_ name: String) {
        let attachment = XCTAttachment(string: app.debugDescription)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func button(containing text: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    private func dismissMenu() {
        // The middle of the screen: outside any menu, on the overview's
        // background or an observed terminal, where a tap does nothing.
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.72)).tap()
    }

    func testWalkthrough() throws {
        // The overview, once connected.
        XCTAssertTrue(app.staticTexts["Agent"].waitForExistence(timeout: 30))
        // The fake Agents cycle; one is waiting or working within a minute.
        XCTAssertTrue(button(containing: "spike ·").waitForExistence(timeout: 60)
                      || button(containing: "Spike ·").waitForExistence(timeout: 5), "an Agent row")
        shot("01-overview")

        // The Host menu.
        let host = button(containing: "WormdeMac")
        if host.waitForExistence(timeout: 5) {
            // The row's own button wraps the menu's; tap the menu.
            let menu = host.buttons.firstMatch.exists ? host.buttons.firstMatch : host
            menu.tap()
            sleep(1)
            shot("02-host-menu")
            tree("02-host-menu-tree")
            XCTAssertTrue(app.buttons["添加 Host"].waitForExistence(timeout: 5), "the Host menu")
            dismissMenu()
        }

        // An Agent pushes its project page.
        let agent = button(containing: "Spike ·").exists ? button(containing: "Spike ·") : button(containing: "spike ·")
        agent.tap()
        XCTAssertTrue(app.buttons["文件与会话"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.tabBars.firstMatch.isHittable, "the tab bar hides on the project page")
        shot("03-project")

        // The title menu switches project.
        let title = app.navigationBars.buttons["Spike"]
        if title.waitForExistence(timeout: 5) {
            title.tap()
            XCTAssertTrue(app.buttons["回到总控"].waitForExistence(timeout: 5))
            shot("04-project-switcher")
            dismissMenu()
        } else {
            XCTFail("no title menu")
        }

        // A tab with two panes, and the pane bar.
        shot("04b-after-switcher")
        button(containing: "Server").tap()
        sleep(1)
        tree("05-panes-tree")
        let logs = app.buttons["Logs"]
        XCTAssertTrue(logs.waitForExistence(timeout: 10), "the pane bar")
        shot("05-panes")
        logs.tap()
        shot("06-pane-logs")

        // The tab overview.
        app.buttons["全部 Tab"].tap()
        XCTAssertTrue(app.buttons["完成"].waitForExistence(timeout: 5))
        shot("07-tab-overview")
        app.buttons["完成"].tap()

        // Take control, type, give back.
        let take = button(containing: "接管")
        XCTAssertTrue(take.waitForExistence(timeout: 5))
        take.tap()
        let giveBack = app.buttons["交还电脑"]
        XCTAssertTrue(giveBack.waitForExistence(timeout: 10))
        app.typeText("echo uitest\n")
        sleep(1)
        shot("08-control")
        giveBack.tap()

        // Files and sessions, then a file.
        app.buttons["文件与会话"].tap()
        XCTAssertTrue(app.buttons["README.md"].waitForExistence(timeout: 10))
        shot("09-files")
        app.buttons["会话"].tap()
        sleep(1)
        shot("10-sessions")
        app.buttons["文件"].tap()
        app.buttons["README.md"].tap()
        XCTAssertTrue(app.buttons["保存"].waitForExistence(timeout: 10))
        sleep(1)
        shot("11-editor")
        app.buttons["关闭"].tap()

        // Back to the overview, then the other tabs.
        app.navigationBars.buttons["总控"].tap()
        XCTAssertTrue(app.tabBars.firstMatch.waitForExistence(timeout: 5))
        app.tabBars.buttons["项目"].tap()
        shot("12-projects")
        app.tabBars.buttons["设置"].tap()
        XCTAssertTrue(app.buttons["添加 Host"].waitForExistence(timeout: 5))
        shot("13-settings")

        // Forget a Host that is not the current one: swipe, then confirm.
        let hosts = app.buttons.matching(NSPredicate(format: "label CONTAINS 'WormdeMac' AND NOT (label CONTAINS '当前')"))
        if hosts.count > 0 {
            hosts.firstMatch.swipeLeft()
            let forget = app.buttons["忘记"]
            XCTAssertTrue(forget.waitForExistence(timeout: 5))
            forget.tap()
            sleep(1)
            shot("14-forget-confirm")
            let confirm = app.sheets.buttons["忘记"].exists ? app.sheets.buttons["忘记"] : app.buttons["忘记"].firstMatch
            confirm.tap()
            sleep(1)
            shot("15-settings-after-forget")
        }
    }
}
