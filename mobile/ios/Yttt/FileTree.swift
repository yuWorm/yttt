import Foundation

struct FileRow: Identifiable, Equatable {
    let entry: FileEntry
    let depth: Int
    var id: String { entry.path }
}

extension FileEntry {
    var isDirectory: Bool { kind == .directory || kind == .symlinkDirectory }
}

/// The right drawer's file tree, read a directory at a time: directories
/// first, then by name ignoring case; a directory's entries show while it is
/// expanded.
struct FileTree {
    private var children: [String: [FileEntry]] = [:]
    private var expanded: Set<String> = []

    mutating func loaded(_ directory: String, _ entries: [FileEntry]) {
        children[directory] = entries.sorted {
            if $0.isDirectory != $1.isDirectory { return $0.isDirectory }
            let (left, right) = ($0.name.lowercased(), $1.name.lowercased())
            return left != right ? left < right : $0.name < $1.name
        }
    }

    /// Expands or collapses `path`; returns it when its entries must be read first.
    mutating func toggle(_ path: String) -> String? {
        if expanded.remove(path) != nil { return nil }
        expanded.insert(path)
        return children[path] == nil ? path : nil
    }

    func isExpanded(_ path: String) -> Bool { expanded.contains(path) }

    func rows() -> [FileRow] {
        var rows: [FileRow] = []
        func add(_ directory: String, _ depth: Int) {
            for entry in children[directory] ?? [] {
                rows.append(FileRow(entry: entry, depth: depth))
                if entry.isDirectory, expanded.contains(entry.path) { add(entry.path, depth + 1) }
            }
        }
        add("", 0)
        return rows
    }
}
