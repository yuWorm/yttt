import XCTest

final class LinkBannerTests: XCTestCase {
    func testTheBannerSaysWhatTheLinkDoes() {
        XCTAssertEqual(LinkBannerText.title(.ready), "")
        XCTAssertEqual(LinkBannerText.title(.connecting), "正在连接…")
        XCTAssertEqual(LinkBannerText.title(.reconnecting(attempt: 1, reason: "reset")), "连接断开，正在重连…")
        XCTAssertEqual(LinkBannerText.title(.reconnecting(attempt: 3, reason: "reset")), "连接断开，正在重连（第 3 次）…")
        XCTAssertEqual(LinkBannerText.title(.lost(reason: "the Host refused")), "连接已断开：the Host refused")
        XCTAssertEqual(LinkBannerText.freshAfter, 15)
    }
}
