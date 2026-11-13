import XCTest
import SwiftTerm

/// Feeds the repaints `VtEncoder` produced (see
/// `crates/yttt-mobile-core/examples/vt_fixtures.rs`) into SwiftTerm and
/// checks the screen matches the Host viewport they came from.
final class RepaintFixtureTests: XCTestCase {
    struct Fixtures: Decodable { let fixtures: [Fixture] }
    struct Fixture: Decodable {
        let name: String
        let cols: Int
        let rows: Int
        let bytes: [UInt8]
        let expected: Expected
    }
    struct Expected: Decodable {
        let rows: [String]
        let cursor: Cursor
        let cells: [Cell]
    }
    struct Cursor: Decodable { let row: Int; let col: Int; let visible: Bool }
    struct Cell: Decodable {
        let row: Int
        let col: Int
        let char: String?
        let fg: String
        let bg: String
        let bold: Bool
        let italic: Bool
        let underline: Bool
        let inverse: Bool
    }

    final class Sink: TerminalDelegate {
        func send(source: Terminal, data: ArraySlice<UInt8>) {}
    }

    func loadFixtures() throws -> [Fixture] {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "vt-fixtures", withExtension: "json"))
        return try JSONDecoder().decode(Fixtures.self, from: Data(contentsOf: url)).fixtures
    }

    func describe(_ color: Attribute.Color) -> String {
        switch color {
        case .ansi256(let code): return "ansi:\(code)"
        case .trueColor(let red, let green, let blue): return "rgb:\(red),\(green),\(blue)"
        case .defaultColor, .defaultInvertedColor: return "default"
        }
    }

    func testRepaintsMatchTheHostViewport() throws {
        let fixtures = try loadFixtures()
        XCTAssertFalse(fixtures.isEmpty)
        for fixture in fixtures {
            let sink = Sink()
            let terminal = Terminal(
                delegate: sink,
                options: TerminalOptions(cols: fixture.cols, rows: fixture.rows, scrollback: 0))
            terminal.feed(byteArray: fixture.bytes)

            for (row, expected) in fixture.expected.rows.enumerated() {
                // The repaint writes a row's trailing blanks as real spaces,
                // which `trimRight` keeps; trim them here like the expectation.
                var line = terminal.getLine(row: row)?
                    .translateToString(trimRight: true, skipNullCellsFollowingWide: true) ?? ""
                while line.last == " " { line.removeLast() }
                XCTAssertEqual(line, expected, "\(fixture.name) row \(row)")
            }
            let cursor = terminal.getCursorLocation()
            XCTAssertEqual(cursor.y, fixture.expected.cursor.row, "\(fixture.name) cursor row")
            XCTAssertEqual(cursor.x, fixture.expected.cursor.col, "\(fixture.name) cursor column")

            for cell in fixture.expected.cells {
                let data = try XCTUnwrap(terminal.getCharData(col: cell.col, row: cell.row), "\(fixture.name) \(cell.row),\(cell.col)")
                let place = "\(fixture.name) \(cell.row),\(cell.col)"
                if let char = cell.char {
                    XCTAssertEqual(String(terminal.getCharacter(for: data)), char, "\(place) char")
                }
                let attribute = data.attribute
                XCTAssertEqual(describe(attribute.fg), cell.fg, "\(place) fg")
                XCTAssertEqual(describe(attribute.bg), cell.bg, "\(place) bg")
                XCTAssertEqual(attribute.style.contains(.bold), cell.bold, "\(place) bold")
                XCTAssertEqual(attribute.style.contains(.italic), cell.italic, "\(place) italic")
                XCTAssertEqual(attribute.underlineStyle != .none, cell.underline, "\(place) underline")
                XCTAssertEqual(attribute.style.contains(.inverse), cell.inverse, "\(place) inverse")
            }
        }
    }
}
