import SwiftUI

/// The overview: the Host and its link, counts, then what needs the user,
/// what runs and what ended. Tapping an Agent pushes where it runs.
struct OverviewView: View {
    @EnvironmentObject private var model: AppModel
    @State private var showFinished = false

    var body: some View {
        let sections = HomeSections(model.overview)
        List {
            Section {
                hostMenu
                HStack {
                    count("folder", sections.projects, "项目")
                    count("rectangle.stack", sections.tabs, "Tab")
                    count("terminal", sections.terminals, "终端")
                    count("cpu", sections.agents, "Agent")
                }
                .padding(.vertical, 2)
            }
            if model.overview.agents.isEmpty {
                Section {
                    ContentUnavailableView(
                        model.session == nil ? "还没连上 Host" : "没有运行中的 Agent",
                        systemImage: model.session == nil ? "bolt.horizontal.circle" : "cpu",
                        description: Text(model.session == nil ? "连上以后，这里会列出电脑上的 Agent。" : "在电脑上启动一个 Agent，它会出现在这里。"))
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
                    DisclosureGroup(isExpanded: $showFinished) {
                        ForEach(sections.finished, id: \.sessionId, content: row)
                    } label: {
                        Label { Text("已结束（\(sections.finished.count)）") } icon: { IconTile(systemName: "checkmark", color: .green) }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle("总控")
        .navigationDestination(for: Destination.self) { DestinationView(destination: $0) }
        .refreshable { await model.refresh() }
    }

    private var linkLabel: String {
        if !model.status.isEmpty { return model.status }
        guard model.session != nil else { return "未连接" }
        switch model.linkPath {
        case .direct: return "已连接 · 直连"
        case .relay: return "已连接 · 经中继"
        case .unknown: return "已连接"
        }
    }

    private var hostMenu: some View {
        HostMenuRow(hosts: model.hostNames.map { HostMenuRow.Host(id: $0.id, name: $0.name) },
                    current: model.hosts.current, label: model.hostLabel, linkLabel: linkLabel,
                    connected: model.session != nil, linkPath: model.linkPath,
                    onSwitch: { id in Task { await model.switchTo(id) } },
                    onAdd: { model.addingHost = true },
                    onManage: { model.tab = .settings })
            .equatable()
    }

    private func count(_ symbol: String, _ value: Int, _ label: String) -> some View {
        VStack(spacing: 2) {
            Image(systemName: symbol).font(.footnote).foregroundStyle(.tint)
            Text("\(value)").font(.title3.weight(.medium).monospacedDigit())
            Text(label).font(.caption2).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
    }

    private func row(_ agent: AgentSummary) -> some View {
        Button {
            Task { await model.open(agent: agent) }
        } label: {
            HStack(alignment: .top, spacing: 12) {
                AgentTile(provider: agent.provider)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(alignment: .firstTextBaseline) {
                        Text("\(model.overview.projectName(agent.projectId)) · \(model.overview.tabTitle(project: agent.projectId, tab: agent.tabId))")
                            .font(.headline)
                            .lineLimit(1)
                        Spacer(minLength: 4)
                        Label(agent.stateLabel, systemImage: agent.stateSymbol)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(agent.stateColor)
                            .labelStyle(.titleAndIcon)
                    }
                    if let text = agent.waitingMessage ?? agent.task {
                        Text(text).font(.subheadline).lineLimit(2)
                    }
                    Text("\(agent.elapsed())\(agent.currentAction.map { " · " + $0 } ?? "")")
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
        }
        .foregroundStyle(.primary)
        .contextMenu {
            Button { Task { await model.open(agent: agent) } } label: { Label("打开终端", systemImage: "terminal") }
        }
    }
}

/// The current Host; the menu switches, adds and manages Hosts. Equatable on
/// what it shows, so the overview's frequent updates leave an open menu be:
/// rebuilding it closes it.
struct HostMenuRow: View, Equatable {
    struct Host: Equatable {
        let id: String
        let name: String
    }

    let hosts: [Host]
    let current: String?
    let label: String
    let linkLabel: String
    let connected: Bool
    let linkPath: LinkPath
    let onSwitch: (String) -> Void
    let onAdd: () -> Void
    let onManage: () -> Void

    static func == (a: Self, b: Self) -> Bool {
        a.hosts == b.hosts && a.current == b.current && a.label == b.label && a.linkLabel == b.linkLabel
            && a.connected == b.connected && a.linkPath == b.linkPath
    }

    var body: some View {
        Menu {
            ForEach(hosts, id: \.id) { host in
                Button { onSwitch(host.id) } label: {
                    if host.id == current {
                        Label(host.name, systemImage: "checkmark")
                    } else {
                        Label(host.name, systemImage: "desktopcomputer")
                    }
                }
            }
            Divider()
            Button(action: onAdd) { Label("添加 Host", systemImage: "plus") }
            Button(action: onManage) { Label("管理 Host…", systemImage: "gear") }
        } label: {
            HStack(spacing: 12) {
                IconTile(systemName: "laptopcomputer", color: .blue)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 4) {
                        Text(label).lineLimit(1)
                        Image(systemName: "chevron.up.chevron.down").font(.caption2.weight(.semibold))
                    }
                    HStack(spacing: 4) {
                        Circle().fill(connected ? Color.green : .orange).frame(width: 7, height: 7)
                        Text(linkLabel).lineLimit(1)
                        if connected, linkPath != .unknown {
                            Image(systemName: linkPath == .relay ? "cloud.fill" : "bolt.fill")
                        }
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
        }
        // A row, not a link: the name in the text colour.
        .tint(.primary)
    }
}

/// The projects, each with its running Agents' logos, waits and runs.
struct ProjectsView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let overview = model.overview
        let badges = overview.badges()
        Group {
            if overview.projects.isEmpty {
                ContentUnavailableView("电脑上还没有打开的项目", systemImage: "folder")
            } else {
                List(overview.projects, id: \.projectId) { project in
                    let mark = overview.projectMark(project.projectId)
                    NavigationLink(value: Destination.project(overview.initialPlace(project.projectId))) {
                        HStack(spacing: 12) {
                            IconTile(systemName: "folder.fill", color: .blue)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(project.name).lineLimit(1)
                                Text("\(project.tabs.count) 个 tab").font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer(minLength: 4)
                            HStack(spacing: 3) {
                                ForEach(mark.providers.prefix(3), id: \.self) { AgentLogo(provider: $0, size: 16) }
                            }
                            if mark.waiting > 0 { pill("hand.raised.fill", mark.waiting, .orange) }
                            if let running = badges[project.projectId]?.running, running > 0 {
                                pill("bolt.fill", running, Color(uiColor: .systemGray3))
                            }
                        }
                    }
                }
                .listStyle(.insetGrouped)
            }
        }
        .navigationTitle("项目")
        .navigationDestination(for: Destination.self) { DestinationView(destination: $0) }
    }

    private func pill(_ symbol: String, _ value: Int, _ color: Color) -> some View {
        Label("\(value)", systemImage: symbol)
            .font(.caption2.weight(.semibold))
            .labelStyle(.titleAndIcon)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .foregroundStyle(.white)
            .background(color, in: Capsule())
    }
}

/// Settings: the paired Hosts, and about.
struct SettingsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var forgetting: (id: String, name: String)?
    @State private var renaming: String?
    @State private var newName = ""

    var body: some View {
        Form {
            Section {
                ForEach(model.hostNames, id: \.id) { host in
                    Button { Task { await model.switchTo(host.id) } } label: {
                        HStack(spacing: 12) {
                            IconTile(systemName: host.id == model.hosts.current ? "laptopcomputer" : "server.rack",
                                     color: host.id == model.hosts.current ? .blue : .gray)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(host.name).foregroundStyle(.primary)
                                if host.id == model.hosts.current {
                                    Text(model.session == nil ? "当前" : "已连接 · 当前")
                                        .font(.caption).foregroundStyle(.secondary)
                                }
                            }
                            Spacer()
                            if host.id == model.hosts.current { Image(systemName: "checkmark").foregroundStyle(.tint) }
                        }
                    }
                    .tint(.primary)
                    .swipeActions(edge: .trailing) {
                        Button(role: .destructive) { forgetting = host } label: { Label("忘记", systemImage: "trash") }
                        Button { startRenaming(host.id) } label: { Label("重命名", systemImage: "pencil") }
                            .tint(.blue)
                    }
                    .contextMenu {
                        Button { startRenaming(host.id) } label: { Label("重命名", systemImage: "pencil") }
                        Button(role: .destructive) { forgetting = host } label: { Label("忘记", systemImage: "trash") }
                    }
                }
                Button { model.addingHost = true } label: {
                    HStack(spacing: 12) {
                        IconTile(systemName: "plus", color: .green)
                        Text("添加 Host")
                    }
                }
            } header: {
                Text("Host")
            } footer: {
                Text("点一台 Host 切换过去；向左滑或长按可以重命名、忘记它。")
            }
            AppearanceSection()
            AccountSection()
            Section("关于") {
                HStack(spacing: 12) {
                    IconTile(systemName: "info", color: Color(uiColor: .systemGray))
                    LabeledContent("版本", value: Self.version)
                }
                NavigationLink {
                    LicensesView()
                } label: {
                    HStack(spacing: 12) {
                        IconTile(systemName: "doc.text", color: .indigo)
                        Text("开源许可")
                    }
                }
            }
        }
        .navigationTitle("设置")
        .alert("重命名 Host", isPresented: Binding(
            get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("名称", text: $newName)
            Button("取消", role: .cancel) { renaming = nil }
            Button("好") {
                if let id = renaming { model.rename(id, to: newName) }
                renaming = nil
            }
        } message: {
            Text("只改这台手机上显示的名称。留空则用电脑自己的名称。")
        }
        .confirmationDialog("忘记 \(forgetting?.name ?? "")？", isPresented: Binding(
            get: { forgetting != nil }, set: { if !$0 { forgetting = nil } }), titleVisibility: .visible) {
            Button("忘记", role: .destructive) {
                if let id = forgetting?.id { Task { await model.forget(id) } }
                forgetting = nil
            }
        } message: {
            Text("这台手机不再连接它。要再用，需要在电脑上重新配对。")
        }
    }

    private func startRenaming(_ id: String) {
        newName = model.editableName(id)
        renaming = id
    }

    static var version: String {
        let info = Bundle.main.infoDictionary
        let short = info?["CFBundleShortVersionString"] as? String ?? ""
        let build = info?["CFBundleVersion"] as? String ?? ""
        return "\(short) (\(build))"
    }
}

/// The open-source libraries the app ships with, and their licenses.
struct LicensesView: View {
    private let licenses = [
        ("SwiftTerm", "MIT"),
        ("Runestone", "MIT"),
        ("TreeSitterLanguages（tree-sitter 语法）", "MIT"),
        ("JetBrains Mono", "SIL OFL 1.1"),
        ("Lobe Icons（Agent 图标）", "MIT"),
        ("Oh My Pi（图标）", "MIT"),
    ]

    var body: some View {
        List(licenses, id: \.0) { name, license in
            LabeledContent(name, value: license)
        }
        .navigationTitle("开源许可")
    }
}
