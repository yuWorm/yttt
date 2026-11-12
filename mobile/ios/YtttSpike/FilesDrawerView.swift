import SwiftUI

/// The right drawer: the project's files, a directory at a time, and its Agent sessions.
struct FilesDrawerView: View {
    @EnvironmentObject private var model: AppModel
    let projectId: String
    @State private var section = 0
    @State private var tree = FileTree()
    @State private var sessions: [SessionEntry]?
    @State private var error: String?

    var body: some View {
        VStack(spacing: 0) {
            Picker("", selection: $section) {
                Text("文件").tag(0)
                Text("会话").tag(1)
            }
            .pickerStyle(.segmented)
            .padding()
            if let error { Text(error).font(.caption).foregroundStyle(.red).padding(.horizontal) }
            if section == 0 {
                List(tree.rows()) { row in
                    Button {
                        if row.entry.isDirectory {
                            if let toRead = tree.toggle(row.entry.path) { Task { await read(toRead) } }
                        } else {
                            model.drawerFilesOpen = false
                            model.editing = EditingFile(projectId: projectId, path: row.entry.path)
                        }
                    } label: {
                        HStack(spacing: 6) {
                            Image(systemName: row.entry.isDirectory
                                  ? (tree.isExpanded(row.entry.path) ? "folder.fill" : "folder")
                                  : "doc.text")
                                .foregroundStyle(row.entry.isDirectory ? Color.accentColor : .secondary)
                            Text(row.entry.name).lineLimit(1)
                        }
                        .padding(.leading, CGFloat(row.depth) * 16)
                    }
                    .foregroundStyle(.primary)
                }
                .listStyle(.plain)
            } else if let sessions {
                if sessions.isEmpty {
                    Text("这个项目还没有 Agent 会话").foregroundStyle(.secondary).padding()
                    Spacer()
                } else {
                    List(sessions, id: \.id) { entry in
                        VStack(alignment: .leading) {
                            Text(entry.title).lineLimit(2)
                            Text([entry.provider, entry.model,
                                  Date(timeIntervalSince1970: TimeInterval(entry.updatedAtMs) / 1000)
                                    .formatted(date: .abbreviated, time: .shortened)]
                                    .compactMap { $0 }.joined(separator: " · "))
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    .listStyle(.plain)
                }
            } else {
                ProgressView().padding()
                Spacer()
            }
        }
        .task(id: projectId) { await read("") }
        .task(id: section) {
            guard section == 1, sessions == nil, let session = model.session else { return }
            do { sessions = try await session.agentSessions(projectId: projectId) } catch {
                self.error = error.localizedDescription
                sessions = []
            }
        }
    }

    private func read(_ directory: String) async {
        guard let session = model.session else { return }
        do {
            tree.loaded(directory, try await session.listDirectory(projectId: projectId, directory: directory))
        } catch {
            self.error = error.localizedDescription
        }
    }
}
