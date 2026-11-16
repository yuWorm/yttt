import XCTest

@MainActor
final class TrackingTerminalViewTests: XCTestCase {
    func testPinyinShowsAtTheCursorUntilItCommits() {
        let view = TrackingTerminalView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        view.refreshComposition()
        XCTAssertTrue(view.composition.isHidden)

        view.setMarkedText("ni hao", selectedRange: NSRange(location: 6, length: 0))
        view.refreshComposition()
        XCTAssertFalse(view.composition.isHidden)
        XCTAssertEqual(view.composition.attributedText?.string, "ni hao")
        XCTAssertTrue(view.bounds.contains(view.composition.frame))

        view.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        view.refreshComposition()
        XCTAssertEqual(view.composition.attributedText?.string, "ni")

        view.setMarkedText(nil, selectedRange: NSRange(location: 0, length: 0))
        view.refreshComposition()
        XCTAssertTrue(view.composition.isHidden)
    }
}
