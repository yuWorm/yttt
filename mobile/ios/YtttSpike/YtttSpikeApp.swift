import PhotosUI
import SwiftUI

@main
struct YtttSpikeApp: App {
    @StateObject private var model = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            Group {
                if model.record == nil {
                    NavigationStack { PairView() }
                } else {
                    SideDrawer(isOpen: $model.drawerOpen) {
                        RootPage()
                    } drawer: {
                        LeftDrawerView()
                    }
                }
            }
            .environmentObject(model)
            .task {
                if model.record == nil, let code = AppModel.autoPairCode {
                    await model.pair(code: code)
                }
            }
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .background { model.enterBackground() }
        }
    }
}

struct PairView: View {
    @EnvironmentObject private var model: AppModel
    @State private var code = ""
    @State private var scanning = false
    @State private var photo: PhotosPickerItem?
    @State private var scanError: String?

    var body: some View {
        Form {
            Section("扫码") {
                Button { scanning = true } label: { Label("扫描电脑上的二维码", systemImage: "qrcode.viewfinder") }
                    .disabled(!PairingScanner.available || model.busy)
                if !PairingScanner.available {
                    Text("这台设备不能用相机扫码，可以从照片识别或粘贴配对码。")
                        .font(.caption).foregroundStyle(.secondary)
                }
                PhotosPicker(selection: $photo, matching: .images) {
                    Label("从照片识别", systemImage: "photo")
                }
                .disabled(model.busy)
                if let scanError { Text(scanError).font(.caption).foregroundStyle(.red) }
            }
            Section("配对码") {
                TextField("YTTT-PAIR:1:…", text: $code, axis: .vertical)
                    .font(.system(.footnote, design: .monospaced))
                    .lineLimit(3...6)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("配对") { Task { await model.pair(code: code) } }
                    .disabled(model.busy || code.isEmpty)
            }
            if let shortCode = model.shortCode {
                Section("核对数字") {
                    Text(String(format: "%06d", shortCode))
                        .font(.system(.largeTitle, design: .monospaced))
                }
            }
            if !model.status.isEmpty {
                Section { Text(model.status).foregroundStyle(.secondary) }
            }
        }
        .navigationTitle("配对 Host")
        .sheet(isPresented: $scanning) {
            PairingScanner { found in
                scanning = false
                Task { await model.pair(code: found) }
            }
            .ignoresSafeArea()
        }
        .onChange(of: photo) { _, item in
            guard let item else { return }
            Task {
                photo = nil
                guard let data = try? await item.loadTransferable(type: Data.self),
                      let image = UIImage(data: data)?.cgImage,
                      let found = pairingCode(in: image)
                else {
                    scanError = "这张照片里没有配对二维码"
                    return
                }
                scanError = nil
                await model.pair(code: found)
            }
        }
    }
}

/// The overview or a project, the two root pages the drawer switches.
struct RootPage: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        switch model.root {
        case .home:
            NavigationStack(path: $model.path) { TerminalListView() }
        case .project(let place):
            SideDrawer(edge: .trailing, isOpen: $model.drawerFilesOpen) {
                NavigationStack { ProjectView(place: place) }
            } drawer: {
                // A new project starts a new tree; the same project keeps its own
                // while a file is open.
                FilesDrawerView(projectId: place.projectId).id(place.projectId)
            }
            .fullScreenCover(item: $model.editing) { file in
                EditorView(file: file).environmentObject(model)
            }
        }
    }
}

/// The home screen: counts, then what needs the user, what runs and what
/// ended; tapping an Agent opens its terminal.
struct TerminalListView: View {
    @EnvironmentObject private var model: AppModel
    @State private var showFinished = false
    @State private var showTerminals = false

    var body: some View {
        let sections = HomeSections(model.overview)
        List {
            Section {
                if !model.status.isEmpty {
                    Text(model.status).foregroundStyle(.secondary)
                }
                HStack {
                    count(sections.projects, "项目")
                    count(sections.tabs, "Tab")
                    count(sections.terminals, "终端")
                    count(sections.agents, "Agent")
                }
            }
            if !sections.attention.isEmpty {
                Section("需要处理") { ForEach(sections.attention, id: \.sessionId, content: row) }
            }
            if !sections.running.isEmpty {
                Section("运行中") { ForEach(sections.running, id: \.sessionId, content: row) }
            }
            if !sections.finished.isEmpty {
                Section {
                    DisclosureGroup("已结束（\(sections.finished.count)）", isExpanded: $showFinished) {
                        ForEach(sections.finished, id: \.sessionId, content: row)
                    }
                }
            }
            Section {
                DisclosureGroup("所有终端（\(model.terminals.count)）", isExpanded: $showTerminals) {
                    ForEach(model.terminals, id: \.sessionId) { terminal in
                        NavigationLink(value: terminal) {
                            VStack(alignment: .leading) {
                                Text(terminal.sessionId).font(.system(.body, design: .monospaced))
                                Text("\(terminal.cols)×\(terminal.rows) · \(terminal.projectId)")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("总控")
        .navigationDestination(for: TerminalInfo.self) { terminal in
            if let session = model.session {
                TerminalScreen(session: session, terminal: terminal,
                               tip: tipText(model.overview.agents.first { $0.sessionId == terminal.sessionId }))
                    .navigationTitle(terminal.sessionId)
                    .navigationBarTitleDisplayMode(.inline)
            }
        }
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button { model.drawerOpen = true } label: { Image(systemName: "line.3.horizontal") }
            }
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button("刷新") { Task { await model.refresh() } }
                    Button("忘记这台 Host", role: .destructive) { Task { await model.forget() } }
                } label: { Image(systemName: "ellipsis.circle") }
            }
        }
        .task { await model.connect() }
        .refreshable { await model.refresh() }
    }

    private func count(_ value: Int, _ label: String) -> some View {
        VStack {
            Text("\(value)").font(.title2.monospacedDigit())
            Text(label).font(.caption).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
    }

    private func row(_ agent: AgentSummary) -> some View {
        Button {
            Task { await model.open(agent: agent) }
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                HStack {
                    Text("\(model.overview.projectName(agent.projectId)) · \(model.overview.tabTitle(project: agent.projectId, tab: agent.tabId))")
                        .font(.headline)
                    Spacer()
                    Text(agent.stateLabel)
                        .font(.caption.bold())
                        .foregroundStyle(agent.state == .waiting || agent.state == .failed ? .orange : .secondary)
                }
                if let text = agent.task ?? agent.waitingMessage {
                    Text(text).font(.subheadline).lineLimit(2)
                }
                Text("\(agent.provider) · \(agent.elapsed())\(agent.currentAction.map { " · " + $0 } ?? "")")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .foregroundStyle(.primary)
    }
}
