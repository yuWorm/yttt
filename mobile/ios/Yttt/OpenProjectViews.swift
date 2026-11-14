import SwiftUI

/// How a project is added from the phone.
enum OpenProjectSource: String, Identifiable {
    case folder, history
    var id: String { rawValue }
}

/// Chooses a folder on the Host, starting at its home, and opens it as a project.
struct HostFolderPicker: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var directory: HostDirectory?
    @State private var error: String?
    @State private var opening = false

    var body: some View {
        NavigationStack {
            List {
                if let directory {
                    Section {
                        if let parent = directory.parent {
                            Button { Task { await load(parent) } } label: {
                                Label("上一级", systemImage: "arrow.turn.left.up")
                            }
                        }
                        if directory.folders.isEmpty {
                            Text("这里没有子目录").foregroundStyle(.secondary)
                        }
                        ForEach(directory.folders, id: \.path) { folder in
                            Button { Task { await load(folder.path) } } label: {
                                HStack(spacing: 12) {
                                    Image(systemName: "folder.fill").foregroundStyle(.blue)
                                    Text(folder.name).foregroundStyle(.primary).lineLimit(1)
                                    Spacer(minLength: 4)
                                    Image(systemName: "chevron.right")
                                        .font(.caption.weight(.semibold))
                                        .foregroundStyle(.tertiary)
                                }
                            }
                        }
                    } header: {
                        Text(directory.path).textCase(nil).lineLimit(2).truncationMode(.head)
                    }
                } else if error == nil {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
            .safeAreaInset(edge: .top) {
                if let error {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.red).padding(.horizontal)
                }
            }
            .safeAreaInset(edge: .bottom) {
                if let directory {
                    Button {
                        Task { await open(directory.path) }
                    } label: {
                        Group {
                            if opening { ProgressView() } else { Text("打开这个目录") }
                        }
                        .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(opening)
                    .padding()
                    .background(.bar)
                }
            }
            .navigationTitle("从目录打开")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("取消") { dismiss() } }
            }
        }
        .task {
            guard directory == nil, let session = model.session else { return }
            do {
                let home = try await session.hostHome()
                await load(home)
            } catch {
                self.error = error.message
            }
        }
    }

    private func load(_ path: String) async {
        guard let session = model.session else { return }
        do {
            directory = try await session.browseHost(path: path)
            error = nil
        } catch {
            self.error = error.message
        }
    }

    private func open(_ path: String) async {
        opening = true
        defer { opening = false }
        if let failure = await model.openProject(path: path) {
            error = failure
        } else {
            dismiss()
        }
    }
}

/// The projects the Host opened before; one still open is shown, not reopened.
struct RecentProjectsPicker: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var projects: [RecentProject]?
    @State private var error: String?
    @State private var opening: String?

    var body: some View {
        NavigationStack {
            Group {
                if let projects, projects.isEmpty {
                    ContentUnavailableView("没有打开过的项目", systemImage: "clock",
                                           description: Text("在电脑上打开过的项目会出现在这里。"))
                } else if let projects {
                    List(projects, id: \.path) { project in
                        Button { Task { await open(project) } } label: { row(project) }
                            .disabled(opening != nil)
                    }
                } else if error == nil {
                    ProgressView()
                }
            }
            .safeAreaInset(edge: .top) {
                if let error {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.red).padding(.horizontal)
                }
            }
            .navigationTitle("从历史打开")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("取消") { dismiss() } }
            }
        }
        .task {
            guard let session = model.session else { return }
            do { projects = try await session.recentProjects() } catch { self.error = error.message }
        }
    }

    private func row(_ project: RecentProject) -> some View {
        HStack(spacing: 12) {
            IconTile(systemName: "clock.arrow.circlepath", color: .indigo)
            VStack(alignment: .leading, spacing: 2) {
                Text(project.title).foregroundStyle(.primary).lineLimit(1)
                Text(project.path).font(.caption).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.head)
            }
            Spacer(minLength: 4)
            if opening == project.path {
                ProgressView()
            } else if project.openProjectId != nil {
                Text("已打开").font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
            }
        }
    }

    private func open(_ project: RecentProject) async {
        opening = project.path
        defer { opening = nil }
        if let failure = await model.openProject(path: project.path) {
            error = failure
        } else {
            dismiss()
        }
    }
}
