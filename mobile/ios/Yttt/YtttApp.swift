import PhotosUI
import SwiftUI

@main
struct YtttApp: App {
    @StateObject private var model = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            Group {
                if model.record == nil {
                    NavigationStack { PairView() }
                } else {
                    MainTabs()
                }
            }
            .environmentObject(model)
            .task {
                // With a Host paired already, adds one more, as "添加 Host" does.
                if let code = AppModel.autoPairCode {
                    await model.pair(code: code)
                }
                if let suffix = AppModel.hooks.switchTo,
                   let host = model.hosts.hosts.first(where: { $0.id.hasSuffix(suffix) }) {
                    await model.switchTo(host.id)
                }
                if AppModel.hooks.forget, let current = model.hosts.current { await model.forget(current) }
                switch AppModel.hooks.tabBar {
                case "projects": model.tab = .projects
                case "settings": model.tab = .settings
                default: break
                }
            }
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .background { model.enterBackground() }
        }
    }
}

/// The tab bar: the overview, the projects and settings, each with its own
/// navigation stack; how many Agents need the user shows on the overview's tab.
struct MainTabs: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let attention = HomeSections(model.overview).attention.count
        TabView(selection: $model.tab) {
            NavigationStack(path: $model.overviewPath) { OverviewView() }
                .tabItem { Label("总控", systemImage: "square.grid.2x2") }
                .badge(attention)
                .tag(AppTab.overview)
            NavigationStack(path: $model.projectsPath) { ProjectsView() }
                .tabItem { Label("项目", systemImage: "folder") }
                .tag(AppTab.projects)
            NavigationStack { SettingsView() }
                .tabItem { Label("设置", systemImage: "gear") }
                .tag(AppTab.settings)
        }
        .sheet(isPresented: $model.addingHost) {
            NavigationStack { PairView() }.environmentObject(model)
        }
        .fullScreenCover(item: $model.editing) { file in
            EditorView(file: file).environmentObject(model)
        }
        .task { await model.connect() }
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
            AccountStep()
            Section {
                Button { scanning = true } label: {
                    HStack(spacing: 12) {
                        IconTile(systemName: "qrcode.viewfinder", color: .blue)
                        Text("扫描二维码")
                    }
                }
                .disabled(!PairingScanner.available || model.busy)
                PhotosPicker(selection: $photo, matching: .images) {
                    HStack(spacing: 12) {
                        IconTile(systemName: "photo", color: .green)
                        Text("从照片识别")
                    }
                }
                .disabled(model.busy)
                if let scanError { Text(scanError).font(.caption).foregroundStyle(.red) }
            } header: {
                Text("第 2 步 · 配对电脑")
            } footer: {
                VStack(alignment: .leading, spacing: 4) {
                    Text("在电脑上打开 yttt 的“设置 ▸ 远程访问 ▸ 配对设备”，会显示一个二维码。")
                    if !PairingScanner.available {
                        Text("这台设备不能用相机扫码，可以从照片识别或粘贴配对码。")
                    }
                }
            }
            Section("配对码") {
                HStack(alignment: .top) {
                    TextField("YTTT-PAIR:1:…", text: $code, axis: .vertical)
                        .font(.system(.footnote, design: .monospaced))
                        .lineLimit(3...6)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    PasteButton(payloadType: String.self) { strings in
                        code = strings.first?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                    }
                    .labelStyle(.iconOnly)
                    .buttonBorderShape(.circle)
                }
                Button("配对") { Task { await model.pair(code: code) } }
                    .disabled(model.busy || code.isEmpty)
            }
            if let shortCode = model.shortCode {
                Section {
                    Text(String(format: "%06d", shortCode))
                        .font(.system(.largeTitle, design: .monospaced))
                        .frame(maxWidth: .infinity)
                } header: {
                    Text("核对数字")
                } footer: {
                    Text("和电脑上显示的一样，就在电脑上点“确认”。")
                }
            }
            if !model.status.isEmpty {
                Section {
                    HStack {
                        if model.busy { ProgressView() }
                        Text(model.status).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .navigationTitle(model.record == nil ? "配对 Host" : "添加 Host")
        .toolbar {
            if model.record != nil {
                ToolbarItem(placement: .cancellationAction) {
                    Button("取消") { model.addingHost = false }.disabled(model.busy)
                }
            }
        }
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
