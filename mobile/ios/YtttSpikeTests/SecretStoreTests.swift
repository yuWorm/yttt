import XCTest

final class SecretStoreTests: XCTestCase {
    private var store: SecretStore!
    private var legacy: URL!

    override func setUp() {
        legacy = FileManager.default.temporaryDirectory.appendingPathComponent("legacy-secret-\(UUID())")
        store = SecretStore(service: "dev.yttt.spike.test.\(UUID())", account: "device-secret", legacyFile: legacy)
    }

    override func tearDown() {
        store.delete()
        try? FileManager.default.removeItem(at: legacy)
    }

    func testANewSecretIsMadeOnceAndKept() throws {
        let first = try store.deviceSecret()
        XCTAssertEqual(first.count, 32)
        XCTAssertEqual(try store.deviceSecret(), first)
        XCTAssertEqual(try store.load(), first)
    }

    func testTheOldPlainFileMovesIntoTheKeychainAndIsDeleted() throws {
        let old = Data((0..<32).map { UInt8($0) })
        try old.write(to: legacy)
        XCTAssertEqual(try store.deviceSecret(), old)
        XCTAssertFalse(FileManager.default.fileExists(atPath: legacy.path))
        XCTAssertEqual(try store.load(), old)
    }
}
