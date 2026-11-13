import XCTest

final class SecretStoreTests: XCTestCase {
    private var store: SecretStore!

    override func setUp() {
        store = SecretStore(service: "com.yttt.yai.test.\(UUID())", account: "device-secret")
    }

    override func tearDown() {
        store.delete()
    }

    func testANewSecretIsMadeOnceAndKept() throws {
        let first = try store.deviceSecret()
        XCTAssertEqual(first.count, 32)
        XCTAssertEqual(try store.deviceSecret(), first)
        XCTAssertEqual(try store.load(), first)
    }
}
