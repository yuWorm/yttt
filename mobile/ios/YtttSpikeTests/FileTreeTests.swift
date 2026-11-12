import XCTest

final class FileTreeTests: XCTestCase {
    private func file(_ path: String) -> FileEntry {
        FileEntry(name: String(path.split(separator: "/").last!), path: path, kind: .file)
    }
    private func dir(_ path: String) -> FileEntry {
        FileEntry(name: String(path.split(separator: "/").last!), path: path, kind: .directory)
    }

    func testDirectoriesFirstThenNamesAndChildrenWhenExpanded() {
        var tree = FileTree()
        tree.loaded("", [file("b.txt"), dir("src"), file("A.md"), dir("docs")])
        XCTAssertEqual(tree.rows().map(\.entry.path), ["docs", "src", "A.md", "b.txt"])

        XCTAssertEqual(tree.toggle("src"), "src")
        tree.loaded("src", [file("src/main.js"), dir("src/lib")])
        XCTAssertEqual(tree.rows().map { "\($0.entry.path):\($0.depth)" },
                       ["docs:0", "src:0", "src/lib:1", "src/main.js:1", "A.md:0", "b.txt:0"])
        XCTAssertNil(tree.toggle("src"))
        XCTAssertEqual(tree.rows().count, 4)
        XCTAssertNil(tree.toggle("src"))
        XCTAssertTrue(tree.isExpanded("src"))
        XCTAssertEqual(tree.rows().count, 6)
    }
}
