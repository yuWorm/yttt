import SwiftUI

/// A file's symbol and colour, by extension.
private func fileSymbol(_ name: String) -> (String, Color) {
    let ext = (name as NSString).pathExtension.lowercased()
    switch ext {
    case "swift": return ("swift", .orange)
    case "ts", "tsx", "js", "jsx", "mjs": return ("curlybraces", .blue)
    case "json", "toml", "yaml", "yml": return ("curlybraces.square", .yellow)
    case "md", "txt": return ("doc.richtext", .secondary)
    case "rs", "kt", "py", "go", "c", "h", "cpp", "java": return ("chevron.left.forwardslash.chevron.right", .purple)
    case "sh", "zsh", "bash", "ps1": return ("terminal", .green)
    case "png", "jpg", "jpeg", "gif", "svg", "webp": return ("photo", .pink)
    default: return (name.hasPrefix(".") ? "gearshape" : "doc.text", .secondary)
    }
}

/// The project's files, a directory at a time, and its Agent sessions, in a sheet.
struct FilesView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let projectId: String
    @State private var section = 0
    @State private var tree = FileTree()
    @State private var sessions: [SessionEntry]?
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Group {
                if section == 0 { files } else { sessionList }
            }
            .safeAreaInset(edge: .top) {
                if let error {
                    Label(error, systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.red)
                        .padding(.horizontal)
                }
            }
            .navigationTitle(model.overview.projectName(projectId))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    Picker("", selection: $section) {
                        // A segmented control shows a label's icon alone; the words say more.
                        Text("文件").tag(0)
                        Text("会话").tag(1)
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 200)
                }
                ToolbarItem(placement: .confirmationAction) { Button("完成") { dismiss() } }
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

    private var files: some View {
        List(tree.rows()) { row in
            Button {
                if row.entry.isDirectory {
                    if let toRead = tree.toggle(row.entry.path) { Task { await read(toRead) } }
                } else {
                    dismiss()
                    model.editing = EditingFile(projectId: projectId, path: row.entry.path)
                }
            } label: {
                HStack(spacing: 8) {
                    if row.entry.isDirectory {
                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.tertiary)
                            .rotationEffect(.degrees(tree.isExpanded(row.entry.path) ? 90 : 0))
                            .frame(width: 12)
                        Image(systemName: "folder.fill").foregroundStyle(Color.accentColor)
                    } else {
                        let (symbol, color) = fileSymbol(row.entry.name)
                        Color.clear.frame(width: 12)
                        Image(systemName: symbol).foregroundStyle(color)
                    }
                    Text(row.entry.name).lineLimit(1)
                }
                .padding(.leading, CGFloat(row.depth) * 18)
            }
            .foregroundStyle(.primary)
        }
        .listStyle(.plain)
    }

    @ViewBuilder private var sessionList: some View {
        if let sessions {
            if sessions.isEmpty {
                ContentUnavailableView("还没有 Agent 会话", systemImage: "clock.arrow.circlepath",
                                       description: Text("在这个项目里跑过的 Agent 会列在这里。"))
            } else {
                List(sessions, id: \.id) { entry in
                    HStack(spacing: 12) {
                        AgentTile(provider: entry.provider, size: 18)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(entry.title).lineLimit(2)
                            Text([providerName(entry.provider), entry.model,
                                  Date(timeIntervalSince1970: TimeInterval(entry.updatedAtMs) / 1000)
                                    .formatted(date: .abbreviated, time: .shortened)]
                                    .compactMap { $0 }.joined(separator: " · "))
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                .listStyle(.plain)
            }
        } else {
            ProgressView().frame(maxHeight: .infinity)
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
