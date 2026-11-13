import Foundation

/// A paired Host: `id` from the core's `hostInfo`, `record` as the core stores it.
struct HostEntry: Codable, Equatable {
    var id: String
    var record: Data
}

/// The Hosts this phone paired with, in the order they were added, and the
/// one it connects to.
struct HostList: Codable, Equatable {
    var hosts: [HostEntry] = []
    var current: String?

    var currentRecord: Data? { hosts.first { $0.id == current }?.record }

    /// A new Host goes last; pairing with a known one again replaces its record in place.
    func added(_ id: String, _ record: Data) -> HostList {
        var list = self
        if let at = hosts.firstIndex(where: { $0.id == id }) {
            list.hosts[at].record = record
        } else {
            list.hosts.append(HostEntry(id: id, record: record))
        }
        list.current = id
        return list
    }

    /// The current Host falls back to the first one left.
    func removed(_ id: String) -> HostList {
        var list = self
        list.hosts.removeAll { $0.id == id }
        if current == id { list.current = list.hosts.first?.id }
        return list
    }

    func selected(_ id: String) -> HostList {
        guard hosts.contains(where: { $0.id == id }) else { return self }
        var list = self
        list.current = id
        return list
    }

    /// Each Host's name; Hosts with the same name get their id's last four characters.
    func names(_ label: (Data) -> String) -> [(id: String, name: String)] {
        let labels = hosts.map { (id: $0.id, name: label($0.record)) }
        let counts = Dictionary(labels.map { ($0.name, 1) }, uniquingKeysWith: +)
        return labels.map { entry in
            (entry.id, counts[entry.name, default: 0] > 1 ? "\(entry.name) · \(entry.id.suffix(4))" : entry.name)
        }
    }
}

/// The list in `hosts.json`, app-private and left out of backups.
struct HostStore {
    let directory: URL

    private var file: URL { directory.appendingPathComponent("hosts.json") }

    func load() -> HostList {
        guard let data = try? Data(contentsOf: file),
              let list = try? JSONDecoder().decode(HostList.self, from: data)
        else { return HostList() }
        return list
    }

    func save(_ list: HostList) {
        guard let data = try? JSONEncoder().encode(list) else { return }
        var url = file
        try? data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? url.setResourceValues(values)
    }
}
