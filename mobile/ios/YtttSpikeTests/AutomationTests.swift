import XCTest

/// Tests run in the Debug configuration, where the hooks are compiled in.
final class AutomationTests: XCTestCase {
    func testReadsEveryVariable() {
        let hooks = LaunchHooks(environment: [
            "YTTT_SPIKE_PAIR_CODE": "YTTT-PAIR:1:x",
            "YTTT_SPIKE_QR_IMAGE": "/tmp/qr.png",
            "YTTT_SPIKE_SWITCH": "a6a3",
            "YTTT_SPIKE_FORGET": "1",
            "YTTT_SPIKE_OPEN": "s1",
            "YTTT_SPIKE_TYPE": "ls",
            "YTTT_SPIKE_PROJECT": "p",
            "YTTT_SPIKE_TAB": "t",
            "YTTT_SPIKE_PANE": "n",
            "YTTT_SPIKE_DRAWER": "1",
            "YTTT_SPIKE_FILES": "1",
            "YTTT_SPIKE_EDIT": "README.md",
            "YTTT_SPIKE_RELAY_ONLY": "1",
        ])
        XCTAssertEqual(hooks.pairCode, "YTTT-PAIR:1:x")
        XCTAssertEqual(hooks.qrImage, "/tmp/qr.png")
        XCTAssertEqual(hooks.switchTo, "a6a3")
        XCTAssertTrue(hooks.forget)
        XCTAssertEqual(hooks.open, "s1")
        XCTAssertEqual(hooks.type, "ls")
        XCTAssertEqual(hooks.project, "p")
        XCTAssertEqual(hooks.tab, "t")
        XCTAssertEqual(hooks.pane, "n")
        XCTAssertTrue(hooks.drawer)
        XCTAssertTrue(hooks.files)
        XCTAssertEqual(hooks.edit, "README.md")
        XCTAssertTrue(hooks.relayOnly)
    }

    func testNothingSetIsEmpty() {
        let hooks = LaunchHooks(environment: [:])
        XCTAssertEqual(hooks, LaunchHooks(environment: ["PATH": "/usr/bin"]))
        XCTAssertNil(hooks.pairCode)
        XCTAssertFalse(hooks.forget || hooks.drawer || hooks.files || hooks.relayOnly)
    }

    func testOnlyOneTurnsAFlagOn() {
        let hooks = LaunchHooks(environment: ["YTTT_SPIKE_FORGET": "0", "YTTT_SPIKE_RELAY_ONLY": "yes"])
        XCTAssertFalse(hooks.forget)
        XCTAssertFalse(hooks.relayOnly)
    }
}
