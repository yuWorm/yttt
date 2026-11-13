import XCTest

final class HostListTests: XCTestCase {
    private let a = "aaaaaaaa1111", b = "bbbbbbbb2222"
    private func data(_ text: String) -> Data { Data(text.utf8) }

    func testANewHostGoesLastAndBecomesCurrent() {
        let list = HostList().added(a, data("A")).added(b, data("B"))
        XCTAssertEqual(list.hosts.map(\.id), [a, b])
        XCTAssertEqual(list.current, b)
        XCTAssertEqual(list.currentRecord, data("B"))
    }

    func testPairingAgainReplacesTheRecordInPlace() {
        let list = HostList().added(a, data("A")).added(b, data("B")).added(a, data("A2"))
        XCTAssertEqual(list.hosts.map(\.id), [a, b])
        XCTAssertEqual(list.current, a)
        XCTAssertEqual(list.currentRecord, data("A2"))
    }

    func testRemovingTheCurrentHostFallsBackToTheFirstLeft() {
        let list = HostList().added(a, data("A")).added(b, data("B"))
        XCTAssertEqual(list.removed(b).current, a)
        XCTAssertEqual(list.selected(a).removed(a).current, b)
        XCTAssertEqual(list.removed(a).current, b)
        let empty = list.removed(a).removed(b)
        XCTAssertNil(empty.current)
        XCTAssertNil(empty.currentRecord)
    }

    func testSelectingSwitchesOnlyToAKnownHost() {
        let list = HostList().added(a, data("A")).added(b, data("B"))
        XCTAssertEqual(list.selected(a).current, a)
        XCTAssertEqual(list.selected("unknown").current, b)
    }

    func testHostsWithTheSameNameAreToldApartByTheirIdEnd() {
        let c = "cccccccc3333"
        let list = HostList().added(a, data("mac")).added(b, data("mac")).added(c, data("linux"))
        let names = list.names { String(decoding: $0, as: UTF8.self) }
        XCTAssertEqual(names.map(\.id), [a, b, c])
        XCTAssertEqual(names.map(\.name), ["mac · 1111", "mac · 2222", "linux"])
    }

    func testTheListIsKeptAndTheOldSingleRecordMovesIn() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("hosts-\(UUID())")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = HostStore(directory: dir) { String(decoding: $0, as: UTF8.self) == "old" ? self.a : self.b }
        try data("old").write(to: dir.appendingPathComponent("host.json"))
        let migrated = store.load()
        XCTAssertEqual(migrated.hosts.map(\.id), [a])
        XCTAssertEqual(migrated.current, a)
        XCTAssertFalse(FileManager.default.fileExists(atPath: dir.appendingPathComponent("host.json").path))

        store.save(migrated.added(b, data("new")).selected(a))
        let loaded = store.load()
        XCTAssertEqual(loaded.hosts.map(\.id), [a, b])
        XCTAssertEqual(loaded.current, a)
        XCTAssertEqual(loaded.hosts[1].record, data("new"))
    }
}
